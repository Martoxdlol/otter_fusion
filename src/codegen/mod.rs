use std::collections::HashMap;

use cranelift_codegen::{
    ir::{self, AbiParam, Block, InstBuilder, Signature, TrapCode, Type, Value, types},
    isa::CallConv,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, FuncId, Linkage, Module, ModuleError};

use crate::{
    hir::PrimitiveType,
    mir::{
        self, Abi, BlockId, LocalId, MirConst, MirFnId, MirProgram, MirType, Operand, Terminator,
        TrapReason,
    },
};

pub struct Codegen<M: Module> {
    mir: MirProgram,
    module: M,
}

impl<M: Module> Codegen<M> {
    pub fn new(mir: MirProgram, module: M) -> Self {
        Self { mir, module }
    }

    pub fn compile(mut self) -> Result<M, ModuleError> {
        let mut function_ids = HashMap::new();

        for (id, f) in &self.mir.functions {
            let sig = self.build_signature(f);
            let linkage = if f.id == self.mir.entry {
                Linkage::Export
            } else {
                // TODO: expose extern functions with body???
                Linkage::Local
            };
            let fid = self.module.declare_function(&f.name, linkage, &sig)?;
            function_ids.insert(*id, fid);
        }

        // Context for building functions
        let mut ctx = self.module.make_context();
        let mut builder_ctx = FunctionBuilderContext::new();
        for (id, f) in &self.mir.functions {
            if f.abi == Abi::Extern && f.blocks.is_empty() {
                continue;
            } // import only

            ctx.func.signature = self.build_signature(f);
            self.lower_function(&mut ctx.func, &mut builder_ctx, &function_ids, f)?;
            self.module.define_function(function_ids[id], &mut ctx)?;
            self.module.clear_context(&mut ctx);
        }

        Ok(self.module)
    }

    fn build_signature(&self, f: &mir::MirFunction) -> Signature {
        let call_conv = match f.abi {
            Abi::Otter => self.module.target_config().default_call_conv,
            Abi::Extern => CallConv::triple_default(self.module.isa().triple()),
        };
        let mut sig = Signature::new(call_conv);
        for local_id in &f.params {
            let ty = Self::clif_type(&f.locals[local_id].ty);
            sig.params.push(AbiParam::new(ty));
        }
        if !matches!(f.return_type, MirType::Unit) {
            sig.returns
                .push(AbiParam::new(Self::clif_type(&f.return_type)));
        }
        sig
    }

    fn lower_function(
        &mut self,
        func: &mut ir::Function,
        builder_ctx: &mut FunctionBuilderContext,
        function_ids: &HashMap<mir::MirFnId, cranelift_module::FuncId>,
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
            b.declare_var(v, Codegen::<M>::clif_type(&loc.ty));
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
                self.lower_stmt(&mut b, function_ids, &vars, &blocks, mir_func, stmt);
            }
            self.lower_terminator(&mut b, &vars, &blocks, &blk.terminator);
        }

        b.seal_all_blocks();
        b.finalize();

        todo!()
    }

    fn lower_stmt(
        &self,
        b: &mut FunctionBuilder,
        func_ids: &HashMap<mir::MirFnId, cranelift_module::FuncId>,
        vars: &HashMap<LocalId, Variable>,
        blocks: &HashMap<BlockId, Block>,
        mir_func: &mir::MirFunction,
        stmt: &mir::Stmt,
    ) {
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

    pub fn lower_terminator(
        &mut self,
        b: &mut FunctionBuilder,
        vars: &HashMap<LocalId, Variable>,
        blocks: &HashMap<BlockId, Block>,
        t: &Terminator,
    ) {
        match t {
            Terminator::Goto(target) => {
                b.ins().jump(blocks[target], &[]);
            }
            Terminator::CondBr(cond, then_b, else_b) => {
                let c = self.use_operand(b, vars, cond);
                b.ins().brif(c, blocks[then_b], &[], blocks[else_b], &[]);
            }
            Terminator::Switch {
                scrutinee,
                arms,
                default,
            } => {
                let s = self.use_operand(b, vars, scrutinee);
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
                let v = self.use_operand(b, vars, op);
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

    fn use_operand(
        &mut self,
        b: &mut FunctionBuilder,
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
                MirConst::String(s) => self.emit_string_const(b, s),
                MirConst::Fn(id) => self.emit_func_addr(b, id),
            },
        }
    }

    fn emit_string_const(&mut self, b: &mut FunctionBuilder, s: &str) -> Value {
        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0); // null-terminate so the runtime can treat it as a C string too
        let mut desc = DataDescription::new();
        desc.define(bytes.into_boxed_slice());
        let did = self.module.declare_anonymous_data(false, false).unwrap();
        self.module.define_data(did, &desc).unwrap();
        let gv: ir::GlobalValue = self.module.declare_data_in_func(did, b.func);
        b.ins().symbol_value(types::I64, gv)
    }

    fn emit_func_addr(
        &mut self,
        b: &mut FunctionBuilder,
        func_ids: &HashMap<MirFnId, FuncId>,
        id: &MirFnId,
    ) -> Value {
        let func_ref = self.module.declare_func_in_func(func_ids[id], b.func);
        b.ins().func_addr(types::I64, func_ref)
    }
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
