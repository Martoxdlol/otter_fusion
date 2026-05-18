use std::collections::HashMap;

use cranelift_codegen::{
    ir::{
        self, AbiParam, Block, InstBuilder, Signature, TrapCode, Type, Value,
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
        self, Abi, AssignValue, BinOp, BlockId, Callee, LocalId, MirConst, MirFnId, MirProgram,
        MirType, Operand, Stmt, Terminator, TrapReason, UnOp,
    },
};

pub fn compile<M: Module>(mir: MirProgram, mut module: M) -> Result<M, ModuleError> {
    let mut function_ids = HashMap::new();

    for (id, f) in &mir.functions {
        let sig: Signature = build_signature(&module, f);
        let linkage = if f.id == mir.entry {
            Linkage::Export
        } else {
            // TODO: expose extern functions with body???
            Linkage::Local
        };
        let fid = module.declare_function(&f.name, linkage, &sig)?;
        function_ids.insert(*id, fid);
    }

    // Context for building functions
    let mut ctx = module.make_context();
    let mut builder_ctx = FunctionBuilderContext::new();
    for (id, f) in &mir.functions {
        if f.abi == Abi::Extern && f.blocks.is_empty() {
            continue;
        } // import only

        ctx.func.signature = build_signature(&module, f);
        lower_function(
            &mut module,
            &mut ctx.func,
            &mut builder_ctx,
            &function_ids,
            f,
        )?;
        module.define_function(function_ids[id], &mut ctx)?;
        module.clear_context(&mut ctx);
    }

    Ok(module)
}

fn build_signature<M: Module>(module: &M, f: &mir::MirFunction) -> Signature {
    let call_conv = match f.abi {
        Abi::Otter => module.target_config().default_call_conv,
        Abi::Extern => CallConv::triple_default(module.isa().triple()),
    };
    let mut sig = Signature::new(call_conv);
    for local_id in &f.params {
        let ty = clif_type(&f.locals[local_id].ty);
        sig.params.push(AbiParam::new(ty));
    }
    if !matches!(f.return_type, MirType::Unit) {
        sig.returns.push(AbiParam::new(clif_type(&f.return_type)));
    }
    sig
}

fn lower_function<M: Module>(
    module: &mut M,
    func: &mut ir::Function,
    builder_ctx: &mut FunctionBuilderContext,
    function_ids: &HashMap<MirFnId, FuncId>,
    mir_func: &mir::MirFunction,
) -> Result<(), ModuleError> {
    let mut b = FunctionBuilder::new(func, builder_ctx);

    let mut blocks: HashMap<BlockId, Block> = HashMap::new();
    for &bid in mir_func.blocks.keys() {
        blocks.insert(bid, b.create_block());
    }

    let entry = blocks[&mir_func.entry];

    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);

    let mut vars: HashMap<LocalId, Variable> = HashMap::new();
    for (lid, loc) in &mir_func.locals {
        let v = Variable::from_u32(lid.0);
        b.declare_var(v, clif_type(&loc.ty));
        vars.insert(*lid, v);
    }

    let entry_args = b.block_params(entry).to_vec();
    for (param_lid, val) in mir_func.params.iter().zip(entry_args) {
        b.def_var(vars[param_lid], val);
    }

    for (bid, blk) in &mir_func.blocks {
        let cb = blocks[bid];
        b.switch_to_block(cb);
        for stmt in &blk.stmts {
            lower_stmt(module, &mut b, function_ids, &vars, &blocks, mir_func, stmt);
        }
        lower_terminator(
            module,
            &mut b,
            function_ids,
            &vars,
            &blocks,
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
    vars: &HashMap<LocalId, Variable>,
    _blocks: &HashMap<BlockId, Block>,
    mir_func: &mir::MirFunction,
    stmt: &mir::Stmt,
) {
    match stmt {
        Stmt::Assign(lid, av) => {
            let v = lower_rvalue(module, b, func_ids, vars, mir_func, av);
            b.def_var(vars[lid], v);
        }
    }
}

fn lower_rvalue<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    vars: &HashMap<LocalId, Variable>,
    mir_func: &mir::MirFunction,
    av: &AssignValue,
) -> Value {
    match av {
        AssignValue::Use(op) => use_operand(module, b, func_ids, vars, op),
        AssignValue::Bin(op, l, r) => lower_bin(module, b, func_ids, vars, mir_func, *op, l, r),
        AssignValue::Un(op, x) => lower_un(module, b, func_ids, vars, mir_func, *op, x),
        AssignValue::Cast(op, target) => {
            lower_cast(module, b, func_ids, vars, mir_func, op, target)
        }
        AssignValue::Call(callee, args) => {
            lower_call(module, b, func_ids, vars, mir_func, callee, args)
        }
        AssignValue::AllocStruct(_, _)
        | AssignValue::AllocList(_, _)
        | AssignValue::AllocMap(_, _, _)
        | AssignValue::AllocClosure(_, _)
        | AssignValue::Field(_, _)
        | AssignValue::UnionConstruct(_, _, _)
        | AssignValue::UnionTag(_)
        | AssignValue::UnionPayload(_, _) => {
            todo!("rvalue requires runtime/layout context: {av:?}")
        }
    }
}

