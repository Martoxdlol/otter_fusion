use crate::hir::{ResolvedType, TypeParamId};
use std::collections::HashMap;

/// Tool for substituting type params with the concrete implementation
///
/// Example:
/// ```text
/// function foo<T>(x: T) -> T { x }
/// foo<int>(42)
/// ```
///
/// En este caso, el `T` dentro de la función `foo` se va a sustituir por `int` cuando monomorfizamos la llamada `foo<int>(42)`.
#[derive(Debug, Clone)]
pub struct Subst {
    mappings: HashMap<TypeParamId, ResolvedType>,
}

// Esto lo que hace es
impl Subst {
    pub fn new(params: Vec<TypeParamId>, args: Vec<ResolvedType>) -> Self {
        Self {
            mappings: params.into_iter().zip(args.into_iter()).collect(),
        }
    }

    /// Aplicar substitución. En la mayoría de los casos no se substituye nada.
    /// Pero en TypeParam, es el caso donde se busca en el map.
    pub fn apply(&self, ty: &ResolvedType) -> ResolvedType {
        use ResolvedType::*;
        match ty {
            Primitive(p) => Primitive(p.clone()),
            Null => Null,

            // Caso que nos importa
            TypeParam(id) => self.mappings.get(id).cloned().unwrap_or(TypeParam(*id)),

            Struct(id, args) => Struct(*id, args.iter().map(|a| self.apply(a)).collect()),
            Interface(id, args) => Interface(*id, args.iter().map(|a| self.apply(a)).collect()),
            Union(vs) => Union(vs.iter().map(|v| self.apply(v)).collect()),
            Function(args, ret) => Function(
                args.iter().map(|a| self.apply(a)).collect(),
                Box::new(self.apply(ret)),
            ),
        }
    }
}
