use crate::ast::{self, ImportSymbols, Item};
use crate::hir::{self, ModuleId, ResolvedType, TypeId};

use super::errors::{ErrorKind, ValidationError};
use super::scope::{ModuleScope, TypeAliasInfo, TypeKind, VisibleNames};
use super::Validator;

impl Validator {
    /// Pass 0: Register all modules, their top-level items, and resolve imports
    pub(crate) fn pass_register_modules(&mut self) {
        let modules = std::mem::take(&mut self.modules);

        // First pass: allocate module IDs and register all type/function names
        for module in &modules {
            let module_id = self.next_module_id();
            self.module_name_to_id.insert(module.name.clone(), module_id);

            let mut scope = ModuleScope::default();
            let hir_module = hir::HirModule {
                id: module_id,
                name: module.name.clone(),
                structs: vec![],
                interfaces: vec![],
                functions: vec![],
                imports: vec![],
            };
            self.hir.modules.insert(module_id, hir_module);

            for item in &module.program.items {
                match item {
                    Item::Struct(decl) => {
                        let type_id = self.next_type_id();
                        if scope.types.contains_key(&decl.name) {
                            self.errors.push(ValidationError {
                                kind: ErrorKind::DuplicateType {
                                    name: decl.name.clone(),
                                },
                                module: module.name.clone(),
                                context: None,
                            });
                            continue;
                        }
                        scope.types.insert(decl.name.clone(), type_id);
                        self.type_kinds.insert(type_id, TypeKind::Struct);
                        self.hir.structs.insert(
                            type_id,
                            hir::HirStruct {
                                id: type_id,
                                module: module_id,
                                name: decl.name.clone(),
                                type_params: vec![],
                                fields: vec![],
                                methods: vec![],
                                implements: vec![],
                                specialized_methods: vec![],
                                specialized_implements: vec![],
                            },
                        );
                        self.hir.modules.get_mut(&module_id).unwrap().structs.push(type_id);
                    }

                    Item::Interface(decl) => {
                        let type_id = self.next_type_id();
                        if scope.types.contains_key(&decl.name) {
                            self.errors.push(ValidationError {
                                kind: ErrorKind::DuplicateType {
                                    name: decl.name.clone(),
                                },
                                module: module.name.clone(),
                                context: None,
                            });
                            continue;
                        }
                        scope.types.insert(decl.name.clone(), type_id);
                        self.type_kinds.insert(type_id, TypeKind::Interface);
                        self.hir.interfaces.insert(
                            type_id,
                            hir::HirInterface {
                                id: type_id,
                                module: module_id,
                                name: decl.name.clone(),
                                type_params: vec![],
                                fields: vec![],
                                methods: vec![],
                                extends: vec![],
                            },
                        );
                        self.hir
                            .modules
                            .get_mut(&module_id)
                            .unwrap()
                            .interfaces
                            .push(type_id);
                    }

                    Item::TypeAlias(decl) => {
                        let type_id = self.next_type_id();
                        if scope.types.contains_key(&decl.name) {
                            self.errors.push(ValidationError {
                                kind: ErrorKind::DuplicateType {
                                    name: decl.name.clone(),
                                },
                                module: module.name.clone(),
                                context: None,
                            });
                            continue;
                        }
                        scope.types.insert(decl.name.clone(), type_id);
                        self.type_kinds.insert(type_id, TypeKind::Alias);
                        self.type_aliases.insert(
                            type_id,
                            TypeAliasInfo {
                                name: decl.name.clone(),
                                generic_names: decl.generics.iter().map(|g| g.name.clone()).collect(),
                                type_param_ids: vec![], // filled in pass 2
                                body: decl.ty.clone(),
                            },
                        );
                    }

                    Item::Function(decl) => {
                        let fn_id = self.next_fn_id();
                        if scope.functions.contains_key(&decl.name) {
                            self.errors.push(ValidationError {
                                kind: ErrorKind::DuplicateFunction {
                                    name: decl.name.clone(),
                                },
                                module: module.name.clone(),
                                context: None,
                            });
                            continue;
                        }
                        scope.functions.insert(decl.name.clone(), fn_id);
                        self.hir.functions.insert(
                            fn_id,
                            hir::HirFunction {
                                id: fn_id,
                                module: module_id,
                                name: decl.name.clone(),
                                owner: None,
                                has_self: decl.has_self_param,
                                type_params: vec![],
                                params: vec![],
                                return_type: ResolvedType::Null,
                                body: None,
                            },
                        );
                        if let Some(body) = &decl.body {
                            self.ast_fn_bodies.insert(fn_id, body.clone());
                        }
                        self.hir
                            .modules
                            .get_mut(&module_id)
                            .unwrap()
                            .functions
                            .push(fn_id);
                    }

                    Item::Extend(decl) => {
                        self.pending_extends.push((module_id, decl.clone()));
                    }

                    Item::Import(_) => {
                        // Deferred to import resolution below
                    }
                }
            }

            self.module_scopes.insert(module_id, scope);
        }

        // Build visible names for each module (start with own scope + builtins)
        for (&module_id, scope) in &self.module_scopes {
            let mut visible = VisibleNames::default();
            for (name, &id) in &scope.types {
                visible.types.insert(name.clone(), id);
            }
            for (name, &id) in &scope.functions {
                visible.functions.insert(name.clone(), id);
            }
            self.visible_names.insert(module_id, visible);
        }

        // Resolve imports
        for module in &modules {
            let module_id = self.module_name_to_id[&module.name];
            for item in &module.program.items {
                if let Item::Import(import_decl) = item {
                    self.resolve_import(module_id, &module.name, import_decl);
                }
            }
        }

        // Store modules back (we still need them for later passes)
        self.modules = modules;
    }