fn lower_bin<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    vars: &HashMap<LocalId, Variable>,
    mir_func: &mir::MirFunction,
    op: BinOp,
    l: &Operand,
    r: &Operand,
) -> Value {
    let prim = operand_prim(mir_func, l);
    let lv = use_operand(module, b, func_ids, vars, l);
    let rv = use_operand(module, b, func_ids, vars, r);
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
    vars: &HashMap<LocalId, Variable>,
    mir_func: &mir::MirFunction,
    op: UnOp,
    x: &Operand,
) -> Value {
    let prim = operand_prim(mir_func, x);
    let xv = use_operand(module, b, func_ids, vars, x);
    match op {
        UnOp::Neg if is_float(&prim) => b.ins().fneg(xv),
        UnOp::Neg => b.ins().ineg(xv),
        // bool is i8 with values 0/1 → flip the low bit.
        UnOp::Not => b.ins().bxor_imm(xv, 1),
    }
}

fn lower_cast<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    vars: &HashMap<LocalId, Variable>,
    mir_func: &mir::MirFunction,
    op: &Operand,
    target: &MirType,
) -> Value {
    let src = operand_prim(mir_func, op);
    let dst = match target {
        MirType::Primitive(p) => p.clone(),
        _ => unreachable!("Cast target must be primitive"),
    };
    let v = use_operand(module, b, func_ids, vars, op);
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

fn lower_call<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    vars: &HashMap<LocalId, Variable>,
    mir_func: &mir::MirFunction,
    callee: &Callee,
    args: &[Operand],
) -> Value {
    let arg_vals: Vec<Value> = args
        .iter()
        .map(|a| use_operand(module, b, func_ids, vars, a))
        .collect();
    match callee {
        Callee::Static(target) => {
            let func_ref = module.declare_func_in_func(func_ids[target], b.func);
            let inst = b.ins().call(func_ref, &arg_vals);
            b.inst_results(inst)
                .first()
                .copied()
                .expect("static call assigned to local must return a value")
        }
        Callee::Indirect(_) | Callee::Virtual(_, _, _) => {
            let _ = (module, mir_func, arg_vals);
            todo!("indirect/virtual calls require runtime/vtable lowering")
        }
    }
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

pub fn clif_type(t: &MirType) -> Type {
    match t {
        MirType::Primitive(p) => match p {
            PrimitiveType::Int8 | PrimitiveType::Uint8 | PrimitiveType::Bool => types::I8,
            PrimitiveType::Int16 | PrimitiveType::Uint16 => types::I16,
            PrimitiveType::Int32 | PrimitiveType::Uint32 | PrimitiveType::Char => types::I32,
            PrimitiveType::Int64 | PrimitiveType::Uint64 => types::I64,
            PrimitiveType::Float32 => types::F32,
            PrimitiveType::Float64 => types::F64,
            PrimitiveType::String => types::I64, // pointer to managed string
        },
        // Every reference type is a pointer.
        MirType::ManagedRef(_)
        | MirType::NullableRef(_)
        | MirType::Pointer(_)
        | MirType::Closure(_)
        | MirType::FnPtr(_, _) => types::I64,
        // Unions are 16 bytes; pass by pointer or as a (i16, i64) pair.
        // We pick "by pointer" here for uniformity.
        MirType::Union(_) => types::I64,
        MirType::Unit => unreachable!("Unit has no CLIF representation; callers must guard"),
    }
}

pub fn lower_terminator<M: Module>(
    module: &mut M,
    b: &mut FunctionBuilder,
    func_ids: &HashMap<MirFnId, FuncId>,
    vars: &HashMap<LocalId, Variable>,
    blocks: &HashMap<BlockId, Block>,
    t: &Terminator,
) {
    match t {
        Terminator::Goto(target) => {
            b.ins().jump(blocks[target], &[]);
        }
        Terminator::CondBr(cond, then_b, else_b) => {
            let c = use_operand(module, b, func_ids, vars, cond);
            b.ins().brif(c, blocks[then_b], &[], blocks[else_b], &[]);
        }
        Terminator::Switch {
            scrutinee,
            arms,
            default,
        } => {
            let s = use_operand(module, b, func_ids, vars, scrutinee);
            // Cranelift has no native switch, but cranelift-frontend ships
            // a Switch helper that picks br_table or an if-chain for you.
            let mut sw = cranelift_frontend::Switch::new();
            for (val, target) in arms {
                sw.set_entry(*val as u128, blocks[target]);
            }
            sw.emit(b, s, blocks[default]);
        }
        Terminator::Return(None) => {
            b.ins().return_(&[]);
        }
        Terminator::Return(Some(op)) => {
            let v = use_operand(module, b, func_ids, vars, op);
            b.ins().return_(&[v]);
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
    vars: &HashMap<LocalId, Variable>,
    op: &Operand,
) -> Value {
    match op {
        Operand::Copy(l) | Operand::Move(l) => b.use_var(vars[l]),
        Operand::Const(c) => match c {
            MirConst::Int(n, p) => b.ins().iconst(int_ty(p), *n),
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
    bytes.push(0); // null-terminate so the runtime can treat it as a C string too
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

fn int_ty(p: &PrimitiveType) -> Type {
    match p {
        PrimitiveType::Int8 | PrimitiveType::Uint8 => types::I8,
        PrimitiveType::Int16 | PrimitiveType::Uint16 => types::I16,
        PrimitiveType::Int32 | PrimitiveType::Uint32 => types::I32,
        PrimitiveType::Int64 | PrimitiveType::Uint64 => types::I64,
        _ => unreachable!("int_ty called on non-integer primitive"),
    }
}
