use std::collections::HashMap;

use cranelift::codegen;
use cranelift::frontend::Switch;
use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::hir::PrimitiveType;
use crate::mir::{
    Abi, AssignValue, BinOp, BlockId, Callee, LocalId, MirConst, MirFnId, MirFunction, MirProgram,
    MirType, MirTypeDef, MirTypeId, Operand, Stmt, Terminator, TrapReason, UnOp,
};

/// Symbol exported for the program's entry function. The runtime crate
/// provides a `main` shim that calls into this.
pub const ENTRY_SYMBOL: &str = "otter_main";

/// MIR -> Cranelift IR. Generic over the cranelift `Module` backend so
/// the same translation drives both JIT (`JITModule`) and AOT
/// (`ObjectModule`) compilation. Use [`Codegen::jit`] or
/// [`Codegen::object`] to construct.
pub struct Codegen<M: Module> {
    module: M,
    fn_ids: HashMap<MirFnId, FuncId>,
    helpers: Helpers,
    ptr: Type,
}

struct Helpers {
    alloc_struct: FuncId,
    alloc_closure: FuncId,
    alloc_env: FuncId,
    union_construct: FuncId,
    union_tag: FuncId,
    union_payload: FuncId,
    vcall_lookup: FuncId,
    trap: FuncId,
}

impl<M: Module> Codegen<M> {
    fn from_module(mut module: M) -> Result<Self, String> {
        let ptr = module.target_config().pointer_type();
        let helpers = declare_helpers(&mut module, ptr)?;
        Ok(Self {
            module,
            fn_ids: HashMap::new(),
            helpers,
            ptr,
        })
    }

    /// Translate every MIR function into the underlying module. Does NOT
    /// finalize — call `finish_jit` or `finish_object` for that.
    pub fn compile(&mut self, program: &MirProgram) -> Result<(), String> {
        // Pass 1: declare every function so calls can resolve forward refs.
        for (id, f) in &program.functions {
            let sig = self.signature_of(f);
            let (name, linkage) = symbol_for(*id, f, program.entry);
            let cl_id = self
                .module
                .declare_function(&name, linkage, &sig)
                .map_err(|e| e.to_string())?;
            self.fn_ids.insert(*id, cl_id);
        }

        // Pass 2: define each Otter-ABI body. Externs stay as imports.
        let mut ctx = self.module.make_context();
        let mut fbx = FunctionBuilderContext::new();
        for (id, f) in &program.functions {
            if matches!(f.abi, Abi::Extern) {
                continue;
            }
            ctx.func.signature = self.signature_of(f);
            let cl_id = self.fn_ids[id];
            {
                let builder = FunctionBuilder::new(&mut ctx.func, &mut fbx);
                let t = FnTrans {
                    builder,
                    module: &mut self.module,
                    fn_ids: &self.fn_ids,
                    helpers: &self.helpers,
                    program,
                    f,
                    cl_blocks: HashMap::new(),
                    vars: HashMap::new(),
                    ptr: self.ptr,
                };
                t.translate();
            }
            self.module
                .define_function(cl_id, &mut ctx)
                .map_err(|e| e.to_string())?;
            self.module.clear_context(&mut ctx);
        }
        Ok(())
    }

    fn signature_of(&self, f: &MirFunction) -> Signature {
        let mut sig = self.module.make_signature();
        for pid in &f.params {
            let local = &f.locals[pid];
            sig.params.push(AbiParam::new(cl_type(&local.ty, self.ptr)));
        }
        sig.returns
            .push(AbiParam::new(cl_type(&f.return_type, self.ptr)));
        sig
    }
}

impl Codegen<JITModule> {
    /// Build a JIT-backed Codegen. `symbols` are name→address pairs that
    /// the JIT will use to resolve unresolved imports — pass the runtime
    /// helpers (`otter_*`) plus any externs your programs reference.
    pub fn jit(symbols: &[(&str, *const u8)]) -> Result<Self, String> {
        let isa_builder = cranelift_native::builder().map_err(|e| e.to_string())?;
        let isa = isa_builder
            .finish(settings::Flags::new(settings::builder()))
            .map_err(|e| e.to_string())?;
        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        for (name, addr) in symbols {
            builder.symbol(*name, *addr);
        }
        let module = JITModule::new(builder);
        Self::from_module(module)
    }