    fn resolve_import(
        &mut self,
        importer_id: ModuleId,
        importer_name: &str,
        import: &ast::ImportDecl,
    ) {
        let target_module_id = match self.module_name_to_id.get(&import.module) {
            Some(&id) => id,
            None => {
                self.errors.push(ValidationError {
                    kind: ErrorKind::UndefinedModule(import.module.clone()),
                    module: importer_name.to_string(),
                    context: None,
                });
                return;
            }
        };

        let target_scope = match self.module_scopes.get(&target_module_id) {
            Some(s) => s,
            None => return,
        };

        let visible = self.visible_names.get_mut(&importer_id).unwrap();
        let hir_module = self.hir.modules.get_mut(&importer_id).unwrap();

        match &import.symbols {
            ImportSymbols::Glob => {
                // Import all types and functions from target module
                for (name, &id) in &target_scope.types {
                    visible.types.insert(name.clone(), id);
                }
                for (name, &id) in &target_scope.functions {
                    visible.functions.insert(name.clone(), id);
                }
                hir_module
                    .imports
                    .push(hir::HirImport::Glob(target_module_id));
            }
            ImportSymbols::Named(symbols) => {
                let mut hir_symbols = Vec::new();
                for sym in symbols {
                    let local_name = sym.alias.as_ref().unwrap_or(&sym.name);

                    if let Some(&type_id) = target_scope.types.get(&sym.name) {
                        visible.types.insert(local_name.clone(), type_id);
                        hir_symbols.push(hir::HirImportSymbol::Type(type_id, local_name.clone()));
                    } else if let Some(&fn_id) = target_scope.functions.get(&sym.name) {
                        visible.functions.insert(local_name.clone(), fn_id);
                        hir_symbols
                            .push(hir::HirImportSymbol::Function(fn_id, local_name.clone()));
                    } else {
                        self.errors.push(ValidationError {
                            kind: ErrorKind::ImportSymbolNotFound {
                                module: import.module.clone(),
                                symbol: sym.name.clone(),
                            },
                            module: importer_name.to_string(),
                            context: None,
                        });
                    }
                }
                hir_module
                    .imports
                    .push(hir::HirImport::Named(target_module_id, hir_symbols));
            }
        }
    }

