use crate::hir::{self, FnId, HirField, TypeId};

use super::errors::{ErrorKind, ValidationError};
use super::Validator;

impl Validator {
    /// Pass 4: Validate that structs properly implement their declared interfaces
    pub(crate) fn pass_validate_interfaces(&mut self) {
        let struct_ids: Vec<TypeId> = self.hir.structs.keys().copied().collect();

        for struct_id in struct_ids {
            let hir_struct = self.hir.structs[&struct_id].clone();

            // Skip built-in types
            if hir_struct.module == hir::ModuleId(u32::MAX) {
                continue;
            }

            let module_name = self.module_name(hir_struct.module);

            // Validate generic implements
            for &iface_id in &hir_struct.implements {
                self.validate_interface_impl(
                    iface_id,
                    &hir_struct.fields,
                    &hir_struct.methods,
                    &hir_struct.name,
                    &module_name,
                );
            }

            // Validate specialized implements
            for (spec_args, iface_id) in &hir_struct.specialized_implements {
                // Collect methods from the matching specialization
                let mut spec_method_ids: Vec<hir::FnId> = Vec::new();
                for spec in &hir_struct.specialized_methods {
                    if spec.type_args == *spec_args {
                        spec_method_ids.extend(&spec.methods);
                    }
                }
                // Also include generic methods (they apply to all specializations)
                spec_method_ids.extend(&hir_struct.methods);

                self.validate_interface_impl(
                    *iface_id,
                    &hir_struct.fields,
                    &spec_method_ids,
                    &hir_struct.name,
                    &module_name,
                );
            }
        }
    }

    /// Validate that a set of fields + methods satisfies an interface's requirements.
    fn validate_interface_impl(
        &mut self,
        iface_id: TypeId,
        struct_fields: &[HirField],
        method_ids: &[FnId],
        struct_name: &str,
        module_name: &str,
    ) {
        let (required_fields, required_method_ids) =
            self.collect_interface_requirements(iface_id);

        // Check fields
        for req_field in &required_fields {
            let found = struct_fields.iter().find(|f| f.name == req_field.name);
            match found {
                None => {
                    let iface_name = self
                        .hir
                        .interfaces
                        .get(&iface_id)
                        .map(|i| i.name.as_str())
                        .unwrap_or("?");
                    self.errors.push(ValidationError {
                        kind: ErrorKind::MissingInterfaceField {
                            iface: iface_name.to_string(),
                            field: req_field.name.clone(),
                        },
                        module: module_name.to_string(),
                        context: Some(format!("in struct '{struct_name}'")),
                    });
                }
                Some(actual_field) => {
                    if !self.types_compatible(&req_field.ty, &actual_field.ty) {
                        let iface_name = self
                            .hir
                            .interfaces
                            .get(&iface_id)
                            .map(|i| i.name.as_str())
                            .unwrap_or("?");
                        self.errors.push(ValidationError {
                            kind: ErrorKind::MethodSignatureMismatch {
                                iface: iface_name.to_string(),
                                method: req_field.name.clone(),
                                detail: format!(
                                    "field type mismatch: expected '{}', found '{}'",
                                    self.format_type(&req_field.ty),
                                    self.format_type(&actual_field.ty)
                                ),
                            },
                            module: module_name.to_string(),
                            context: Some(format!("in struct '{struct_name}'")),
                        });
                    }
                }
            }
        }

        // Check methods
        for &req_fn_id in &required_method_ids {
            let req_fn = match self.hir.functions.get(&req_fn_id) {
                Some(f) => f.clone(),
                None => continue,
            };

            let found = method_ids.iter().find(|&&m_id| {
                self.hir
                    .functions
                    .get(&m_id)
                    .map(|f| f.name == req_fn.name)
                    .unwrap_or(false)
            });

            match found {
                None => {
                    if !self.ast_fn_bodies.contains_key(&req_fn_id) && req_fn.body.is_none() {
                        let iface_name = self
                            .hir
                            .interfaces
                            .get(&iface_id)
                            .map(|i| i.name.as_str())
                            .unwrap_or("?");
                        self.errors.push(ValidationError {
                            kind: ErrorKind::MissingInterfaceMethod {
                                iface: iface_name.to_string(),
                                method: req_fn.name.clone(),
                            },
                            module: module_name.to_string(),
                            context: Some(format!("in struct '{struct_name}'")),
                        });
                    }
                }
                Some(&actual_fn_id) => {
                    if let Some(actual_fn) = self.hir.functions.get(&actual_fn_id) {
                        let actual_fn = actual_fn.clone();
                        self.check_method_signature_match(
                            iface_id,
                            &req_fn,
                            &actual_fn,
                            struct_name,
                            module_name,
                        );
                    }
                }
            }
        }
    }

