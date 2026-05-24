#![allow(clippy::too_many_arguments, clippy::result_large_err)]

use std::collections::HashMap;

use cranelift_codegen::{
    ir::{
        self, AbiParam, Block, InstBuilder, MemFlags, Signature, TrapCode, Type, Value,
        condcodes::{FloatCC, IntCC},
        types,
    },
    isa::CallConv,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, FuncId, Linkage, Module, ModuleError};

use crate::{
    hir::PrimitiveType,
    mir::{
        self, Abi, AssignValue, BinOp, BlockId, Callee, GC_HEADER_SIZE, LocalId, MirConst, MirFnId,
        MirProgram, MirType, MirTypeDef, MirTypeId, NULL_TAG, Operand, POINTER_SIZE, Stmt,
        Terminator, TrapReason, UNION_PAYLOAD_SIZE, UNION_TOTAL_SIZE, UnOp,
    },
};

pub struct Compiled<M: Module> {
    pub module: M,
    pub function_ids: HashMap<MirFnId, FuncId>,
}

pub fn compile<M: Module>(mir: &MirProgram, mut module: M) -> Result<Compiled<M>, ModuleError> {
    let mut function_ids = HashMap::new();
    let mut declared: HashMap<String, FuncId> = HashMap::new();

    // Declare every MIR function. Extern functions share a single C symbol
    // across monomorphizations (e.g. `pin<i32>` and `pin<str>` both link to
    // `pin`); use the first FuncId we see for that name on every later one.
    for (id, f) in &mir.functions {
        let sig: Signature = build_signature(&module, f);
        let linkage = if f.id == mir.entry {
            Linkage::Export
        } else if f.abi == Abi::Extern && f.blocks.is_empty() {
            Linkage::Import
        } else if f.abi == Abi::Extern {
            Linkage::Export
        } else {
            Linkage::Local
        };
        let fid = if f.abi == Abi::Extern {
            if let Some(&existing) = declared.get(&f.name) {
                existing
            } else {
                let fid = module.declare_function(&f.name, linkage, &sig)?;
                declared.insert(f.name.clone(), fid);
                fid
            }
        } else {
            let fid = module.declare_function(&f.name, linkage, &sig)?;
            declared.insert(f.name.clone(), fid);
            fid
        };
        function_ids.insert(*id, fid);
    }

    let mut ctx = module.make_context();
    let mut builder_ctx = FunctionBuilderContext::new();
    for (id, f) in &mir.functions {
        if f.abi == Abi::Extern && f.blocks.is_empty() {
            continue;
        }
        ctx.func.signature = build_signature(&module, f);
        lower_function(mir, &mut module, &mut ctx.func, &mut builder_ctx, &function_ids, f)?;
        module.define_function(function_ids[id], &mut ctx)?;
        module.clear_context(&mut ctx);
    }

    Ok(Compiled {
        module,
        function_ids,
    })
}

/// Emit a C-ABI `main` trampoline that calls the program entry and returns
/// its value as an i32 exit code (or 0 for Unit). Required for AOT-linked
/// executables: the system linker resolves `_main` against this symbol.
pub fn emit_c_main<M: Module>(
    compiled: &mut Compiled<M>,
    mir: &MirProgram,
) -> Result<(), ModuleError> {
    let entry_fid = compiled.function_ids[&mir.entry];
    let entry_returns_unit = matches!(mir.functions[&mir.entry].return_type, MirType::Unit);

    let call_conv = CallConv::triple_default(compiled.module.isa().triple());
    let mut sig = Signature::new(call_conv);
    sig.returns.push(AbiParam::new(types::I32));
    let main_fid = compiled
        .module
        .declare_function("main", Linkage::Export, &sig)?;

    let mut ctx = compiled.module.make_context();
    ctx.func.signature = sig;
    let mut fbctx = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbctx);
    let block = b.create_block();
    b.switch_to_block(block);
    b.seal_block(block);

    let entry_ref = compiled.module.declare_func_in_func(entry_fid, b.func);
    let call = b.ins().call(entry_ref, &[]);
    let exit_code = if entry_returns_unit {
        b.ins().iconst(types::I32, 0)
    } else {
        let v = b.inst_results(call)[0];
        let ty = b.func.dfg.value_type(v);
        if ty == types::I32 {
            v
        } else if ty == types::I64 {
            b.ins().ireduce(types::I32, v)
        } else if ty.bits() < 32 {
            b.ins().uextend(types::I32, v)
        } else {
            b.ins().iconst(types::I32, 0)
        }
    };
    b.ins().return_(&[exit_code]);
    b.finalize();

    compiled.module.define_function(main_fid, &mut ctx)?;
    compiled.module.clear_context(&mut ctx);
    Ok(())
}

fn build_signature<M: Module>(module: &M, f: &mir::MirFunction) -> Signature {
    let call_conv = match f.abi {
        Abi::Otter => module.target_config().default_call_conv,
        Abi::Extern => CallConv::triple_default(module.isa().triple()),
    };
    let mut sig = Signature::new(call_conv);
    for local_id in &f.params {
        let ty = clif_param_type(&f.locals[local_id].ty);
        sig.params.push(AbiParam::new(ty));
    }
    if !matches!(f.return_type, MirType::Unit) {
        sig.returns
            .push(AbiParam::new(clif_param_type(&f.return_type)));
    }
    sig
}

