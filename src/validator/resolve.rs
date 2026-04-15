use crate::ast;
use crate::hir::{self, ModuleId, ResolvedType, TypeId};

use super::errors::{ErrorKind, ValidationError};
use super::scope::TypeKind;
use super::Validator;

impl Validator {
    /// Convert an AST TypeExpr into a resolved HIR ResolvedType
    pub(crate) fn resolve_type(
        &mut self,
        module_id: ModuleId,
        type_expr: &ast::TypeExpr,
    ) -> Result<ResolvedType, ValidationError> {
        let module_name = self.module_name(module_id);
        match type_expr {
            ast::TypeExpr::Primitive(p) => Ok(convert_primitive(p)),

            ast::TypeExpr::Named(name, args) => {
                // Check type param scope first
                if let Some(&tp_id) = self.type_param_scope.get(name.as_str()) {
                    if !args.is_empty() {
                        return Err(ValidationError {
                            kind: ErrorKind::WrongTypeArgCount {
                                expected: 0,
                                found: args.len(),
                            },
                            module: module_name,
                            context: Some(format!("type parameter '{name}'")),
                        });
                    }
                    return Ok(ResolvedType::TypeParam(tp_id));
                }

                // Look up in visible names
                let visible = self.visible_names.get(&module_id).cloned().unwrap_or_default();
                let type_id = visible.types.get(name.as_str()).copied().ok_or_else(|| {
                    ValidationError {
                        kind: ErrorKind::UndefinedType(name.clone()),
                        module: module_name.clone(),
                        context: None,
                    }
                })?;

                // Resolve type arguments
                let resolved_args = args
                    .iter()
                    .map(|a| self.resolve_type(module_id, a))
                    .collect::<Result<Vec<_>, _>>()?;

                // Check what kind of type this is
                let kind = self.type_kinds.get(&type_id).copied();
                match kind {
                    Some(TypeKind::Alias) => self.expand_type_alias(module_id, type_id, &resolved_args),
                    Some(TypeKind::Struct) => Ok(ResolvedType::Struct(type_id, resolved_args)),
                    Some(TypeKind::Interface) => Ok(ResolvedType::Interface(type_id, resolved_args)),
                    None => Err(ValidationError {
                        kind: ErrorKind::UndefinedType(name.clone()),
                        module: module_name,
                        context: None,
                    }),
                }
            }

            ast::TypeExpr::Union(variants) => {
                let mut resolved = Vec::new();
                for v in variants {
                    let r = self.resolve_type(module_id, v)?;
                    // Flatten nested unions
                    match r {
                        ResolvedType::Union(inner) => resolved.extend(inner),
                        other => resolved.push(other),
                    }
                }
                // Deduplicate
                let mut deduped: Vec<ResolvedType> = Vec::new();
                for r in resolved {
                    if !deduped.contains(&r) {
                        deduped.push(r);
                    }
                }
                if deduped.len() == 1 {
                    Ok(deduped.into_iter().next().unwrap())
                } else {
                    Ok(ResolvedType::Union(deduped))
                }
            }

            ast::TypeExpr::Function(params, ret) => {
                let resolved_params = params
                    .iter()
                    .map(|p| self.resolve_type(module_id, p))
                    .collect::<Result<Vec<_>, _>>()?;
                let resolved_ret = self.resolve_type(module_id, ret)?;
                Ok(ResolvedType::Function(resolved_params, Box::new(resolved_ret)))
            }
        }
    }

