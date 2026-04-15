use crate::ast;
use crate::hir::{self, ResolvedType, SpecializedExtend};

use super::errors::{ErrorKind, ValidationError};
use super::scope::TypeKind;
use super::Validator;

impl Validator {
    /// Pass 3: Merge extend blocks into their target structs
    pub(crate) fn pass_merge_extends(&mut self) {
        let extends = std::mem::take(&mut self.pending_extends);

        for (module_id, extend_decl) in &extends {
            let module_id = *module_id;
            let module_name = self.module_name(module_id);

            // Resolve the target struct ID and its type args
            let (target_type_id, target_type_args) =
                match self.resolve_extend_target_with_args(module_id, &extend_decl.target) {
                    Some(v) => v,
                    None => continue,
                };

            let struct_tp_ids = self.hir.structs[&target_type_id].type_params.clone();

            // Validate arity: target type args must match struct's type param count
            if !target_type_args.is_empty() && target_type_args.len() != struct_tp_ids.len() {
                self.errors.push(ValidationError {
                    kind: ErrorKind::SpecializationArityMismatch {
                        type_name: self.hir.structs[&target_type_id].name.clone(),
                        expected: struct_tp_ids.len(),
                        found: target_type_args.len(),
                    },
                    module: module_name.clone(),
                    context: None,
                });
                continue;
            }

            // Classify: is this a generic or specialized extend?
            let is_specialized = self.is_specialized_extend(
                &extend_decl.generic_params,
                &target_type_args,
                &struct_tp_ids,
            );

            let saved_scope = self.type_param_scope.clone();
            self.type_param_scope.clear();

            if is_specialized {
                self.process_specialized_extend(
                    module_id,
                    extend_decl,
                    target_type_id,
                    &target_type_args,
                    &struct_tp_ids,
                    &module_name,
                );
            } else {
                self.process_generic_extend(
                    module_id,
                    extend_decl,
                    target_type_id,
                    &struct_tp_ids,
                    &module_name,
                );
            }

            self.type_param_scope = saved_scope;
        }

        self.pending_extends = extends;
    }

    /// Determine if an extend block is specialized.
    /// It's generic if: no target type args, OR every target type arg is a TypeParam
    /// that maps 1:1 to the extend's own generic params.
    fn is_specialized_extend(
        &self,
        extend_generics: &[ast::GenericParam],
        target_type_args: &[ast::TypeExpr],
        struct_tp_ids: &[hir::TypeParamId],
    ) -> bool {
        // No type args on target and struct has params → generic (e.g., `extend<T> X<T>`)
        // No type args on target and struct has no params → generic (plain `extend X`)
        if target_type_args.is_empty() {
            return false;
        }

        // If there are target type args, check if they're all just the extend's own
        // generic param names in 1:1 order
        if target_type_args.len() != struct_tp_ids.len() {
            return true; // arity mismatch, will error later
        }

        if extend_generics.len() != target_type_args.len() {
            return true; // different count → specialized or partial
        }

        // Check if target args are exactly the extend's generic param names in order
        for (i, arg) in target_type_args.iter().enumerate() {
            match arg {
                ast::TypeExpr::Named(name, inner_args) => {
                    if inner_args.is_empty()
                        && i < extend_generics.len()
                        && *name == extend_generics[i].name
                    {
                        continue; // This arg is the extend's own param
                    }
                    return true; // Concrete type or different param
                }
                ast::TypeExpr::Primitive(_) => return true, // Concrete primitive
                _ => return true,
            }
        }

        false // All args are 1:1 generic params → generic extend
    }

    /// Process a generic extend block (existing behavior).
    fn process_generic_extend(
        &mut self,
        module_id: hir::ModuleId,
        extend_decl: &ast::ExtendDecl,
        target_type_id: hir::TypeId,
        struct_tp_ids: &[hir::TypeParamId],
        module_name: &str,
    ) {
        // Bring the target struct's type params into scope
        for &tp_id in struct_tp_ids {
            if let Some(tp_info) = self.hir.type_params.get(&tp_id) {
                self.type_param_scope
                    .insert(tp_info.name.clone(), tp_id);
            }
        }

        // Add the extend block's own type params
        let extend_tp_ids = self.allocate_type_params(&extend_decl.generic_params);
        for (i, gp) in extend_decl.generic_params.iter().enumerate() {
            self.type_param_scope.insert(gp.name.clone(), extend_tp_ids[i]);
        }
        self.resolve_type_param_bounds(module_id, &extend_decl.generic_params, &extend_tp_ids);

        // Register methods
        for method in &extend_decl.methods {
            let fn_id = self.register_extend_method(
                module_id,
                method,
                target_type_id,
                module_name,
            );

            self.hir
                .structs
                .get_mut(&target_type_id)
                .unwrap()
                .methods
                .push(fn_id);
        }

        // Resolve implements clauses (generic)
        self.resolve_extend_implements(module_id, extend_decl, target_type_id, None, module_name);
    }