/// CLIF representation for a function parameter / return slot. Unions are
/// passed as 16-byte values; for simplicity we represent them as pointers
/// to caller-supplied stack slots elsewhere — but here in the param slot
/// we use I64, matching MIR's "every reference and primitive ≤ 8 bytes" rule.
fn clif_param_type(t: &MirType) -> Type {
    match t {
        MirType::Unit => unreachable!("Unit must be filtered before reaching CLIF param"),
        _ => clif_type(t),
    }
}

pub fn clif_type(t: &MirType) -> Type {
    match t {
        MirType::Primitive(p) => prim_clif(p),
        MirType::ManagedRef(_)
        | MirType::NullableRef(_)
        | MirType::Pointer(_)
        | MirType::Closure(_)
        | MirType::FnPtr(_, _) => types::I64,
        // Unions carry { tag: u16, _pad: 6, payload: 8 } = 16 bytes. They
        // are stored in stack slots; the SSA variable holds the slot
        // address as a pointer.
        MirType::Union(_) => types::I64,
        MirType::Unit => types::I8, // sentinel only; never read or written
    }
}

fn prim_clif(p: &PrimitiveType) -> Type {
    match p {
        PrimitiveType::Int8 | PrimitiveType::Uint8 | PrimitiveType::Bool => types::I8,
        PrimitiveType::Int16 | PrimitiveType::Uint16 => types::I16,
        PrimitiveType::Int32 | PrimitiveType::Uint32 | PrimitiveType::Char => types::I32,
        PrimitiveType::Int64 | PrimitiveType::Uint64 => types::I64,
        PrimitiveType::Float32 => types::F32,
        PrimitiveType::Float64 => types::F64,
        PrimitiveType::String => types::I64,
    }
}

fn is_signed_int(p: &PrimitiveType) -> bool {
    matches!(
        p,
        PrimitiveType::Int8 | PrimitiveType::Int16 | PrimitiveType::Int32 | PrimitiveType::Int64
    )
}

fn is_float(p: &PrimitiveType) -> bool {
    matches!(p, PrimitiveType::Float32 | PrimitiveType::Float64)
}

struct FnCtx<'a> {
    mir: &'a MirProgram,
    vars: HashMap<LocalId, Variable>,
    blocks: HashMap<BlockId, Block>,
    /// Cached FuncRef in the current function for runtime helpers.
    alloc_ref: Option<ir::FuncRef>,
    vtable_lookup_ref: Option<ir::FuncRef>,
    helpers: HashMap<&'static str, ir::FuncRef>,
}

fn lower_function<M: Module>(
    mir: &MirProgram,
    module: &mut M,
    func: &mut ir::Function,
    builder_ctx: &mut FunctionBuilderContext,
    function_ids: &HashMap<MirFnId, FuncId>,
    mir_func: &mir::MirFunction,
) -> Result<(), ModuleError> {
    let mut b = FunctionBuilder::new(func, builder_ctx);

    let mut blocks: HashMap<BlockId, Block> = HashMap::new();
    let entry = b.create_block();
    blocks.insert(mir_func.entry, entry);
    for &bid in mir_func.blocks.keys() {
        if bid != mir_func.entry {
            blocks.insert(bid, b.create_block());
        }
    }

    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);

    // Declare a Cranelift variable for every non-Unit local. Union locals
    // hold the address of a 16-byte heap block (allocated via __of_alloc)
    // so the value survives function returns; non-parameter union locals
    // get that heap pointer assigned in the entry block before any user
    // instructions run.
    let mut vars: HashMap<LocalId, Variable> = HashMap::new();
    let mut union_locals: Vec<LocalId> = vec![];
    for (lid, loc) in &mir_func.locals {
        if matches!(loc.ty, MirType::Unit) {
            continue;
        }
        let v = Variable::from_u32(lid.0);
        b.declare_var(v, clif_type(&loc.ty));
        vars.insert(*lid, v);
        if matches!(loc.ty, MirType::Union(_)) {
            union_locals.push(*lid);
        }
    }

    let entry_args = b.block_params(entry).to_vec();
    let param_set: std::collections::HashSet<LocalId> = mir_func.params.iter().copied().collect();
    for (param_lid, val) in mir_func.params.iter().zip(entry_args) {
        if let Some(v) = vars.get(param_lid) {
            b.def_var(*v, val);
        }
    }

    let mut order: Vec<BlockId> = std::iter::once(mir_func.entry)
        .chain(
            mir_func
                .blocks
                .keys()
                .copied()
                .filter(|bid| *bid != mir_func.entry),
        )
        .collect();

    let mut ctx = FnCtx {
        mir,
        vars,
        blocks,
        alloc_ref: None,
        vtable_lookup_ref: None,
        helpers: HashMap::new(),
    };

    // Allocate a 16-byte heap block (via __of_alloc) for each non-parameter
    // Union local and bind the variable to the resulting pointer. Done
    // here so the variable is live across the whole CFG, including any
    // return that propagates the union back to the caller.
    let union_param_addrs = union_locals
        .iter()
        .copied()
        .filter(|lid| !param_set.contains(lid))
        .collect::<Vec<_>>();
    for lid in union_param_addrs {
        let alloc = ensure_alloc(module, &mut b, &mut ctx);
        let size_v = b.ins().iconst(types::I64, UNION_TOTAL_SIZE as i64);
        let tid_v = b.ins().iconst(types::I64, 0);
        let call = b.ins().call(alloc, &[size_v, tid_v]);
        let addr = b.inst_results(call)[0];
        b.def_var(ctx.vars[&lid], addr);
    }

    let entry_cb = ctx.blocks[&mir_func.entry];
    for (idx, bid) in order.drain(..).enumerate() {
        let cb = ctx.blocks[&bid];
        let blk = &mir_func.blocks[&bid];
        // The very first iteration is the entry block — we're already
        // inside it (it accumulated the pre-bind code above). Cranelift
        // forbids switching into a block that has instructions but no
        // terminator, so just skip the switch.
        if !(idx == 0 && cb == entry_cb) {
            b.switch_to_block(cb);
        }
        for stmt in &blk.stmts {
            lower_stmt(module, &mut b, function_ids, &mut ctx, mir_func, stmt);
        }
        lower_terminator(
            module,
            &mut b,
            function_ids,
            &mut ctx,
            mir_func,
            &blk.terminator,
        );
    }

    b.seal_all_blocks();
    b.finalize();

    Ok(())
}