    /// Pass 1: Register methods on structs/interfaces, resolve implements/extends clauses
    pub(crate) fn pass_register_type_shapes(&mut self) {
        let modules = std::mem::take(&mut self.modules);

        for module in &modules {
            let module_id = self.module_name_to_id[&module.name];

            for item in &module.program.items {
                match item {
                    Item::Struct(decl) => {
                        let visible = self.visible_names.get(&module_id).cloned().unwrap_or_default();
                        let type_id = visible.types[&decl.name];

                        // Register methods
                        for method in &decl.methods {
                            let fn_id = self.next_fn_id();
                            self.hir.functions.insert(
                                fn_id,
                                hir::HirFunction {
                                    id: fn_id,
                                    module: module_id,
                                    name: method.name.clone(),
                                    owner: Some(type_id),
                                    has_self: method.has_self_param,
                                    type_params: vec![],
                                    params: vec![],
                                    return_type: ResolvedType::Null,
                                    body: None,
                                },
                            );
                            if let Some(body) = &method.body {
                                self.ast_fn_bodies.insert(fn_id, body.clone());
                            }
                            self.hir.structs.get_mut(&type_id).unwrap().methods.push(fn_id);
                        }

                        // Resolve implements clauses (name-only, just TypeExpr -> TypeId)
                        for iface_expr in &decl.implements {
                            if let Some(iface_id) = self.resolve_type_name_to_id(module_id, iface_expr, &module.name) {
                                self.hir
                                    .structs
                                    .get_mut(&type_id)
                                    .unwrap()
                                    .implements
                                    .push(iface_id);
                            }
                        }
                    }

                    Item::Interface(decl) => {
                        let visible = self.visible_names.get(&module_id).cloned().unwrap_or_default();
                        let type_id = visible.types[&decl.name];

                        // Register methods
                        for method in &decl.methods {
                            let fn_id = self.next_fn_id();
                            self.hir.functions.insert(
                                fn_id,
                                hir::HirFunction {
                                    id: fn_id,
                                    module: module_id,
                                    name: method.name.clone(),
                                    owner: Some(type_id),
                                    has_self: method.has_self_param,
                                    type_params: vec![],
                                    params: vec![],
                                    return_type: ResolvedType::Null,
                                    body: None,
                                },
                            );
                            if let Some(body) = &method.body {
                                self.ast_fn_bodies.insert(fn_id, body.clone());
                            }
                            self.hir
                                .interfaces
                                .get_mut(&type_id)
                                .unwrap()
                                .methods
                                .push(fn_id);
                        }

                        // Resolve extends clauses
                        for parent_expr in &decl.implements {
                            if let Some(parent_id) = self.resolve_type_name_to_id(module_id, parent_expr, &module.name) {
                                self.hir
                                    .interfaces
                                    .get_mut(&type_id)
                                    .unwrap()
                                    .extends
                                    .push(parent_id);
                            }
                        }
                    }

                    _ => {}
                }
            }
        }

        self.modules = modules;
    }

    /// Simple name-only resolution of a TypeExpr to a TypeId (for implements/extends)
    fn resolve_type_name_to_id(
        &mut self,
        module_id: ModuleId,
        type_expr: &ast::TypeExpr,
        module_name: &str,
    ) -> Option<TypeId> {
        match type_expr {
            ast::TypeExpr::Named(name, _) => {
                let visible = self.visible_names.get(&module_id)?;
                if let Some(&id) = visible.types.get(name.as_str()) {
                    Some(id)
                } else {
                    self.errors.push(ValidationError {
                        kind: ErrorKind::UndefinedType(name.clone()),
                        module: module_name.to_string(),
                        context: None,
                    });
                    None
                }
            }
            _ => {
                self.errors.push(ValidationError {
                    kind: ErrorKind::UndefinedType(format!("{type_expr:?}")),
                    module: module_name.to_string(),
                    context: Some("expected a named type".to_string()),
                });
                None
            }
        }
    }

