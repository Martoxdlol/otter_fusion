use crate::{ast::Program, hir::Hir};

pub struct Validator {
    program: Program,
}

impl Validator {
    pub fn new(program: Program) -> Self {
        Self { program }
    }

    pub fn validate(&self) -> Result<Hir, String> {
        todo!()
    }
}