    /// Expand a type alias, substituting type arguments for type parameters
    fn expand_type_alias(
        &mut self,
        module_id: ModuleId,
        alias_id: TypeId,
        args: &[ResolvedType],
    ) -> Result<ResolvedType, ValidationError> {
        let module_name = self.module_name(module_id);

        // Cycle detection: return a recursive marker instead of erroring
        if self.alias_expanding.contains(&alias_id) {
            return Ok(ResolvedType::Alias(alias_id, args.to_vec()));
        }

        let alias_info = self.type_aliases.get(&alias_id).unwrap().clone();

        // Check arity
        if args.len() != alias_info.generic_names.len() {
            return Err(ValidationError {
                kind: ErrorKind::WrongTypeArgCount {
                    expected: alias_info.generic_names.len(),
                    found: args.len(),
                },
                module: module_name,
                context: Some(format!("type alias '{}'", alias_info.name)),
            });
        }

        // Set up type param scope for alias expansion
        let saved_tp_scope = self.type_param_scope.clone();
        for (i, name) in alias_info.generic_names.iter().enumerate() {
            self.type_param_scope.insert(name.clone(), alias_info.type_param_ids[i]);
        }

        self.alias_expanding.insert(alias_id);
        let result = self.resolve_type(module_id, &alias_info.body);
        self.alias_expanding.remove(&alias_id);

        self.type_param_scope = saved_tp_scope;

        match result {
            Ok(resolved_body) => {
                // Cache the pre-substitution body (with TypeParam IDs) for unfold_alias_readonly
                if self.contains_alias_ref(&resolved_body, alias_id)
                    && !self.resolved_alias_bodies.contains_key(&alias_id)
                {
                    self.resolved_alias_bodies.insert(alias_id, resolved_body.clone());
                }

                // Substitute type params with concrete args
                let resolved = substitute_type_params(&resolved_body, &alias_info.type_param_ids, args);

                // Strip bare self-references from top-level unions
                let resolved = self.strip_bare_self_refs(alias_id, resolved);

                // Validate recursive aliases have a base case
                if self.contains_alias_ref(&resolved, alias_id) {
                    if !self.has_non_recursive_base(&resolved, alias_id) {
                        return Err(ValidationError {
                            kind: ErrorKind::RecursiveAliasNoBaseCase(alias_info.name.clone()),
                            module: module_name,
                            context: None,
                        });
                    }
                }

                Ok(resolved)
            }
            Err(e) => Err(e),
        }
    }

    /// Strip bare self-references from a top-level union.
    /// `type Json = i64 | Json | null` -> the bare `| Json |` is redundant.
    fn strip_bare_self_refs(&self, alias_id: TypeId, ty: ResolvedType) -> ResolvedType {
        match ty {
            ResolvedType::Union(variants) => {
                let filtered: Vec<_> = variants
                    .into_iter()
                    .filter(|v| !matches!(v, ResolvedType::Alias(id, _) if *id == alias_id))
                    .collect();
                if filtered.len() == 1 {
                    filtered.into_iter().next().unwrap()
                } else if filtered.is_empty() {
                    // Degenerate: all variants were self-refs
                    ResolvedType::Alias(alias_id, vec![])
                } else {
                    ResolvedType::Union(filtered)
                }
            }
            _ => ty,
        }
    }

    /// Check if a resolved type contains a reference to the given alias
    fn contains_alias_ref(&self, ty: &ResolvedType, alias_id: TypeId) -> bool {
        match ty {
            ResolvedType::Alias(id, _) => *id == alias_id,
            ResolvedType::Union(variants) => variants.iter().any(|v| self.contains_alias_ref(v, alias_id)),
            ResolvedType::Struct(_, args) | ResolvedType::Interface(_, args) => {
                args.iter().any(|a| self.contains_alias_ref(a, alias_id))
            }
            ResolvedType::Function(params, ret) => {
                params.iter().any(|p| self.contains_alias_ref(p, alias_id))
                    || self.contains_alias_ref(ret, alias_id)
            }
            _ => false,
        }
    }

    /// Check that a recursive type has at least one non-recursive variant (a base case)
    fn has_non_recursive_base(&self, ty: &ResolvedType, alias_id: TypeId) -> bool {
        match ty {
            ResolvedType::Union(variants) => {
                variants.iter().any(|v| !self.contains_alias_ref(v, alias_id))
            }
            // Non-union body that references itself has no base case
            _ => !self.contains_alias_ref(ty, alias_id),
        }
    }

