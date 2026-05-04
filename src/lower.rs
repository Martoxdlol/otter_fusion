use crate::{
    hir::{FnId, Hir, PrimitiveType, ResolvedType},
    mir::MirProgram,
};

pub struct Lower {
    hir: Hir,
    mir: MirProgram,
}

impl Lower {
    pub fn new(hir: Hir) -> Self {
        let mir = MirProgram::new();
        Self { hir, mir }
    }
}

impl Lower {
    pub fn lower(mut self) -> Result<MirProgram, String> {
        Ok(self.mir)
    }
}