    /// Pass 2: Register type params, resolve field types and function signatures
    pub(crate) fn pass_register_type_params(&mut self) {
        let modules = std::mem::take(&mut self.modules);

        for module in &modules {
            let module_id = self.module_name_to_id[&module.name];

            for item in &module.program.items {
                match item {
                    Item::Struct(decl) => {
                        let visible = self.visible_names.get(&module_id).cloned().unwrap_or_default();
                        let type_id = visible.types[&decl.name];
                        self.register_struct_type_params(module_id, type_id, decl);
                    }
                    Item::Interface(decl) => {
                        let visible = self.visible_names.get(&module_id).cloned().unwrap_or_default();
                        let type_id = visible.types[&decl.name];
                        self.register_interface_type_params(module_id, type_id, decl);
                    }
                    Item::Function(decl) => {
                        let visible = self.visible_names.get(&module_id).cloned().unwrap_or_default();
                        let fn_id = visible.functions[&decl.name];
                        self.register_function_type_params(module_id, fn_id, decl, None);
                    }
                    Item::TypeAlias(decl) => {
                        let visible = self.visible_names.get(&module_id).cloned().unwrap_or_default();
                        let type_id = visible.types[&decl.name];
                        self.register_alias_type_params(type_id, decl);
                    }
                    _ => {}
                }
            }
        }

        self.modules = modules;
    }

    fn register_struct_type_params(
        &mut self,
        module_id: ModuleId,
        type_id: TypeId,
        decl: &ast::StructDecl,
    ) {
        // Allocate type params
        let tp_ids = self.allocate_type_params(&decl.generics);
        self.hir.structs.get_mut(&type_id).unwrap().type_params = tp_ids.clone();

        // Set type param scope for resolving fields and methods
        self.type_param_scope.clear();
        for (i, gp) in decl.generics.iter().enumerate() {
            self.type_param_scope.insert(gp.name.clone(), tp_ids[i]);
        }

        // Resolve type param bounds
        self.resolve_type_param_bounds(module_id, &decl.generics, &tp_ids);

        // Resolve fields
        let mut hir_fields = Vec::new();
        for field in &decl.fields {
            match self.resolve_type(module_id, &field.ty) {
                Ok(resolved) => {
                    hir_fields.push(hir::HirField {
                        name: field.name.clone(),
                        ty: resolved,
                    });
                }
                Err(e) => self.errors.push(e),
            }
        }
        self.hir.structs.get_mut(&type_id).unwrap().fields = hir_fields;

        // Resolve method signatures
        let method_ids: Vec<_> = self.hir.structs[&type_id].methods.clone();
        let method_decls: Vec<_> = decl.methods.clone();
        for (i, method_decl) in method_decls.iter().enumerate() {
            let fn_id = method_ids[i];
            self.register_function_type_params(module_id, fn_id, method_decl, Some(&decl.generics));
        }

        self.type_param_scope.clear();
    }

    fn register_interface_type_params(
        &mut self,
        module_id: ModuleId,
        type_id: TypeId,
        decl: &ast::InterfaceDecl,
    ) {
        let tp_ids = self.allocate_type_params(&decl.generics);
        self.hir.interfaces.get_mut(&type_id).unwrap().type_params = tp_ids.clone();

        self.type_param_scope.clear();
        for (i, gp) in decl.generics.iter().enumerate() {
            self.type_param_scope.insert(gp.name.clone(), tp_ids[i]);
        }

        self.resolve_type_param_bounds(module_id, &decl.generics, &tp_ids);

        // Resolve fields
        let mut hir_fields = Vec::new();
        for field in &decl.fields {
            match self.resolve_type(module_id, &field.ty) {
                Ok(resolved) => {
                    hir_fields.push(hir::HirField {
                        name: field.name.clone(),
                        ty: resolved,
                    });
                }
                Err(e) => self.errors.push(e),
            }
        }
        self.hir.interfaces.get_mut(&type_id).unwrap().fields = hir_fields;

        // Resolve method signatures
        let method_ids: Vec<_> = self.hir.interfaces[&type_id].methods.clone();
        let method_decls: Vec<_> = decl.methods.clone();
        for (i, method_decl) in method_decls.iter().enumerate() {
            let fn_id = method_ids[i];
            self.register_function_type_params(module_id, fn_id, method_decl, Some(&decl.generics));
        }

        self.type_param_scope.clear();
    }