    /// Unfold a recursive alias one level: resolve the body with self-references
    /// remaining as Alias markers.
    pub(crate) fn unfold_alias(
        &mut self,
        module_id: ModuleId,
        alias_id: TypeId,
        args: &[ResolvedType],
    ) -> Result<ResolvedType, ValidationError> {
        let alias_info = self.type_aliases.get(&alias_id).unwrap().clone();

        let saved_tp_scope = self.type_param_scope.clone();
        for (i, name) in alias_info.generic_names.iter().enumerate() {
            self.type_param_scope
                .insert(name.clone(), alias_info.type_param_ids[i]);
        }

        self.alias_expanding.insert(alias_id);
        let result = self.resolve_type(module_id, &alias_info.body);
        self.alias_expanding.remove(&alias_id);

        self.type_param_scope = saved_tp_scope;

        match result {
            Ok(resolved) => {
                let resolved =
                    substitute_type_params(&resolved, &alias_info.type_param_ids, args);
                Ok(self.strip_bare_self_refs(alias_id, resolved))
            }
            Err(e) => Err(e),
        }
    }

    /// Check if two types are compatible
    pub(crate) fn types_compatible(&self, expected: &ResolvedType, actual: &ResolvedType) -> bool {
        self.types_compatible_depth(expected, actual, 0)
    }

    fn types_compatible_depth(
        &self,
        expected: &ResolvedType,
        actual: &ResolvedType,
        depth: u32,
    ) -> bool {
        const MAX_DEPTH: u32 = 64;
        if depth > MAX_DEPTH {
            return false;
        }
        if expected == actual {
            return true;
        }
        match (expected, actual) {
            // Alias vs Alias: same ID, compare args
            (ResolvedType::Alias(id1, args1), ResolvedType::Alias(id2, args2)) if id1 == id2 => {
                args1.len() == args2.len()
                    && args1
                        .iter()
                        .zip(args2)
                        .all(|(a, b)| self.types_compatible_depth(a, b, depth + 1))
            }
            // Alias on expected side: unfold one level
            (ResolvedType::Alias(id, args), _) => {
                if let Some(unfolded) = self.unfold_alias_readonly(*id, args) {
                    self.types_compatible_depth(&unfolded, actual, depth + 1)
                } else {
                    false
                }
            }
            // Alias on actual side: unfold one level
            (_, ResolvedType::Alias(id, args)) => {
                if let Some(unfolded) = self.unfold_alias_readonly(*id, args) {
                    self.types_compatible_depth(expected, &unfolded, depth + 1)
                } else {
                    false
                }
            }
            // Null is compatible with unions containing Null
            (ResolvedType::Union(variants), ResolvedType::Null) => {
                variants.iter().any(|v| matches!(v, ResolvedType::Null))
            }
            // Both unions: every actual variant must fit somewhere in the expected union
            (ResolvedType::Union(_), ResolvedType::Union(actual_variants)) => {
                actual_variants.iter().all(|v| self.types_compatible_depth(expected, v, depth + 1))
            }
            // A concrete type is compatible with a union if it's one of the variants
            (ResolvedType::Union(variants), actual) => {
                variants.iter().any(|v| self.types_compatible_depth(v, actual, depth + 1))
            }
            // An actual union is compatible if all its variants are in the expected
            (expected, ResolvedType::Union(actual_variants)) => {
                actual_variants.iter().all(|v| self.types_compatible_depth(expected, v, depth + 1))
            }
            // Struct with matching IDs and compatible args
            (ResolvedType::Struct(id1, args1), ResolvedType::Struct(id2, args2)) => {
                id1 == id2
                    && args1.len() == args2.len()
                    && args1.iter().zip(args2).all(|(a, b)| self.types_compatible_depth(a, b, depth + 1))
            }
            // Interface with matching IDs and compatible args
            (ResolvedType::Interface(id1, args1), ResolvedType::Interface(id2, args2)) => {
                id1 == id2
                    && args1.len() == args2.len()
                    && args1.iter().zip(args2).all(|(a, b)| self.types_compatible_depth(a, b, depth + 1))
            }
            // Function types
            (ResolvedType::Function(p1, r1), ResolvedType::Function(p2, r2)) => {
                p1.len() == p2.len()
                    && p1.iter().zip(p2).all(|(a, b)| self.types_compatible_depth(a, b, depth + 1))
                    && self.types_compatible_depth(r1, r2, depth + 1)
            }
            _ => false,
        }
    }