fn lower_stmt<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    ctx: &mut FnCtx,
    mir_func: &mir::MirFunction,
    stmt: &mir::Stmt,
) {
    match stmt {
        Stmt::Assign(lid, av) => {
            let dest_ty = &mir_func.locals[lid].ty;
            if matches!(dest_ty, MirType::Unit) {
                let _ = lower_rvalue(module, b, func_ids, ctx, mir_func, av, dest_ty, *lid);
                return;
            }

            // Union destination: the variable is pre-bound to a 16-byte
            // heap block; in-place rvalues (UnionConstruct) already wrote
            // through it, but value-producing rvalues (Use, Call, Field,
            // UnionPayload) yield a fresh pointer to copy from.
            if matches!(dest_ty, MirType::Union(_)) {
                let src = lower_rvalue(module, b, func_ids, ctx, mir_func, av, dest_ty, *lid);
                if matches!(av, AssignValue::UnionConstruct(_, _, _)) {
                    return;
                }
                let dest = b.use_var(ctx.vars[lid]);
                let lo = b.ins().load(types::I64, MemFlags::trusted(), src, 0);
                b.ins().store(MemFlags::trusted(), lo, dest, 0);
                let hi = b.ins().load(types::I64, MemFlags::trusted(), src, 8);
                b.ins().store(MemFlags::trusted(), hi, dest, 8);
                return;
            }

            let v = lower_rvalue(module, b, func_ids, ctx, mir_func, av, dest_ty, *lid);
            if let Some(var) = ctx.vars.get(lid).copied() {
                b.def_var(var, v);
            }
        }
    }
}

fn lower_rvalue<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    ctx: &mut FnCtx,
    mir_func: &mir::MirFunction,
    av: &AssignValue,
    dest_ty: &MirType,
    dest_lid: LocalId,
) -> Value {
    match av {
        AssignValue::Use(op) => use_operand(module, b, func_ids, ctx, op),
        AssignValue::Bin(op, l, r) => lower_bin(module, b, func_ids, ctx, mir_func, *op, l, r),
        AssignValue::Un(op, x) => lower_un(module, b, func_ids, ctx, mir_func, *op, x),
        AssignValue::Cast(src, target) => {
            lower_cast(module, b, func_ids, ctx, mir_func, src, target)
        }
        AssignValue::Call(callee, args) => {
            lower_call(module, b, func_ids, ctx, mir_func, callee, args, dest_ty)
        }
        AssignValue::AllocStruct(tid, fields) => {
            lower_alloc_struct(module, b, func_ids, ctx, mir_func, *tid, fields)
        }
        AssignValue::AllocList(_, _)
        | AssignValue::AllocMap(_, _, _)
        | AssignValue::AllocClosure(_, _) => {
            // Out-of-line container/closure allocation is delegated to the
            // runtime via the same __of_alloc seam used by structs (16 bytes
            // here is just a placeholder header for the runtime to fill in).
            lower_alloc_generic(module, b, func_ids, ctx)
        }
        AssignValue::Field(recv, idx) => {
            lower_field(module, b, func_ids, ctx, mir_func, recv, *idx, dest_ty)
        }
        AssignValue::UnionConstruct(_uid, tag, payload) => {
            lower_union_construct(module, b, func_ids, ctx, mir_func, *tag, payload, dest_lid)
        }
        AssignValue::UnionTag(scrut) => {
            lower_union_tag(module, b, func_ids, ctx, mir_func, scrut)
        }
        AssignValue::UnionPayload(scrut, _uid) => {
            lower_union_payload(module, b, func_ids, ctx, mir_func, scrut, dest_ty)
        }
    }
}

