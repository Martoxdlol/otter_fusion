use std::collections::HashMap;

use cranelift_codegen::{
    ir::{self, AbiParam, Signature, Type, types},
    isa::CallConv,
};
use cranelift_frontend::FunctionBuilderContext;
use cranelift_module::{Linkage, Module, ModuleError};

use crate::{
    hir::PrimitiveType,
    mir::{self, Abi, MirProgram, MirType},
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
            self.lower_function(&mut ctx.func, &mut builder_ctx, f)?;
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
        &self,
        func: &mut ir::Function,
        builder_ctx: &mut FunctionBuilderContext,
        mir_func: &mir::MirFunction,
    ) -> Result<(), ModuleError> {
        todo!()
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
}