    /// Finalize and return the machine address of `entry`.
    pub fn finish(mut self, entry: MirFnId) -> Result<*const u8, String> {
        self.module
            .finalize_definitions()
            .map_err(|e| e.to_string())?;
        let id = self.fn_ids[&entry];
        Ok(self.module.get_finalized_function(id))
    }
}

impl Codegen<ObjectModule> {
    /// Build an object-file-backed Codegen. `name` is the SO name written
    /// into the object's metadata; pass the source file stem.
    pub fn object(name: &str) -> Result<Self, String> {
        let isa_builder = cranelift_native::builder().map_err(|e| e.to_string())?;
        let isa = isa_builder
            .finish(settings::Flags::new(settings::builder()))
            .map_err(|e| e.to_string())?;
        let builder = ObjectBuilder::new(
            isa,
            name.to_string(),
            cranelift_module::default_libcall_names(),
        )
        .map_err(|e| e.to_string())?;
        let module = ObjectModule::new(builder);
        Self::from_module(module)
    }

    /// Emit the linkable object file as a byte vector. Write it to disk
    /// and link with `libotter_runtime.a`.
    pub fn finish(self) -> Result<Vec<u8>, String> {
        let product = self.module.finish();
        product.emit().map_err(|e| e.to_string())
    }
}

/// Pick the symbol name and linkage for a MIR function. The entry
/// function is exported under [`ENTRY_SYMBOL`]; externs keep their
/// declared name (sanitized) so the linker can resolve them; everything
/// else gets a `name__id` form to avoid collisions between
/// monomorphizations.
fn symbol_for(id: MirFnId, f: &MirFunction, entry: MirFnId) -> (String, Linkage) {
    if id == entry {
        return (ENTRY_SYMBOL.to_string(), Linkage::Export);
    }
    match f.abi {
        Abi::Extern => (sanitize(&f.name), Linkage::Import),
        Abi::Otter => (format!("{}__{}", sanitize(&f.name), id.0), Linkage::Local),
    }
}

/// Replace anything that isn't an identifier-safe character with `_`.
/// Lower's MIR names look like `add<i32>` which most assemblers reject.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '$' | '.' => c,
            _ => '_',
        })
        .collect()
}

fn declare_helpers<M: Module>(module: &mut M, ptr: Type) -> Result<Helpers, String> {
    fn dec<M: Module>(
        module: &mut M,
        name: &str,
        params: &[Type],
        ret: Option<Type>,
    ) -> Result<FuncId, String> {
        let mut sig = module.make_signature();
        for p in params {
            sig.params.push(AbiParam::new(*p));
        }
        if let Some(r) = ret {
            sig.returns.push(AbiParam::new(r));
        }
        module
            .declare_function(name, Linkage::Import, &sig)
            .map_err(|e| e.to_string())
    }

    Ok(Helpers {
        alloc_struct: dec(module, "otter_alloc_struct", &[types::I32], Some(ptr))?,
        alloc_closure: dec(module, "otter_alloc_closure", &[ptr, ptr], Some(ptr))?,
        alloc_env: dec(module, "otter_alloc_env", &[types::I32], Some(ptr))?,
        union_construct: dec(
            module,
            "otter_union_construct",
            &[types::I32, types::I32, ptr],
            Some(ptr),
        )?,
        union_tag: dec(module, "otter_union_tag", &[ptr], Some(types::I32))?,
        union_payload: dec(module, "otter_union_payload", &[ptr], Some(ptr))?,
        vcall_lookup: dec(
            module,
            "otter_vcall_lookup",
            &[ptr, types::I32, types::I32],
            Some(ptr),
        )?,
        trap: dec(module, "otter_trap", &[types::I32], None)?,
    })
}

struct FnTrans<'a, 'b, M: Module> {
    builder: FunctionBuilder<'b>,
    module: &'a mut M,
    fn_ids: &'a HashMap<MirFnId, FuncId>,
    helpers: &'a Helpers,
    program: &'a MirProgram,
    f: &'a MirFunction,
    cl_blocks: HashMap<BlockId, Block>,
    vars: HashMap<LocalId, Variable>,
    ptr: Type,
}