    /// Process a specialized extend block.
    fn process_specialized_extend(
        &mut self,
        module_id: hir::ModuleId,
        extend_decl: &ast::ExtendDecl,
        target_type_id: hir::TypeId,
        target_type_args: &[ast::TypeExpr],
        struct_tp_ids: &[hir::TypeParamId],
        module_name: &str,
    ) {
        // Allocate the extend block's own type params first (for partial specialization)
        let extend_tp_ids = self.allocate_type_params(&extend_decl.generic_params);
        for (i, gp) in extend_decl.generic_params.iter().enumerate() {
            self.type_param_scope.insert(gp.name.clone(), extend_tp_ids[i]);
        }
        self.resolve_type_param_bounds(module_id, &extend_decl.generic_params, &extend_tp_ids);

        // Resolve the target's type args (e.g., i64, str, T)
        let mut resolved_args = Vec::new();
        for arg in target_type_args {
            match self.resolve_type(module_id, arg) {
                Ok(resolved) => resolved_args.push(resolved),
                Err(e) => {
                    self.errors.push(e);
                    return;
                }
            }
        }

        // For specialized extends, bind struct type params to the concrete args
        // in the type_param_scope so method signatures resolve correctly.
        // E.g., for `extend X<i64>` on `struct X<T>`, T resolves to i64 in method bodies.
        for (i, &tp_id) in struct_tp_ids.iter().enumerate() {
            if let Some(tp_info) = self.hir.type_params.get(&tp_id) {
                // Only put the param name in scope if the arg is a TypeParam
                // (partial specialization). For concrete args, DON'T put the
                // struct's original param name in scope — the concrete type
                // will be used via substitution.
                if let ResolvedType::TypeParam(_) = &resolved_args[i] {
                    self.type_param_scope
                        .insert(tp_info.name.clone(), tp_id);
                }
                // If it's a concrete type, we still need the struct's param
                // in scope so that method signatures using T resolve correctly
                // during type-checking. But we want them to resolve to the
                // concrete type. We handle this by putting the param name in
                // scope but NOT mapping it to the original TypeParamId.
                // Instead, we skip it — the method signatures will be resolved
                // with the struct params NOT in scope for concrete args, so
                // references to T in method bodies will fail.
                // Actually, that's wrong. We need a different approach:
                // put all struct params in scope, then after resolving method
                // signatures, substitute them with the concrete args.
            }
        }

        // Actually: bring ALL struct type params into scope for resolution,
        // then substitute the concrete args into the resolved signatures.
        // This matches how generic extends work, but we substitute afterward.
        self.type_param_scope.clear();

        // Re-add extend's own params
        for (i, gp) in extend_decl.generic_params.iter().enumerate() {
            self.type_param_scope.insert(gp.name.clone(), extend_tp_ids[i]);
        }

        // Add struct params into scope
        for &tp_id in struct_tp_ids {
            if let Some(tp_info) = self.hir.type_params.get(&tp_id) {
                self.type_param_scope
                    .insert(tp_info.name.clone(), tp_id);
            }
        }

        // Register methods (resolved with struct params as TypeParam)
        let mut spec_method_ids = Vec::new();
        for method in &extend_decl.methods {
            let fn_id = self.register_extend_method(
                module_id,
                method,
                target_type_id,
                module_name,
            );

            // Substitute struct type params with the concrete specialization args
            // in the method's parameter types and return type.
            if let Some(hir_fn) = self.hir.functions.get_mut(&fn_id) {
                for param in &mut hir_fn.params {
                    param.ty = super::resolve::substitute_type_params(
                        &param.ty,
                        struct_tp_ids,
                        &resolved_args,
                    );
                }
                hir_fn.return_type = super::resolve::substitute_type_params(
                    &hir_fn.return_type,
                    struct_tp_ids,
                    &resolved_args,
                );
            }

            // Check for duplicate specialized methods
            let existing_specs = &self.hir.structs[&target_type_id].specialized_methods;
            let has_dup = existing_specs.iter().any(|s| {
                s.type_args == resolved_args
                    && s.methods.iter().any(|&mid| {
                        self.hir.functions.get(&mid).map(|f| f.name.as_str())
                            == self.hir.functions.get(&fn_id).map(|f| f.name.as_str())
                    })
            });
            if has_dup {
                let method_name = self.hir.functions.get(&fn_id)
                    .map(|f| f.name.clone())
                    .unwrap_or_default();
                let args_str: Vec<String> = resolved_args.iter().map(|a| self.format_type(a)).collect();
                self.errors.push(ValidationError {
                    kind: ErrorKind::DuplicateSpecializedMethod {
                        name: method_name,
                        type_args: args_str.join(", "),
                    },
                    module: module_name.to_string(),
                    context: None,
                });
            }

            spec_method_ids.push(fn_id);
        }

        // Store in the specialized_methods table
        // Find or create the SpecializedExtend entry for these type_args
        let hir_struct = self.hir.structs.get_mut(&target_type_id).unwrap();
        if let Some(existing) = hir_struct
            .specialized_methods
            .iter_mut()
            .find(|s| s.type_args == resolved_args)
        {
            existing.methods.extend(spec_method_ids);
        } else {
            hir_struct.specialized_methods.push(SpecializedExtend {
                type_args: resolved_args.clone(),
                methods: spec_method_ids,
            });
        }

        // Resolve implements clauses (specialized)
        self.resolve_extend_implements(
            module_id,
            extend_decl,
            target_type_id,
            Some(&resolved_args),
            module_name,
        );
    }