    /// Collect all fields and method IDs required by an interface (including inherited)
    fn collect_interface_requirements(&self, iface_id: TypeId) -> (Vec<HirField>, Vec<FnId>) {
        let mut fields = Vec::new();
        let mut methods = Vec::new();

        if let Some(iface) = self.hir.interfaces.get(&iface_id) {
            fields.extend(iface.fields.clone());
            methods.extend(iface.methods.clone());

            // Recursively collect from parent interfaces
            for &parent_id in &iface.extends {
                let (parent_fields, parent_methods) =
                    self.collect_interface_requirements(parent_id);
                for f in parent_fields {
                    if !fields.iter().any(|ef| ef.name == f.name) {
                        fields.push(f);
                    }
                }
                for m in parent_methods {
                    if !methods.contains(&m) {
                        methods.push(m);
                    }
                }
            }
        }

        (fields, methods)
    }

    fn check_method_signature_match(
        &mut self,
        iface_id: TypeId,
        expected: &hir::HirFunction,
        actual: &hir::HirFunction,
        struct_name: &str,
        module_name: &str,
    ) {
        let iface_name = self
            .hir
            .interfaces
            .get(&iface_id)
            .map(|i| i.name.as_str())
            .unwrap_or("?")
            .to_string();

        // Check param count
        if expected.params.len() != actual.params.len() {
            self.errors.push(ValidationError {
                kind: ErrorKind::MethodSignatureMismatch {
                    iface: iface_name.clone(),
                    method: expected.name.clone(),
                    detail: format!(
                        "expected {} params, found {}",
                        expected.params.len(),
                        actual.params.len()
                    ),
                },
                module: module_name.to_string(),
                context: Some(format!("in struct '{struct_name}'")),
            });
            return;
        }

        // Check param types
        for (i, (exp_p, act_p)) in expected.params.iter().zip(&actual.params).enumerate() {
            if !self.types_compatible(&exp_p.ty, &act_p.ty) {
                self.errors.push(ValidationError {
                    kind: ErrorKind::MethodSignatureMismatch {
                        iface: iface_name.clone(),
                        method: expected.name.clone(),
                        detail: format!(
                            "param {} type mismatch: expected '{}', found '{}'",
                            i,
                            self.format_type(&exp_p.ty),
                            self.format_type(&act_p.ty)
                        ),
                    },
                    module: module_name.to_string(),
                    context: Some(format!("in struct '{struct_name}'")),
                });
            }
        }

        // Check return type
        if !self.types_compatible(&expected.return_type, &actual.return_type) {
            self.errors.push(ValidationError {
                kind: ErrorKind::MethodSignatureMismatch {
                    iface: iface_name,
                    method: expected.name.clone(),
                    detail: format!(
                        "return type mismatch: expected '{}', found '{}'",
                        self.format_type(&expected.return_type),
                        self.format_type(&actual.return_type)
                    ),
                },
                module: module_name.to_string(),
                context: Some(format!("in struct '{struct_name}'")),
            });
        }
    }
}