fn lower_bin<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    ctx: &mut FnCtx,
    mir_func: &mir::MirFunction,
    op: BinOp,
    l: &Operand,
    r: &Operand,
) -> Value {
    let prim = operand_prim(mir_func, l);
    let lv = use_operand(module, b, func_ids, ctx, l);
    let rv = use_operand(module, b, func_ids, ctx, r);

    // String ops route through the runtime; +, ==, != on raw pointers
    // would compare addresses, not contents.
    if matches!(prim, PrimitiveType::String) {
        match op {
            BinOp::Add => {
                let helper = ensure_helper(
                    module,
                    b,
                    ctx,
                    "__of_str_concat",
                    &[types::I64, types::I64],
                    types::I64,
                );
                let call = b.ins().call(helper, &[lv, rv]);
                return b.inst_results(call)[0];
            }
            BinOp::Eq | BinOp::Neq => {
                let helper = ensure_helper(
                    module,
                    b,
                    ctx,
                    "__of_str_eq",
                    &[types::I64, types::I64],
                    types::I8,
                );
                let call = b.ins().call(helper, &[lv, rv]);
                let eq = b.inst_results(call)[0];
                return if matches!(op, BinOp::Eq) {
                    eq
                } else {
                    let one = b.ins().iconst(types::I8, 1);
                    b.ins().bxor(eq, one)
                };
            }
            _ => {}
        }
    }

    let ins = b.ins();
    if is_float(&prim) {
        match op {
            BinOp::Add => ins.fadd(lv, rv),
            BinOp::Sub => ins.fsub(lv, rv),
            BinOp::Mul => ins.fmul(lv, rv),
            BinOp::Div => ins.fdiv(lv, rv),
            BinOp::Mod => unreachable!("float % not supported in MIR"),
            BinOp::And | BinOp::Or => unreachable!("logical op on float"),
            BinOp::Eq => ins.fcmp(FloatCC::Equal, lv, rv),
            BinOp::Neq => ins.fcmp(FloatCC::NotEqual, lv, rv),
            BinOp::Lt => ins.fcmp(FloatCC::LessThan, lv, rv),
            BinOp::Le => ins.fcmp(FloatCC::LessThanOrEqual, lv, rv),
            BinOp::Gt => ins.fcmp(FloatCC::GreaterThan, lv, rv),
            BinOp::Ge => ins.fcmp(FloatCC::GreaterThanOrEqual, lv, rv),
        }
    } else {
        let signed = is_signed_int(&prim);
        match op {
            BinOp::Add => ins.iadd(lv, rv),
            BinOp::Sub => ins.isub(lv, rv),
            BinOp::Mul => ins.imul(lv, rv),
            BinOp::Div => {
                if signed {
                    ins.sdiv(lv, rv)
                } else {
                    ins.udiv(lv, rv)
                }
            }
            BinOp::Mod => {
                if signed {
                    ins.srem(lv, rv)
                } else {
                    ins.urem(lv, rv)
                }
            }
            BinOp::And => ins.band(lv, rv),
            BinOp::Or => ins.bor(lv, rv),
            BinOp::Eq => ins.icmp(IntCC::Equal, lv, rv),
            BinOp::Neq => ins.icmp(IntCC::NotEqual, lv, rv),
            BinOp::Lt => ins.icmp(int_cc(signed, Cmp::Lt), lv, rv),
            BinOp::Le => ins.icmp(int_cc(signed, Cmp::Le), lv, rv),
            BinOp::Gt => ins.icmp(int_cc(signed, Cmp::Gt), lv, rv),
            BinOp::Ge => ins.icmp(int_cc(signed, Cmp::Ge), lv, rv),
        }
    }
}

fn lower_un<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    ctx: &mut FnCtx,
    mir_func: &mir::MirFunction,
    op: UnOp,
    x: &Operand,
) -> Value {
    let prim = operand_prim(mir_func, x);
    let xv = use_operand(module, b, func_ids, ctx, x);
    match op {
        UnOp::Neg if is_float(&prim) => b.ins().fneg(xv),
        UnOp::Neg => b.ins().ineg(xv),
        UnOp::Not => b.ins().bxor_imm(xv, 1),
    }
}

fn lower_cast<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    ctx: &mut FnCtx,
    mir_func: &mir::MirFunction,
    op: &Operand,
    target: &MirType,
) -> Value {
    let src = operand_prim(mir_func, op);
    let dst = match target {
        MirType::Primitive(p) => p.clone(),
        _ => unreachable!("Cast target must be primitive"),
    };
    let v = use_operand(module, b, func_ids, ctx, op);

    // String/Char share the I64/I32 CLIF reps with integers; the generic
    // numeric path below treats both ends as bag-of-bits, which is wrong for
    // anything involving a real str pointer. Route those through the
    // runtime, and reject the inverse direction (no parser in rt yet).
    if matches!(dst, PrimitiveType::String) {
        return cast_to_str(module, b, ctx, &src, v);
    }
    if matches!(src, PrimitiveType::String) {
        panic!("cast from str to {dst:?} not supported");
    }
    if matches!(dst, PrimitiveType::Char) && matches!(src, PrimitiveType::Char) {
        return v;
    }
    if matches!(src, PrimitiveType::Char) && !is_int_or_char(&dst) {
        panic!("cast from char to {dst:?} not supported");
    }
    if matches!(dst, PrimitiveType::Char) && !is_int_or_char(&src) {
        panic!("cast to char from {src:?} not supported");
    }

    let src_ty = prim_clif(&src);
    let dst_ty = prim_clif(&dst);

    let src_float = is_float(&src);
    let dst_float = is_float(&dst);
    let src_signed = is_signed_int(&src);
    let dst_signed = is_signed_int(&dst);

    match (src_float, dst_float) {
        (false, false) => {
            if src_ty == dst_ty {
                v
            } else if dst_ty.bits() < src_ty.bits() {
                b.ins().ireduce(dst_ty, v)
            } else if src_signed {
                b.ins().sextend(dst_ty, v)
            } else {
                b.ins().uextend(dst_ty, v)
            }
        }
        (false, true) => {
            if src_signed {
                b.ins().fcvt_from_sint(dst_ty, v)
            } else {
                b.ins().fcvt_from_uint(dst_ty, v)
            }
        }
        (true, false) => {
            if dst_signed {
                b.ins().fcvt_to_sint_sat(dst_ty, v)
            } else {
                b.ins().fcvt_to_uint_sat(dst_ty, v)
            }
        }
        (true, true) => {
            if src_ty == dst_ty {
                v
            } else if dst_ty.bits() > src_ty.bits() {
                b.ins().fpromote(dst_ty, v)
            } else {
                b.ins().fdemote(dst_ty, v)
            }
        }
    }
}