    /// Unfold an alias one level without needing &mut self (for use in types_compatible).
    /// Performs substitution inline using the stored alias body.
    fn unfold_alias_readonly(&self, alias_id: TypeId, args: &[ResolvedType]) -> Option<ResolvedType> {
        let alias_info = self.type_aliases.get(&alias_id)?;
        // We need to do a lightweight expansion: just substitute type params in the
        // already-resolved body pattern. Since the body is an AST TypeExpr, we can't
        // resolve it here without &mut self. Instead, we store the resolved body
        // at expansion time. For now, use a simpler approach: walk the AST body
        // and produce a ResolvedType with Alias markers for self-references.
        // This is done by looking at what expand_type_alias already resolved.
        //
        // Actually, we store enough info: the alias body as AST, the param IDs.
        // We can't call resolve_type here (needs &mut). So we use a pre-cached
        // resolved body instead. Let's check if we have one.
        self.resolved_alias_bodies.get(&alias_id).map(|body| {
            let substituted = substitute_type_params(body, &alias_info.type_param_ids, args);
            self.strip_bare_self_refs_readonly(alias_id, substituted)
        })
    }

    fn strip_bare_self_refs_readonly(&self, alias_id: TypeId, ty: ResolvedType) -> ResolvedType {
        match ty {
            ResolvedType::Union(variants) => {
                let filtered: Vec<_> = variants
                    .into_iter()
                    .filter(|v| !matches!(v, ResolvedType::Alias(id, _) if *id == alias_id))
                    .collect();
                if filtered.len() == 1 {
                    filtered.into_iter().next().unwrap()
                } else if filtered.is_empty() {
                    ResolvedType::Alias(alias_id, vec![])
                } else {
                    ResolvedType::Union(filtered)
                }
            }
            _ => ty,
        }
    }

    /// Format a ResolvedType for error messages
    pub(crate) fn format_type(&self, ty: &ResolvedType) -> String {
        match ty {
            ResolvedType::Primitive(p) => format_primitive(p),
            ResolvedType::Struct(id, args) => {
                let name = self
                    .hir
                    .structs
                    .get(id)
                    .map(|s| s.name.as_str())
                    .unwrap_or("?");
                if args.is_empty() {
                    name.to_string()
                } else {
                    let args_str: Vec<String> = args.iter().map(|a| self.format_type(a)).collect();
                    format!("{name}<{}>", args_str.join(", "))
                }
            }
            ResolvedType::Interface(id, args) => {
                let name = self
                    .hir
                    .interfaces
                    .get(id)
                    .map(|i| i.name.as_str())
                    .unwrap_or("?");
                if args.is_empty() {
                    name.to_string()
                } else {
                    let args_str: Vec<String> = args.iter().map(|a| self.format_type(a)).collect();
                    format!("{name}<{}>", args_str.join(", "))
                }
            }
            ResolvedType::Union(variants) => {
                let parts: Vec<String> = variants.iter().map(|v| self.format_type(v)).collect();
                parts.join(" | ")
            }
            ResolvedType::Function(params, ret) => {
                let params_str: Vec<String> = params.iter().map(|p| self.format_type(p)).collect();
                format!("({}) -> {}", params_str.join(", "), self.format_type(ret))
            }
            ResolvedType::TypeParam(id) => self
                .hir
                .type_params
                .get(id)
                .map(|tp| tp.name.clone())
                .unwrap_or_else(|| "?".to_string()),
            ResolvedType::Alias(id, args) => {
                let name = self
                    .type_aliases
                    .get(id)
                    .map(|a| a.name.as_str())
                    .unwrap_or("?");
                if args.is_empty() {
                    name.to_string()
                } else {
                    let args_str: Vec<String> = args.iter().map(|a| self.format_type(a)).collect();
                    format!("{name}<{}>", args_str.join(", "))
                }
            }
            ResolvedType::Null => "null".to_string(),
        }
    }

    pub(crate) fn module_name(&self, module_id: ModuleId) -> String {
        self.hir
            .modules
            .get(&module_id)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "<unknown>".to_string())
    }
}