    /// Register a single extend method and return its FnId.
    fn register_extend_method(
        &mut self,
        module_id: hir::ModuleId,
        method: &ast::FunctionDecl,
        target_type_id: hir::TypeId,
        _module_name: &str,
    ) -> hir::FnId {
        let fn_id = self.next_fn_id();

        // Allocate the function's own type params
        let fn_tp_ids = self.allocate_type_params(&method.generics);
        for (i, gp) in method.generics.iter().enumerate() {
            self.type_param_scope.insert(gp.name.clone(), fn_tp_ids[i]);
        }
        self.resolve_type_param_bounds(module_id, &method.generics, &fn_tp_ids);

        // Resolve parameter types
        let mut hir_params = Vec::new();
        for param in &method.params {
            match self.resolve_type(module_id, &param.ty) {
                Ok(resolved) => {
                    hir_params.push(hir::HirParam {
                        name: param.name.clone(),
                        ty: resolved,
                    });
                }
                Err(e) => self.errors.push(e),
            }
        }

        // Resolve return type
        let return_type = if let Some(ret_expr) = &method.return_type {
            match self.resolve_type(module_id, ret_expr) {
                Ok(resolved) => resolved,
                Err(e) => {
                    self.errors.push(e);
                    ResolvedType::Null
                }
            }
        } else {
            ResolvedType::Null
        };

        self.hir.functions.insert(
            fn_id,
            hir::HirFunction {
                id: fn_id,
                module: module_id,
                name: method.name.clone(),
                owner: Some(target_type_id),
                has_self: method.has_self_param,
                type_params: fn_tp_ids.clone(),
                params: hir_params,
                return_type,
                body: None,
            },
        );

        if let Some(body) = &method.body {
            self.ast_fn_bodies.insert(fn_id, body.clone());
        }

        // Remove the function's own type params from scope
        for gp in &method.generics {
            self.type_param_scope.remove(&gp.name);
        }

        fn_id
    }

    /// Resolve implements clauses on an extend block.
    /// If `specialization_args` is Some, store as specialized_implements.
    fn resolve_extend_implements(
        &mut self,
        module_id: hir::ModuleId,
        extend_decl: &ast::ExtendDecl,
        target_type_id: hir::TypeId,
        specialization_args: Option<&Vec<ResolvedType>>,
        module_name: &str,
    ) {
        for iface_expr in &extend_decl.implements {
            match iface_expr {
                ast::TypeExpr::Named(name, _) => {
                    let visible = self.visible_names.get(&module_id).cloned().unwrap_or_default();
                    if let Some(&iface_id) = visible.types.get(name.as_str()) {
                        if self.type_kinds.get(&iface_id) == Some(&TypeKind::Interface) {
                            let hir_struct =
                                self.hir.structs.get_mut(&target_type_id).unwrap();
                            if let Some(args) = specialization_args {
                                hir_struct
                                    .specialized_implements
                                    .push((args.clone(), iface_id));
                            } else {
                                hir_struct.implements.push(iface_id);
                            }
                        }
                    } else {
                        self.errors.push(ValidationError {
                            kind: ErrorKind::UndefinedType(name.clone()),
                            module: module_name.to_string(),
                            context: Some("in extend implements clause".to_string()),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    /// Resolve the extend target, returning both the TypeId and the raw type args.
    fn resolve_extend_target_with_args(
        &mut self,
        module_id: hir::ModuleId,
        target: &ast::TypeExpr,
    ) -> Option<(hir::TypeId, Vec<ast::TypeExpr>)> {
        let module_name = self.module_name(module_id);
        match target {
            ast::TypeExpr::Named(name, type_args) => {
                let visible = self.visible_names.get(&module_id)?;
                if let Some(&type_id) = visible.types.get(name.as_str()) {
                    if self.type_kinds.get(&type_id) == Some(&TypeKind::Struct) {
                        Some((type_id, type_args.clone()))
                    } else {
                        self.errors.push(ValidationError {
                            kind: ErrorKind::ExtendTargetNotStruct(name.clone()),
                            module: module_name,
                            context: None,
                        });
                        None
                    }
                } else {
                    self.errors.push(ValidationError {
                        kind: ErrorKind::UndefinedType(name.clone()),
                        module: module_name,
                        context: Some("in extend target".to_string()),
                    });
                    None
                }
            }
            _ => {
                self.errors.push(ValidationError {
                    kind: ErrorKind::ExtendTargetNotStruct(format!("{target:?}")),
                    module: module_name,
                    context: None,
                });
                None
            }
        }
    }
}