fn is_int_or_char(p: &PrimitiveType) -> bool {
    matches!(
        p,
        PrimitiveType::Int8
            | PrimitiveType::Int16
            | PrimitiveType::Int32
            | PrimitiveType::Int64
            | PrimitiveType::Uint8
            | PrimitiveType::Uint16
            | PrimitiveType::Uint32
            | PrimitiveType::Uint64
            | PrimitiveType::Char
    )
}

fn cast_to_str<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    ctx: &mut FnCtx,
    src: &PrimitiveType,
    v: Value,
) -> Value {
    use PrimitiveType::*;
    let (name, arg) = match src {
        String => return v,
        Int8 | Int16 | Int32 => {
            let widened = b.ins().sextend(types::I64, v);
            ("__of_i64_to_str", widened)
        }
        Int64 => ("__of_i64_to_str", v),
        Uint8 | Uint16 | Uint32 => {
            let widened = b.ins().uextend(types::I64, v);
            ("__of_u64_to_str", widened)
        }
        Uint64 => ("__of_u64_to_str", v),
        Float32 => {
            let promoted = b.ins().fpromote(types::F64, v);
            ("__of_f64_to_str", promoted)
        }
        Float64 => ("__of_f64_to_str", v),
        Bool => ("__of_bool_to_str", v),
        Char => ("__of_char_to_str", v),
    };
    let arg_ty = match src {
        Float32 | Float64 => types::F64,
        Bool => types::I8,
        Char => types::I32,
        _ => types::I64,
    };
    let helper = ensure_helper(module, b, ctx, name, &[arg_ty], types::I64);
    let call = b.ins().call(helper, &[arg]);
    b.inst_results(call)[0]
}

fn lower_call<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    ctx: &mut FnCtx,
    mir_func: &mir::MirFunction,
    callee: &Callee,
    args: &[Operand],
    dest_ty: &MirType,
) -> Value {
    let arg_vals: Vec<Value> = args
        .iter()
        .map(|a| use_operand(module, b, func_ids, ctx, a))
        .collect();
    let inst = match callee {
        Callee::Static(target) => {
            let func_ref = module.declare_func_in_func(func_ids[target], b.func);
            b.ins().call(func_ref, &arg_vals)
        }
        Callee::Indirect(callee_op) => {
            let callee_val = use_operand(module, b, func_ids, ctx, callee_op);
            // Build a signature matching the call shape.
            let mut sig = Signature::new(CallConv::triple_default(module.isa().triple()));
            for a in args {
                let ty = operand_ty(mir_func, a);
                sig.params.push(AbiParam::new(clif_type(&ty)));
            }
            if !matches!(dest_ty, MirType::Unit) {
                sig.returns.push(AbiParam::new(clif_type(dest_ty)));
            }
            let sig_ref = b.import_signature(sig);
            b.ins().call_indirect(sig_ref, callee_val, &arg_vals)
        }
        Callee::Virtual(recv_op, iface_mir, slot) => {
            // Resolve through the runtime helper:
            //   fn_ptr = __of_vtable_lookup(recv_obj, iface_id, slot_idx)
            let recv_val = use_operand(module, b, func_ids, ctx, recv_op);
            let iface_const = b.ins().iconst(types::I64, iface_mir.0 as i64);
            let slot_const = b.ins().iconst(types::I64, *slot as i64);
            let lookup = ensure_vtable_lookup(module, b, ctx);
            let lookup_call =
                b.ins().call(lookup, &[recv_val, iface_const, slot_const]);
            let fn_ptr = b.inst_results(lookup_call)[0];

            // Indirect-call through fn_ptr with the slot's signature.
            // Find the slot definition in the interface MirTypeDef.
            let (params, ret_ty) = match &ctx.mir.types[iface_mir] {
                MirTypeDef::Interface { method_slots, .. } => {
                    let s = &method_slots[*slot as usize];
                    (s.params.clone(), s.return_type.clone())
                }
                _ => unreachable!("virtual call on non-interface"),
            };
            let mut sig = Signature::new(CallConv::triple_default(module.isa().triple()));
            // Receiver passes as the first arg too.
            sig.params.push(AbiParam::new(types::I64));
            for p in params.iter().skip(1) {
                sig.params.push(AbiParam::new(clif_type(p)));
            }
            if !matches!(ret_ty, MirType::Unit) {
                sig.returns.push(AbiParam::new(clif_type(&ret_ty)));
            }
            let sig_ref = b.import_signature(sig);
            b.ins().call_indirect(sig_ref, fn_ptr, &arg_vals)
        }
    };
    if matches!(dest_ty, MirType::Unit) {
        // Some indirect/static call paths still produce a result we want to
        // discard. Cranelift requires every Value produced to be used, so
        // return a dummy I8 zero — the caller's `dest_ty == Unit` branch
        // throws this away.
        return b.ins().iconst(types::I8, 0);
    }
    b.inst_results(inst)
        .first()
        .copied()
        .unwrap_or_else(|| b.ins().iconst(types::I8, 0))
}