impl<'a, 'b, M: Module> FnTrans<'a, 'b, M> {
    fn translate(mut self) {
        // One Cranelift block per MIR block.
        for id in self.f.blocks.keys() {
            let b = self.builder.create_block();
            self.cl_blocks.insert(*id, b);
        }

        // Declare every local as a Variable. SSA construction handles
        // re-reads across blocks for us.
        let mut next_var: usize = 0;
        for (id, local) in &self.f.locals {
            let var = Variable::new(next_var);
            next_var += 1;
            self.builder.declare_var(var, cl_type(&local.ty, self.ptr));
            self.vars.insert(*id, var);
        }

        // Walk blocks with entry first so its incoming params are bound
        // before we translate any other block that branches into it.
        let mut ids: Vec<BlockId> = self.f.blocks.keys().copied().collect();
        ids.sort_by_key(|b| if *b == self.f.entry { 0u32 } else { b.0 + 1 });

        for id in ids {
            let cl = self.cl_blocks[&id];
            self.builder.switch_to_block(cl);

            if id == self.f.entry {
                self.builder.append_block_params_for_function_params(cl);
                let params = self.builder.block_params(cl).to_vec();
                for (i, pid) in self.f.params.iter().enumerate() {
                    let var = self.vars[pid];
                    self.builder.def_var(var, params[i]);
                }
            }

            let block = &self.f.blocks[&id];
            for s in &block.stmts {
                self.translate_stmt(s);
            }
            self.translate_terminator(&block.terminator);
        }

        self.builder.seal_all_blocks();
        self.builder.finalize();
    }

    fn translate_stmt(&mut self, s: &Stmt) {
        let Stmt::Assign(dst, av) = s;
        let val = self.translate_assign(av, *dst);
        let var = self.vars[dst];
        self.builder.def_var(var, val);
    }

    fn translate_assign(&mut self, av: &AssignValue, dst: LocalId) -> Value {
        match av {
            AssignValue::Use(o) => self.operand(o),
            AssignValue::Bin(op, a, b) => {
                let lhs_ty = self.operand_mir_type(a);
                let av = self.operand(a);
                let bv = self.operand(b);
                self.bin_op(op, av, bv, &lhs_ty, &self.f.locals[&dst].ty.clone())
            }
            AssignValue::Un(op, x) => {
                let v = self.operand(x);
                let ty = self.f.locals[&dst].ty.clone();
                self.un_op(op, v, &ty)
            }
            AssignValue::Cast(o, target) => {
                let v = self.operand(o);
                let src_ty = self.operand_mir_type(o);
                self.cast(v, &src_ty, target)
            }
            AssignValue::Call(callee, args) => self.call(callee, args, dst),
            AssignValue::AllocStruct(tid, fields) => self.alloc_struct(*tid, fields),
            AssignValue::AllocList(_, _) | AssignValue::AllocMap(_, _, _) => {
                self.builder.ins().iconst(self.ptr, 0)
            }
            AssignValue::AllocClosure(fid, env) => self.alloc_closure(*fid, env),
            AssignValue::Field(base, idx) => {
                self.field_load(base, *idx, &self.f.locals[&dst].ty.clone())
            }
            AssignValue::UnionConstruct(tid, tag, payload) => {
                self.union_construct(*tid, *tag, payload)
            }
            AssignValue::UnionTag(o) => {
                let v = self.operand(o);
                let helper = self
                    .module
                    .declare_func_in_func(self.helpers.union_tag, self.builder.func);
                let inst = self.builder.ins().call(helper, &[v]);
                let r = self.builder.inst_results(inst)[0];
                self.builder.ins().ireduce(types::I16, r)
            }
            AssignValue::UnionPayload(o, _tid) => {
                let v = self.operand(o);
                let helper = self
                    .module
                    .declare_func_in_func(self.helpers.union_payload, self.builder.func);
                let inst = self.builder.ins().call(helper, &[v]);
                self.builder.inst_results(inst)[0]
            }
        }
    }