pub fn convert_primitive(p: &ast::PrimitiveType) -> ResolvedType {
    match p {
        ast::PrimitiveType::Null => ResolvedType::Null,
        ast::PrimitiveType::Int8 => ResolvedType::Primitive(hir::PrimitiveType::Int8),
        ast::PrimitiveType::Int16 => ResolvedType::Primitive(hir::PrimitiveType::Int16),
        ast::PrimitiveType::Int32 => ResolvedType::Primitive(hir::PrimitiveType::Int32),
        ast::PrimitiveType::Int64 => ResolvedType::Primitive(hir::PrimitiveType::Int64),
        ast::PrimitiveType::Uint8 => ResolvedType::Primitive(hir::PrimitiveType::Uint8),
        ast::PrimitiveType::Uint16 => ResolvedType::Primitive(hir::PrimitiveType::Uint16),
        ast::PrimitiveType::Uint32 => ResolvedType::Primitive(hir::PrimitiveType::Uint32),
        ast::PrimitiveType::Uint64 => ResolvedType::Primitive(hir::PrimitiveType::Uint64),
        ast::PrimitiveType::Float32 => ResolvedType::Primitive(hir::PrimitiveType::Float32),
        ast::PrimitiveType::Float64 => ResolvedType::Primitive(hir::PrimitiveType::Float64),
        ast::PrimitiveType::Bool => ResolvedType::Primitive(hir::PrimitiveType::Bool),
        ast::PrimitiveType::String => ResolvedType::Primitive(hir::PrimitiveType::String),
        ast::PrimitiveType::Char => ResolvedType::Primitive(hir::PrimitiveType::Char),
    }
}

fn format_primitive(p: &hir::PrimitiveType) -> String {
    match p {
        hir::PrimitiveType::Int8 => "i8".to_string(),
        hir::PrimitiveType::Int16 => "i16".to_string(),
        hir::PrimitiveType::Int32 => "i32".to_string(),
        hir::PrimitiveType::Int64 => "i64".to_string(),
        hir::PrimitiveType::Uint8 => "u8".to_string(),
        hir::PrimitiveType::Uint16 => "u16".to_string(),
        hir::PrimitiveType::Uint32 => "u32".to_string(),
        hir::PrimitiveType::Uint64 => "u64".to_string(),
        hir::PrimitiveType::Float32 => "f32".to_string(),
        hir::PrimitiveType::Float64 => "f64".to_string(),
        hir::PrimitiveType::Bool => "bool".to_string(),
        hir::PrimitiveType::String => "str".to_string(),
        hir::PrimitiveType::Char => "char".to_string(),
    }
}

/// Substitute type parameters in a resolved type with concrete types
pub(crate) fn substitute_type_params(
    ty: &ResolvedType,
    param_ids: &[hir::TypeParamId],
    args: &[ResolvedType],
) -> ResolvedType {
    match ty {
        ResolvedType::TypeParam(id) => {
            if let Some(pos) = param_ids.iter().position(|pid| pid == id) {
                args[pos].clone()
            } else {
                ty.clone()
            }
        }
        ResolvedType::Struct(id, type_args) => {
            let substituted: Vec<_> = type_args
                .iter()
                .map(|a| substitute_type_params(a, param_ids, args))
                .collect();
            ResolvedType::Struct(*id, substituted)
        }
        ResolvedType::Interface(id, type_args) => {
            let substituted: Vec<_> = type_args
                .iter()
                .map(|a| substitute_type_params(a, param_ids, args))
                .collect();
            ResolvedType::Interface(*id, substituted)
        }
        ResolvedType::Union(variants) => {
            let substituted: Vec<_> = variants
                .iter()
                .map(|v| substitute_type_params(v, param_ids, args))
                .collect();
            ResolvedType::Union(substituted)
        }
        ResolvedType::Function(params, ret) => {
            let substituted_params: Vec<_> = params
                .iter()
                .map(|p| substitute_type_params(p, param_ids, args))
                .collect();
            let substituted_ret = substitute_type_params(ret, param_ids, args);
            ResolvedType::Function(substituted_params, Box::new(substituted_ret))
        }
        ResolvedType::Alias(id, type_args) => {
            let substituted: Vec<_> = type_args
                .iter()
                .map(|a| substitute_type_params(a, param_ids, args))
                .collect();
            ResolvedType::Alias(*id, substituted)
        }
        ResolvedType::Primitive(_) | ResolvedType::Null => ty.clone(),
    }
}