fn lower_alloc_struct<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    ctx: &mut FnCtx,
    mir_func: &mir::MirFunction,
    tid: MirTypeId,
    field_ops: &[Operand],
) -> Value {
    let (size, fields) = match &ctx.mir.types[&tid] {
        MirTypeDef::Struct { layout, fields, .. } => (layout.size, fields.clone()),
        _ => unreachable!("AllocStruct on non-struct type"),
    };
    let alloc = ensure_alloc(module, b, ctx);
    let size_v = b.ins().iconst(types::I64, (size + GC_HEADER_SIZE) as i64);
    let tid_v = b.ins().iconst(types::I64, tid.0 as i64);
    let call = b.ins().call(alloc, &[size_v, tid_v]);
    let obj = b.inst_results(call)[0];

    for (i, op) in field_ops.iter().enumerate() {
        let f = &fields[i];
        let val = use_operand(module, b, func_ids, ctx, op);
        store_field(b, obj, f.offset, &f.ty, val, mir_func, op, ctx);
    }
    obj
}

fn store_field(
    b: &mut FunctionBuilder,
    obj: Value,
    offset: u32,
    ty: &MirType,
    val: Value,
    _mir_func: &mir::MirFunction,
    _op: &Operand,
    _ctx: &FnCtx,
) {
    let addr = if offset == 0 {
        obj
    } else {
        b.ins().iadd_imm(obj, offset as i64)
    };
    match ty {
        MirType::Union(_) => {
            // val is a pointer to a 16-byte union block; memcpy it into the
            // field. For simplicity, do two 8-byte loads/stores.
            let lo = b.ins().load(types::I64, MemFlags::trusted(), val, 0);
            b.ins().store(MemFlags::trusted(), lo, addr, 0);
            let hi = b.ins().load(types::I64, MemFlags::trusted(), val, 8);
            b.ins().store(MemFlags::trusted(), hi, addr, 8);
        }
        _ => {
            b.ins().store(MemFlags::trusted(), val, addr, 0);
        }
    }
}

fn lower_alloc_generic<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    _func_ids: &HashMap<MirFnId, FuncId>,
    ctx: &mut FnCtx,
) -> Value {
    // AllocList/AllocMap/AllocClosure: hand off to the runtime entirely.
    // The runtime helper inspects the type id and lays out element/env
    // storage as it sees fit; this lowering only declares the entry point.
    let alloc = ensure_alloc(module, b, ctx);
    let size_v = b.ins().iconst(types::I64, GC_HEADER_SIZE as i64);
    let tid_v = b.ins().iconst(types::I64, 0);
    let call = b.ins().call(alloc, &[size_v, tid_v]);
    b.inst_results(call)[0]
}

fn lower_field<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    ctx: &mut FnCtx,
    mir_func: &mir::MirFunction,
    recv: &Operand,
    idx: u32,
    dest_ty: &MirType,
) -> Value {
    let recv_val = use_operand(module, b, func_ids, ctx, recv);
    // Find the struct definition behind `recv`.
    let recv_ty = operand_ty(mir_func, recv);
    let struct_mid = match recv_ty {
        MirType::ManagedRef(mid) | MirType::NullableRef(mid) => mid,
        MirType::Pointer(inner) => match *inner {
            MirType::ManagedRef(mid) => mid,
            _ => panic!("field access through Pointer(<non-managed>)"),
        },
        other => panic!("field access on {:?}", other),
    };
    let field = match &ctx.mir.types[&struct_mid] {
        MirTypeDef::Struct { fields, .. } => fields[idx as usize].clone(),
        MirTypeDef::Closure { env_fields, .. } => env_fields[idx as usize].clone(),
        other => panic!("field access on non-struct type def {:?}", other),
    };

    let load_ty = match &field.ty {
        MirType::Union(_) => {
            // Return the pointer to the field — caller (likely an
            // immediate consumer) treats it as a pointer to the union slot.
            if field.offset == 0 {
                return recv_val;
            }
            return b.ins().iadd_imm(recv_val, field.offset as i64);
        }
        other => clif_type(other),
    };
    let _ = dest_ty;
    let offset = field.offset as i32;
    b.ins().load(load_ty, MemFlags::trusted(), recv_val, offset)
}

fn lower_union_construct<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    ctx: &mut FnCtx,
    mir_func: &mir::MirFunction,
    tag: u32,
    payload: &Operand,
    dest_lid: LocalId,
) -> Value {
    // Destination union local already has its variable pre-bound to a
    // heap-allocated 16-byte block; write the tag and payload through it.
    let var = ctx.vars[&dest_lid];
    let addr = b.use_var(var);
    let tag_v = b.ins().iconst(types::I16, tag as i64);
    b.ins().store(MemFlags::trusted(), tag_v, addr, 0);
    let pty = operand_ty(mir_func, payload);
    let pv = use_operand(module, b, func_ids, ctx, payload);
    let widened = widen_to_payload(b, &pty, pv);
    b.ins().store(MemFlags::trusted(), widened, addr, 8);
    addr
}