    fn translate_terminator(&mut self, t: &Terminator) {
        match t {
            Terminator::Goto(b) => {
                let cl = self.cl_blocks[b];
                self.builder.ins().jump(cl, &[]);
            }
            Terminator::CondBr(c, then_b, else_b) => {
                let v = self.operand(c);
                let then_cl = self.cl_blocks[then_b];
                let else_cl = self.cl_blocks[else_b];
                self.builder.ins().brif(v, then_cl, &[], else_cl, &[]);
            }
            Terminator::Switch {
                scrutinee,
                arms,
                default,
            } => {
                let v = self.operand(scrutinee);
                let default_cl = self.cl_blocks[default];
                let mut switch = Switch::new();
                for (val, blk) in arms {
                    switch.set_entry(*val as u128, self.cl_blocks[blk]);
                }
                switch.emit(&mut self.builder, v, default_cl);
            }
            Terminator::Return(opt) => match opt {
                Some(o) => {
                    let v = self.operand(o);
                    self.builder.ins().return_(&[v]);
                }
                None => {
                    let zero = self
                        .builder
                        .ins()
                        .iconst(cl_type(&self.f.return_type, self.ptr), 0);
                    self.builder.ins().return_(&[zero]);
                }
            },
            Terminator::Trap(reason) => {
                let r = match reason {
                    TrapReason::AsMismatch => 1,
                    TrapReason::NullDeref => 2,
                };
                let helper = self
                    .module
                    .declare_func_in_func(self.helpers.trap, self.builder.func);
                let code = self.builder.ins().iconst(types::I32, r as i64);
                self.builder.ins().call(helper, &[code]);
                self.builder.ins().trap(TrapCode::user(1).unwrap());
            }
            Terminator::Unreachable => {
                self.builder.ins().trap(TrapCode::user(2).unwrap());
            }
        }
    }

    fn operand(&mut self, o: &Operand) -> Value {
        match o {
            Operand::Copy(l) | Operand::Move(l) => {
                let var = self.vars[l];
                self.builder.use_var(var)
            }
            Operand::Const(c) => self.const_val(c),
        }
    }

    fn operand_mir_type(&self, o: &Operand) -> MirType {
        match o {
            Operand::Copy(l) | Operand::Move(l) => self.f.locals[l].ty.clone(),
            Operand::Const(c) => const_mir_type(c),
        }
    }

    fn const_val(&mut self, c: &MirConst) -> Value {
        match c {
            MirConst::Int(v, p) => self.builder.ins().iconst(prim_type(p), *v),
            MirConst::Float(v, p) => match p {
                PrimitiveType::Float32 => self.builder.ins().f32const(*v as f32),
                PrimitiveType::Float64 => self.builder.ins().f64const(*v),
                _ => unreachable!("non-float primitive in MirConst::Float"),
            },
            MirConst::Bool(b) => self.builder.ins().iconst(types::I8, if *b { 1 } else { 0 }),
            MirConst::Char(ch) => self.builder.ins().iconst(types::I32, *ch as i64),
            MirConst::Null => self.builder.ins().iconst(self.ptr, 0),
            MirConst::Fn(fid) => {
                let func_id = self.fn_ids[fid];
                let local = self.module.declare_func_in_func(func_id, self.builder.func);
                self.builder.ins().func_addr(self.ptr, local)
            }
            MirConst::String(_) => self.builder.ins().iconst(self.ptr, 0),
        }
    }