    fn register_function_type_params(
        &mut self,
        module_id: ModuleId,
        fn_id: hir::FnId,
        decl: &ast::FunctionDecl,
        owner_generics: Option<&[ast::GenericParam]>,
    ) {
        // Save and set up type param scope
        let saved_scope = self.type_param_scope.clone();

        // If this is a method, the owner's type params should already be in scope
        // (from the caller). Add the function's own type params on top.
        let fn_tp_ids = self.allocate_type_params(&decl.generics);
        for (i, gp) in decl.generics.iter().enumerate() {
            self.type_param_scope.insert(gp.name.clone(), fn_tp_ids[i]);
        }

        self.hir.functions.get_mut(&fn_id).unwrap().type_params = fn_tp_ids.clone();

        // Resolve type param bounds
        self.resolve_type_param_bounds(module_id, &decl.generics, &fn_tp_ids);

        // Resolve parameter types
        let mut hir_params = Vec::new();
        for param in &decl.params {
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
        self.hir.functions.get_mut(&fn_id).unwrap().params = hir_params;

        // Resolve return type
        let return_type = if let Some(ret_expr) = &decl.return_type {
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
        self.hir.functions.get_mut(&fn_id).unwrap().return_type = return_type;

        // Restore scope (but keep owner generics if we had them)
        if owner_generics.is_some() {
            // Restore to parent scope (owner's type params stay)
            self.type_param_scope = saved_scope;
        } else {
            self.type_param_scope.clear();
        }
    }

    fn register_alias_type_params(&mut self, type_id: TypeId, decl: &ast::TypeAliasDecl) {
        let tp_ids = self.allocate_type_params(&decl.generics);
        if let Some(alias) = self.type_aliases.get_mut(&type_id) {
            alias.type_param_ids = tp_ids;
        }
    }

    pub(crate) fn allocate_type_params(&mut self, generics: &[ast::GenericParam]) -> Vec<hir::TypeParamId> {
        let mut ids = Vec::new();
        for gp in generics {
            let tp_id = self.next_tp_id();
            self.hir.type_params.insert(
                tp_id,
                hir::TypeParamInfo {
                    id: tp_id,
                    name: gp.name.clone(),
                    bounds: vec![],
                },
            );
            ids.push(tp_id);
        }
        ids
    }

    pub(crate) fn resolve_type_param_bounds(
        &mut self,
        module_id: ModuleId,
        generics: &[ast::GenericParam],
        tp_ids: &[hir::TypeParamId],
    ) {
        let module_name = self.module_name(module_id);
        for (i, gp) in generics.iter().enumerate() {
            let mut bound_ids = Vec::new();
            for bound_expr in &gp.bounds {
                match bound_expr {
                    ast::TypeExpr::Named(name, _) => {
                        let visible = self.visible_names.get(&module_id).cloned().unwrap_or_default();
                        if let Some(&bound_type_id) = visible.types.get(name.as_str()) {
                            if self.type_kinds.get(&bound_type_id) == Some(&TypeKind::Interface) {
                                bound_ids.push(bound_type_id);
                            } else {
                                self.errors.push(ValidationError {
                                    kind: ErrorKind::TypeParamBoundNotInterface(name.clone()),
                                    module: module_name.clone(),
                                    context: None,
                                });
                            }
                        } else {
                            self.errors.push(ValidationError {
                                kind: ErrorKind::UndefinedType(name.clone()),
                                module: module_name.clone(),
                                context: Some(format!(
                                    "in bound for type parameter '{}'",
                                    gp.name
                                )),
                            });
                        }
                    }
                    _ => {
                        self.errors.push(ValidationError {
                            kind: ErrorKind::TypeParamBoundNotInterface(format!("{bound_expr:?}")),
                            module: module_name.clone(),
                            context: None,
                        });
                    }
                }
            }
            if let Some(tp_info) = self.hir.type_params.get_mut(&tp_ids[i]) {
                tp_info.bounds = bound_ids;
            }
        }
    }
}