fn widen_to_payload(b: &mut FunctionBuilder, ty: &MirType, v: Value) -> Value {
    match ty {
        MirType::Primitive(p) => {
            let ct = prim_clif(p);
            if ct == types::I64 || ct == types::F64 {
                if ct == types::F64 {
                    b.ins().bitcast(types::I64, MemFlags::trusted(), v)
                } else {
                    v
                }
            } else if is_float(p) {
                let i = b.ins().bitcast(types::I32, MemFlags::trusted(), v);
                b.ins().uextend(types::I64, i)
            } else if is_signed_int(p) {
                b.ins().sextend(types::I64, v)
            } else {
                b.ins().uextend(types::I64, v)
            }
        }
        _ => v, // pointers are already 8 bytes
    }
}

fn lower_union_tag<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    ctx: &mut FnCtx,
    _mir_func: &mir::MirFunction,
    scrut: &Operand,
) -> Value {
    let addr = use_operand(module, b, func_ids, ctx, scrut);
    // MIR's UnionTag rvalue produces a value typed `u16` — match that width.
    b.ins().load(types::I16, MemFlags::trusted(), addr, 0)
}

fn lower_union_payload<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    ctx: &mut FnCtx,
    _mir_func: &mir::MirFunction,
    scrut: &Operand,
    dest_ty: &MirType,
) -> Value {
    let addr = use_operand(module, b, func_ids, ctx, scrut);
    match dest_ty {
        MirType::Primitive(p) => {
            let ct = prim_clif(p);
            if ct == types::F64 {
                let i = b
                    .ins()
                    .load(types::I64, MemFlags::trusted(), addr, 8);
                b.ins().bitcast(types::F64, MemFlags::trusted(), i)
            } else if ct == types::F32 {
                let i = b
                    .ins()
                    .load(types::I32, MemFlags::trusted(), addr, 8);
                b.ins().bitcast(types::F32, MemFlags::trusted(), i)
            } else if ct.bits() == 64 {
                b.ins().load(types::I64, MemFlags::trusted(), addr, 8)
            } else {
                let wide = b
                    .ins()
                    .load(types::I64, MemFlags::trusted(), addr, 8);
                b.ins().ireduce(ct, wide)
            }
        }
        _ => b.ins().load(types::I64, MemFlags::trusted(), addr, 8),
    }
}

fn ensure_alloc<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    ctx: &mut FnCtx,
) -> ir::FuncRef {
    if let Some(r) = ctx.alloc_ref {
        return r;
    }
    let mut sig = Signature::new(CallConv::triple_default(module.isa().triple()));
    sig.params.push(AbiParam::new(types::I64)); // size
    sig.params.push(AbiParam::new(types::I64)); // type id
    sig.returns.push(AbiParam::new(types::I64));
    let fid = module
        .declare_function("__of_alloc", Linkage::Import, &sig)
        .expect("declare __of_alloc");
    let r = module.declare_func_in_func(fid, b.func);
    ctx.alloc_ref = Some(r);
    r
}

fn ensure_vtable_lookup<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    ctx: &mut FnCtx,
) -> ir::FuncRef {
    if let Some(r) = ctx.vtable_lookup_ref {
        return r;
    }
    let mut sig = Signature::new(CallConv::triple_default(module.isa().triple()));
    sig.params.push(AbiParam::new(types::I64)); // recv
    sig.params.push(AbiParam::new(types::I64)); // iface id
    sig.params.push(AbiParam::new(types::I64)); // slot idx
    sig.returns.push(AbiParam::new(types::I64));
    let fid = module
        .declare_function("__of_vtable_lookup", Linkage::Import, &sig)
        .expect("declare __of_vtable_lookup");
    let r = module.declare_func_in_func(fid, b.func);
    ctx.vtable_lookup_ref = Some(r);
    r
}

fn ensure_helper<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    ctx: &mut FnCtx,
    name: &'static str,
    params: &[Type],
    ret: Type,
) -> ir::FuncRef {
    if let Some(r) = ctx.helpers.get(name) {
        return *r;
    }
    let mut sig = Signature::new(CallConv::triple_default(module.isa().triple()));
    for p in params {
        sig.params.push(AbiParam::new(*p));
    }
    sig.returns.push(AbiParam::new(ret));
    let fid = module
        .declare_function(name, Linkage::Import, &sig)
        .unwrap_or_else(|_| panic!("declare {name}"));
    let r = module.declare_func_in_func(fid, b.func);
    ctx.helpers.insert(name, r);
    r
}

enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
}

fn int_cc(signed: bool, c: Cmp) -> IntCC {
    match (signed, c) {
        (true, Cmp::Lt) => IntCC::SignedLessThan,
        (true, Cmp::Le) => IntCC::SignedLessThanOrEqual,
        (true, Cmp::Gt) => IntCC::SignedGreaterThan,
        (true, Cmp::Ge) => IntCC::SignedGreaterThanOrEqual,
        (false, Cmp::Lt) => IntCC::UnsignedLessThan,
        (false, Cmp::Le) => IntCC::UnsignedLessThanOrEqual,
        (false, Cmp::Gt) => IntCC::UnsignedGreaterThan,
        (false, Cmp::Ge) => IntCC::UnsignedGreaterThanOrEqual,
    }
}