    fn bin_op(
        &mut self,
        op: &BinOp,
        a: Value,
        b: Value,
        lhs_ty: &MirType,
        _dst_ty: &MirType,
    ) -> Value {
        let signed = is_signed(lhs_ty);
        let float = is_float(lhs_ty);

        match op {
            BinOp::Add => {
                if float {
                    self.builder.ins().fadd(a, b)
                } else {
                    self.builder.ins().iadd(a, b)
                }
            }
            BinOp::Sub => {
                if float {
                    self.builder.ins().fsub(a, b)
                } else {
                    self.builder.ins().isub(a, b)
                }
            }
            BinOp::Mul => {
                if float {
                    self.builder.ins().fmul(a, b)
                } else {
                    self.builder.ins().imul(a, b)
                }
            }
            BinOp::Div => {
                if float {
                    self.builder.ins().fdiv(a, b)
                } else if signed {
                    self.builder.ins().sdiv(a, b)
                } else {
                    self.builder.ins().udiv(a, b)
                }
            }
            BinOp::Mod => {
                if float {
                    panic!("float modulo needs a libcall (fmod) — not yet wired up");
                } else if signed {
                    self.builder.ins().srem(a, b)
                } else {
                    self.builder.ins().urem(a, b)
                }
            }
            BinOp::And => self.builder.ins().band(a, b),
            BinOp::Or => self.builder.ins().bor(a, b),
            BinOp::Eq => {
                if float {
                    self.builder.ins().fcmp(FloatCC::Equal, a, b)
                } else {
                    self.builder.ins().icmp(IntCC::Equal, a, b)
                }
            }
            BinOp::Neq => {
                if float {
                    self.builder.ins().fcmp(FloatCC::NotEqual, a, b)
                } else {
                    self.builder.ins().icmp(IntCC::NotEqual, a, b)
                }
            }
            BinOp::Lt => {
                if float {
                    self.builder.ins().fcmp(FloatCC::LessThan, a, b)
                } else if signed {
                    self.builder.ins().icmp(IntCC::SignedLessThan, a, b)
                } else {
                    self.builder.ins().icmp(IntCC::UnsignedLessThan, a, b)
                }
            }
            BinOp::Le => {
                if float {
                    self.builder.ins().fcmp(FloatCC::LessThanOrEqual, a, b)
                } else if signed {
                    self.builder.ins().icmp(IntCC::SignedLessThanOrEqual, a, b)
                } else {
                    self.builder
                        .ins()
                        .icmp(IntCC::UnsignedLessThanOrEqual, a, b)
                }
            }
            BinOp::Gt => {
                if float {
                    self.builder.ins().fcmp(FloatCC::GreaterThan, a, b)
                } else if signed {
                    self.builder.ins().icmp(IntCC::SignedGreaterThan, a, b)
                } else {
                    self.builder.ins().icmp(IntCC::UnsignedGreaterThan, a, b)
                }
            }
            BinOp::Ge => {
                if float {
                    self.builder.ins().fcmp(FloatCC::GreaterThanOrEqual, a, b)
                } else if signed {
                    self.builder
                        .ins()
                        .icmp(IntCC::SignedGreaterThanOrEqual, a, b)
                } else {
                    self.builder
                        .ins()
                        .icmp(IntCC::UnsignedGreaterThanOrEqual, a, b)
                }
            }
        }
    }

    fn un_op(&mut self, op: &UnOp, v: Value, ty: &MirType) -> Value {
        match op {
            UnOp::Neg => {
                if is_float(ty) {
                    self.builder.ins().fneg(v)
                } else {
                    self.builder.ins().ineg(v)
                }
            }
            UnOp::Not => {
                let one = self.builder.ins().iconst(types::I8, 1);
                self.builder.ins().bxor(v, one)
            }
        }
    }

    fn cast(&mut self, v: Value, src: &MirType, dst: &MirType) -> Value {
        let st = cl_type(src, self.ptr);
        let dt = cl_type(dst, self.ptr);
        if st == dt {
            return v;
        }
        let src_float = is_float(src);
        let dst_float = is_float(dst);
        let src_signed = is_signed(src);

        match (src_float, dst_float) {
            (true, true) => {
                if dt.bits() > st.bits() {
                    self.builder.ins().fpromote(dt, v)
                } else {
                    self.builder.ins().fdemote(dt, v)
                }
            }
            (true, false) => {
                if is_signed(dst) {
                    self.builder.ins().fcvt_to_sint(dt, v)
                } else {
                    self.builder.ins().fcvt_to_uint(dt, v)
                }
            }
            (false, true) => {
                if src_signed {
                    self.builder.ins().fcvt_from_sint(dt, v)
                } else {
                    self.builder.ins().fcvt_from_uint(dt, v)
                }
            }
            (false, false) => {
                if dt.bits() > st.bits() {
                    if src_signed {
                        self.builder.ins().sextend(dt, v)
                    } else {
                        self.builder.ins().uextend(dt, v)
                    }
                } else {
                    self.builder.ins().ireduce(dt, v)
                }
            }
        }
    }

