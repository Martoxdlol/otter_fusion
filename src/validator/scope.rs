use std::collections::HashMap;

use crate::hir::{FnId, ResolvedType, TypeId, TypeParamId};

/// Module-level registry of type and function names
#[derive(Debug, Default)]
pub struct ModuleScope {
    pub types: HashMap<String, TypeId>,
    pub functions: HashMap<String, FnId>,
}

/// Names visible in a module (own definitions + imports)
#[derive(Debug, Default, Clone)]
pub struct VisibleNames {
    pub types: HashMap<String, TypeId>,
    pub functions: HashMap<String, FnId>,
}

/// Distinguishes what a TypeId refers to
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeKind {
    Struct,
    Interface,
    Alias,
}

/// Storage for a type alias definition
#[derive(Debug, Clone)]
pub struct TypeAliasInfo {
    pub name: String,
    pub generic_names: Vec<String>,
    pub type_param_ids: Vec<TypeParamId>,
    pub body: crate::ast::TypeExpr,
}

/// Local variable scope with nested frames (for blocks, loops, etc.)
#[derive(Debug)]
pub struct LocalScope {
    frames: Vec<HashMap<String, ResolvedType>>,
    loop_depth: u32,
}

impl LocalScope {
    pub fn new() -> Self {
        Self {
            frames: vec![HashMap::new()],
            loop_depth: 0,
        }
    }

    pub fn push(&mut self) {
        self.frames.push(HashMap::new());
    }

    pub fn push_loop(&mut self) {
        self.frames.push(HashMap::new());
        self.loop_depth += 1;
    }

    pub fn pop(&mut self) {
        self.frames.pop();
    }

    pub fn pop_loop(&mut self) {
        self.frames.pop();
        self.loop_depth -= 1;
    }

    pub fn define(&mut self, name: String, ty: ResolvedType) {
        let frame = self.frames.last_mut().unwrap();
        frame.insert(name, ty);
    }

    pub fn lookup(&self, name: &str) -> Option<&ResolvedType> {
        for frame in self.frames.iter().rev() {
            if let Some(ty) = frame.get(name) {
                return Some(ty);
            }
        }
        None
    }

    pub fn is_in_loop(&self) -> bool {
        self.loop_depth > 0
    }
}
