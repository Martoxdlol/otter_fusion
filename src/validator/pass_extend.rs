use crate::ast;
use crate::hir::{self, ResolvedType};

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

            // Resolve the target type
            let target_type_id = match self.resolve_extend_target(module_id, &extend_decl.target) {
                Some(id) => id,
                None => continue,
            };

            // Allocate type params for the extend block
            let saved_scope = self.type_param_scope.clone();
            self.type_param_scope.clear();

            // First, bring the target struct's type params into scope
            let struct_tp_ids = self.hir.structs[&target_type_id].type_params.clone();
            for &tp_id in &struct_tp_ids {
                if let Some(tp_info) = self.hir.type_params.get(&tp_id) {
                    self.type_param_scope
                        .insert(tp_info.name.clone(), tp_id);
                }
            }

            // Then add the extend block's own type params
            let extend_tp_ids = self.allocate_type_params(&extend_decl.generic_params);
            for (i, gp) in extend_decl.generic_params.iter().enumerate() {
                self.type_param_scope.insert(gp.name.clone(), extend_tp_ids[i]);
            }

            // Resolve type param bounds for the extend's own generics
            self.resolve_type_param_bounds(module_id, &extend_decl.generic_params, &extend_tp_ids);

            // Register each method
            for method in &extend_decl.methods {
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
                        type_params: fn_tp_ids,
                        params: hir_params,
                        return_type,
                        body: None,
                    },
                );

                if let Some(body) = &method.body {
                    self.ast_fn_bodies.insert(fn_id, body.clone());
                }

                self.hir
                    .structs
                    .get_mut(&target_type_id)
                    .unwrap()
                    .methods
                    .push(fn_id);

                // Remove the function's own type params from scope
                for gp in &method.generics {
                    self.type_param_scope.remove(&gp.name);
                }
            }

            // Resolve any implements clauses on the extend
            for iface_expr in &extend_decl.implements {
                match iface_expr {
                    ast::TypeExpr::Named(name, _) => {
                        let visible = self.visible_names.get(&module_id).cloned().unwrap_or_default();
                        if let Some(&iface_id) = visible.types.get(name.as_str()) {
                            if self.type_kinds.get(&iface_id) == Some(&TypeKind::Interface) {
                                self.hir
                                    .structs
                                    .get_mut(&target_type_id)
                                    .unwrap()
                                    .implements
                                    .push(iface_id);
                            }
                        } else {
                            self.errors.push(ValidationError {
                                kind: ErrorKind::UndefinedType(name.clone()),
                                module: module_name.clone(),
                                context: Some("in extend implements clause".to_string()),
                            });
                        }
                    }
                    _ => {}
                }
            }

            self.type_param_scope = saved_scope;
        }

        self.pending_extends = extends;
    }

    fn resolve_extend_target(
        &mut self,
        module_id: hir::ModuleId,
        target: &ast::TypeExpr,
    ) -> Option<hir::TypeId> {
        let module_name = self.module_name(module_id);
        match target {
            ast::TypeExpr::Named(name, _) => {
                let visible = self.visible_names.get(&module_id)?;
                if let Some(&type_id) = visible.types.get(name.as_str()) {
                    if self.type_kinds.get(&type_id) == Some(&TypeKind::Struct) {
                        Some(type_id)
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