    fn call(&mut self, callee: &Callee, args: &[Operand], dst: LocalId) -> Value {
        let arg_vals: Vec<Value> = args.iter().map(|o| self.operand(o)).collect();
        let dst_ty = cl_type(&self.f.locals[&dst].ty, self.ptr);

        let inst = match callee {
            Callee::Static(fid) => {
                let func_id = self.fn_ids[fid];
                let local = self.module.declare_func_in_func(func_id, self.builder.func);
                self.builder.ins().call(local, &arg_vals)
            }
            Callee::Indirect(o) => {
                let f = self.operand(o);
                let sig = self.import_signature(&arg_vals, dst_ty);
                self.builder.ins().call_indirect(sig, f, &arg_vals)
            }
            Callee::Virtual(recv, iface_mid, slot) => {
                let recv_v = self.operand(recv);
                let iface_id = self.builder.ins().iconst(types::I32, iface_mid.0 as i64);
                let slot_v = self.builder.ins().iconst(types::I32, *slot as i64);
                let helper = self
                    .module
                    .declare_func_in_func(self.helpers.vcall_lookup, self.builder.func);
                let lookup = self.builder.ins().call(helper, &[recv_v, iface_id, slot_v]);
                let fnptr = self.builder.inst_results(lookup)[0];
                let sig = self.import_signature(&arg_vals, dst_ty);
                self.builder.ins().call_indirect(sig, fnptr, &arg_vals)
            }
        };
        let results = self.builder.inst_results(inst);
        if results.is_empty() {
            self.builder.ins().iconst(self.ptr, 0)
        } else {
            results[0]
        }
    }

    fn import_signature(&mut self, args: &[Value], ret: Type) -> codegen::ir::SigRef {
        let mut sig = self.module.make_signature();
        for v in args {
            sig.params
                .push(AbiParam::new(self.builder.func.dfg.value_type(*v)));
        }
        sig.returns.push(AbiParam::new(ret));
        self.builder.import_signature(sig)
    }

    fn alloc_struct(&mut self, tid: MirTypeId, fields: &[Operand]) -> Value {
        let helper = self
            .module
            .declare_func_in_func(self.helpers.alloc_struct, self.builder.func);
        let tid_v = self.builder.ins().iconst(types::I32, tid.0 as i64);
        let inst = self.builder.ins().call(helper, &[tid_v]);
        let base = self.builder.inst_results(inst)[0];

        let offsets = field_offsets(self.program, tid);
        for (i, op) in fields.iter().enumerate() {
            let v = self.operand(op);
            let off = offsets[i] as i32;
            self.builder.ins().store(MemFlags::new(), v, base, off);
        }
        base
    }

    fn alloc_closure(&mut self, fid: MirFnId, env: &[Operand]) -> Value {
        let env_mid = closure_env_type(self.program, fid);
        let alloc_env = self
            .module
            .declare_func_in_func(self.helpers.alloc_env, self.builder.func);
        let env_tid_v = self.builder.ins().iconst(types::I32, env_mid.0 as i64);
        let inst = self.builder.ins().call(alloc_env, &[env_tid_v]);
        let env_base = self.builder.inst_results(inst)[0];

        let offsets = field_offsets(self.program, env_mid);
        for (i, op) in env.iter().enumerate() {
            let v = self.operand(op);
            let off = offsets[i] as i32;
            self.builder.ins().store(MemFlags::new(), v, env_base, off);
        }

        let func_id = self.fn_ids[&fid];
        let fref = self.module.declare_func_in_func(func_id, self.builder.func);
        let fn_ptr = self.builder.ins().func_addr(self.ptr, fref);

        let alloc_cl = self
            .module
            .declare_func_in_func(self.helpers.alloc_closure, self.builder.func);
        let inst = self.builder.ins().call(alloc_cl, &[fn_ptr, env_base]);
        self.builder.inst_results(inst)[0]
    }

    fn field_load(&mut self, base: &Operand, idx: u32, dst_ty: &MirType) -> Value {
        let recv = self.operand(base);
        let mid = match self.operand_mir_type(base) {
            MirType::ManagedRef(m) | MirType::Closure(m) => m,
            MirType::Pointer(inner) => match *inner {
                MirType::ManagedRef(m) => m,
                _ => panic!("field load through non-aggregate pointer"),
            },
            other => panic!("field load on non-aggregate: {:?}", other),
        };
        let offsets = field_offsets(self.program, mid);
        let off = offsets[idx as usize] as i32;
        self.builder
            .ins()
            .load(cl_type(dst_ty, self.ptr), MemFlags::new(), recv, off)
    }

    fn union_construct(&mut self, tid: MirTypeId, tag: u32, payload: &Operand) -> Value {
        let p = self.operand(payload);
        let p = if self.builder.func.dfg.value_type(p) != self.ptr {
            self.builder.ins().uextend(self.ptr, p)
        } else {
            p
        };
        let helper = self
            .module
            .declare_func_in_func(self.helpers.union_construct, self.builder.func);
        let tid_v = self.builder.ins().iconst(types::I32, tid.0 as i64);
        let tag_v = self.builder.ins().iconst(types::I32, tag as i64);
        let inst = self.builder.ins().call(helper, &[tid_v, tag_v, p]);
        self.builder.inst_results(inst)[0]
    }
}