fn operand_prim(mir_func: &mir::MirFunction, op: &Operand) -> PrimitiveType {
    match op {
        Operand::Copy(l) | Operand::Move(l) => match &mir_func.locals[l].ty {
            MirType::Primitive(p) => p.clone(),
            other => panic!("scalar op on non-primitive operand: {other:?}"),
        },
        Operand::Const(c) => match c {
            MirConst::Int(_, p) | MirConst::Float(_, p) => p.clone(),
            MirConst::Bool(_) => PrimitiveType::Bool,
            MirConst::Char(_) => PrimitiveType::Char,
            MirConst::String(_) => PrimitiveType::String,
            MirConst::Null | MirConst::Fn(_) => {
                panic!("non-primitive const used in scalar op: {c:?}")
            }
        },
    }
}

fn operand_ty(mir_func: &mir::MirFunction, op: &Operand) -> MirType {
    match op {
        Operand::Copy(l) | Operand::Move(l) => mir_func.locals[l].ty.clone(),
        Operand::Const(c) => match c {
            MirConst::Int(_, p) | MirConst::Float(_, p) => MirType::Primitive(p.clone()),
            MirConst::Bool(_) => MirType::Primitive(PrimitiveType::Bool),
            MirConst::Char(_) => MirType::Primitive(PrimitiveType::Char),
            MirConst::String(_) => MirType::Primitive(PrimitiveType::String),
            MirConst::Null => MirType::Primitive(PrimitiveType::Int64),
            MirConst::Fn(_) => MirType::Primitive(PrimitiveType::Int64),
        },
    }
}

fn lower_terminator<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    ctx: &mut FnCtx,
    mir_func: &mir::MirFunction,
    t: &Terminator,
) {
    match t {
        Terminator::Goto(target) => {
            b.ins().jump(ctx.blocks[target], &[]);
        }
        Terminator::CondBr(cond, then_b, else_b) => {
            let c = use_operand(module, b, func_ids, ctx, cond);
            b.ins().brif(c, ctx.blocks[then_b], &[], ctx.blocks[else_b], &[]);
        }
        Terminator::Switch {
            scrutinee,
            arms,
            default,
        } => {
            let s = use_operand(module, b, func_ids, ctx, scrutinee);
            let mut sw = cranelift_frontend::Switch::new();
            for (val, target) in arms {
                sw.set_entry(*val as u128, ctx.blocks[target]);
            }
            sw.emit(b, s, ctx.blocks[default]);
        }
        Terminator::Return(None) => {
            b.ins().return_(&[]);
        }
        Terminator::Return(Some(op)) => {
            if matches!(mir_func.return_type, MirType::Unit) {
                b.ins().return_(&[]);
            } else {
                let v = use_operand(module, b, func_ids, ctx, op);
                b.ins().return_(&[v]);
            }
        }
        Terminator::Trap(reason) => {
            let code = match reason {
                TrapReason::AsMismatch => TrapCode::User(1),
                TrapReason::NullDeref => TrapCode::User(2),
            };
            b.ins().trap(code);
        }
        Terminator::Unreachable => {
            b.ins().trap(TrapCode::UnreachableCodeReached);
        }
    }
}

fn use_operand<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    ctx: &mut FnCtx,
    op: &Operand,
) -> Value {
    match op {
        Operand::Copy(l) | Operand::Move(l) => {
            let v = ctx
                .vars
                .get(l)
                .copied()
                .unwrap_or_else(|| panic!("operand uses Unit / unknown local {:?}", l));
            b.use_var(v)
        }
        Operand::Const(c) => match c {
            MirConst::Int(n, p) => b.ins().iconst(prim_clif(p), *n),
            MirConst::Float(f, PrimitiveType::Float32) => b.ins().f32const(*f as f32),
            MirConst::Float(f, _) => b.ins().f64const(*f),
            MirConst::Bool(v) => b.ins().iconst(types::I8, *v as i64),
            MirConst::Char(c) => b.ins().iconst(types::I32, *c as i64),
            MirConst::Null => b.ins().iconst(types::I64, 0),
            MirConst::String(s) => emit_string_const(module, b, s),
            MirConst::Fn(id) => emit_func_addr(module, b, func_ids, id),
        },
    }
}

fn emit_string_const<M: Module>(module: &mut M, b: &mut FunctionBuilder, s: &str) -> Value {
    let mut bytes = s.as_bytes().to_vec();
    bytes.push(0);
    let mut desc = DataDescription::new();
    desc.define(bytes.into_boxed_slice());
    let did = module.declare_anonymous_data(false, false).unwrap();
    module.define_data(did, &desc).unwrap();
    let gv: ir::GlobalValue = module.declare_data_in_func(did, b.func);
    b.ins().symbol_value(types::I64, gv)
}

fn emit_func_addr<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    id: &MirFnId,
) -> Value {
    let func_ref = module.declare_func_in_func(func_ids[id], b.func);
    b.ins().func_addr(types::I64, func_ref)
}

// Silence currently-unused imports that future passes will exercise.
#[allow(dead_code)]
fn _unused_imports() {
    let _ = (POINTER_SIZE, UNION_PAYLOAD_SIZE, NULL_TAG);
}