fn field_offsets(program: &MirProgram, mid: MirTypeId) -> Vec<u32> {
    let fields: &[crate::mir::MirField] = match program.types.get(&mid) {
        Some(MirTypeDef::Struct { fields, .. }) => fields,
        Some(MirTypeDef::Closure { env_fields, .. }) => env_fields,
        _ => panic!("field_offsets on non-aggregate type {:?}", mid),
    };
    let mut offs = Vec::with_capacity(fields.len());
    let mut cur: u32 = 0;
    for f in fields {
        let a = mir_align(&f.ty).max(1);
        cur = align_up(cur, a);
        offs.push(cur);
        cur += mir_size(&f.ty);
    }
    offs
}

fn closure_env_type(program: &MirProgram, fid: MirFnId) -> MirTypeId {
    for (id, def) in &program.types {
        if let MirTypeDef::Closure { function, .. } = def {
            if *function == fid {
                return *id;
            }
        }
    }
    panic!("no MirTypeDef::Closure registered for fn {:?}", fid);
}

fn cl_type(ty: &MirType, ptr: Type) -> Type {
    match ty {
        MirType::Primitive(p) => prim_type(p),
        MirType::ManagedRef(_)
        | MirType::Pointer(_)
        | MirType::FnPtr(_, _)
        | MirType::Closure(_)
        | MirType::NullableRef(_)
        | MirType::Union(_) => ptr,
    }
}

fn prim_type(p: &PrimitiveType) -> Type {
    match p {
        PrimitiveType::Bool | PrimitiveType::Int8 | PrimitiveType::Uint8 => types::I8,
        PrimitiveType::Int16 | PrimitiveType::Uint16 => types::I16,
        PrimitiveType::Int32 | PrimitiveType::Uint32 | PrimitiveType::Char => types::I32,
        PrimitiveType::Int64 | PrimitiveType::Uint64 => types::I64,
        PrimitiveType::Float32 => types::F32,
        PrimitiveType::Float64 => types::F64,
        PrimitiveType::String => types::I64,
    }
}

fn const_mir_type(c: &MirConst) -> MirType {
    match c {
        MirConst::Int(_, p) | MirConst::Float(_, p) => MirType::Primitive(p.clone()),
        MirConst::Bool(_) => MirType::Primitive(PrimitiveType::Bool),
        MirConst::Char(_) => MirType::Primitive(PrimitiveType::Char),
        MirConst::String(_) => MirType::Primitive(PrimitiveType::String),
        MirConst::Null => MirType::Primitive(PrimitiveType::Bool),
        MirConst::Fn(_) => MirType::FnPtr(
            Vec::new(),
            Box::new(MirType::Primitive(PrimitiveType::Bool)),
        ),
    }
}

fn is_float(t: &MirType) -> bool {
    matches!(
        t,
        MirType::Primitive(PrimitiveType::Float32 | PrimitiveType::Float64)
    )
}

fn is_signed(t: &MirType) -> bool {
    matches!(
        t,
        MirType::Primitive(
            PrimitiveType::Int8
                | PrimitiveType::Int16
                | PrimitiveType::Int32
                | PrimitiveType::Int64
        )
    )
}

fn mir_align(t: &MirType) -> u32 {
    match t {
        MirType::Primitive(p) => match p {
            PrimitiveType::Int8 | PrimitiveType::Uint8 | PrimitiveType::Bool => 1,
            PrimitiveType::Int16 | PrimitiveType::Uint16 => 2,
            PrimitiveType::Int32
            | PrimitiveType::Uint32
            | PrimitiveType::Float32
            | PrimitiveType::Char => 4,
            PrimitiveType::Int64
            | PrimitiveType::Uint64
            | PrimitiveType::Float64
            | PrimitiveType::String => 8,
        },
        _ => 8,
    }
}

fn mir_size(t: &MirType) -> u32 {
    mir_align(t)
}

fn align_up(off: u32, align: u32) -> u32 {
    if align == 0 {
        off
    } else {
        (off + align - 1) / align * align
    }
}
