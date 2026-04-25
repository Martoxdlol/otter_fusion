use std::collections::{HashMap, HashSet};

use crate::{
    ast::{
        self, FunctionDecl, GenericParam, ImportSymbol, ImportSymbols, Item, Module, TypeAliasDecl,
        TypeExpr,
    },
    hir::{
        FnId, Hir, HirField, HirFunction, HirImport, HirImportSymbol, HirInterface, HirModule,
        HirParam, HirStruct, ModuleId, PrimitiveType, ResolvedType, TypeId, TypeParamId,
        TypeParamInfo,
    },
};

#[derive(Debug, Clone)]
pub enum ValidationError {
    DuplicateModule {
        name: String,
    },
    UnknownImportModule {
        in_module: String,
        target: String,
    },
    SelfImport {
        module: String,
    },
    DuplicateName {
        module: String,
        name: String,
    },
    UnknownImportSymbol {
        in_module: String,
        from_module: String,
        symbol: String,
    },
    DuplicateImport {
        in_module: String,
        local_name: String,
    },
    UnknownType {
        module: String,
        name: String,
    },
    GenericArgsOnTypeParam {
        module: String,
        name: String,
    },
    GenericArityMismatch {
        module: String,
        name: String,
        expected: usize,
        actual: usize,
    },
    DuplicateGenericParam {
        scope: String,
        name: String,
    },
    BoundNotInterface {
        scope: String,
        type_param: String,
    },
    ExpectedTypeFoundFunction {
        module: String,
        name: String,
    },
    PointerOutsideExtern {
        location: String,
    },
    DuplicateMethod {
        type_name: String,
        method: String,
    },
    DuplicateField {
        type_name: String,
        field: String,
    },
    DuplicateParam {
        function: String,
        param: String,
    },
    InvalidExtendTarget {
        module: String,
    },
    ImplementsNotInterface {
        type_name: String,
    },
    MissingInterfaceMember {
        type_name: String,
        interface: String,
        member: String,
    },
    InterfaceMemberMismatch {
        type_name: String,
        interface: String,
        member: String,
    },
}

#[derive(Debug, Clone)]
enum ScopeEntry {
    Type(TypeId),
    Function(FnId),
    /// Type alias — resolved by inlining its `TypeAliasDecl` from
    /// `Validator.module_aliases[source][name]`.
    Alias {
        source: ModuleId,
        name: String,
    },
}

#[derive(Debug, Default)]
struct ModuleScope {
    /// Direct, named-resolvable entries: locally-defined names plus named imports.
    direct: HashMap<String, ScopeEntry>,
    /// Source modules pulled in by glob import; consulted only on a `direct` miss.
    globs: Vec<ModuleId>,
    /// Names in `direct` that came from a named import (vs. a local definition).
    /// Used to distinguish "local shadows import" (silent) from
    /// "duplicate import" (error).
    imported_names: HashSet<String>,
}

/// Per-module record of named imports we couldn't resolve in phase 0 because
/// type and function names hadn't been registered yet. Drained in phase 1.
struct PendingNamedImport {
    importer: ModuleId,
    source: ModuleId,
    symbols: Vec<ImportSymbol>,
}

/// Context for resolving an AST `TypeExpr` into a `ResolvedType`.
struct TypeResolveCtx {
    module: ModuleId,
    /// Generic scopes from outermost to innermost. Lookup walks innermost-first
    /// so inner names shadow outer ones (decision: shadow on collision).
    generics: Vec<Vec<(String, TypeParamId)>>,
    /// Local name → `ResolvedType` mapping used while inlining a generic
    /// alias body. Takes precedence over `generics`.
    local_subst: HashMap<String, ResolvedType>,
    /// Whether `*T` / `is_pointer` is acceptable in this position. Set only
    /// for extern-context declarations.
    allow_pointer: bool,
}

pub struct Validator {
    modules: Vec<Module>,
    hir: Hir,
    errors: Vec<ValidationError>,

    module_ids: HashMap<String, ModuleId>,
    module_scopes: HashMap<ModuleId, ModuleScope>,
    /// Side table of type aliases — never materialized in HIR; inlined at
    /// resolution time.
    module_aliases: HashMap<ModuleId, HashMap<String, TypeAliasDecl>>,

    pending_named_imports: Vec<PendingNamedImport>,
    /// Function (top-level + method) AST decls keyed by their HIR id, kept
    /// alive across phases for signature resolution (phase 2) and body
    /// validation (phase 5).
    pending_function_bodies: HashMap<FnId, FunctionDecl>,

    /// Generic param scope per struct/interface (name → `TypeParamId`).
    type_generics: HashMap<TypeId, Vec<(String, TypeParamId)>>,
    /// Generic param scope per function/method (only the function's own
    /// generics; the method's owner generics live in `type_generics`).
    fn_generics: HashMap<FnId, Vec<(String, TypeParamId)>>,

    /// Implements clauses contributed by extend blocks (resolved interface
    /// types). Drained by phase 4 and merged into the target struct's
    /// `implements` list.
    pending_extend_implements: Vec<(TypeId, Vec<ResolvedType>)>,

    next_module_id: u32,
    next_type_id: u32,
    next_fn_id: u32,
    next_tp_id: u32,
}

impl Validator {
    pub fn new(modules: Vec<Module>) -> Self {
        Self {
            modules,
            hir: Hir::default(),
            errors: Vec::new(),
            module_ids: HashMap::new(),
            module_scopes: HashMap::new(),
            module_aliases: HashMap::new(),
            pending_named_imports: Vec::new(),
            pending_function_bodies: HashMap::new(),
            type_generics: HashMap::new(),
            fn_generics: HashMap::new(),
            pending_extend_implements: Vec::new(),
            next_module_id: 0,
            next_type_id: 0,
            next_fn_id: 0,
            next_tp_id: 0,
        }
    }

    pub fn validate(mut self) -> Result<Hir, Vec<ValidationError>> {
        let modules = std::mem::take(&mut self.modules);

        // step 0: register modules & resolve imports
        self.register_modules_and_imports(&modules);
        // step 1: register type names
        self.register_type_names(&modules);
        // step 2: register type params, resolve fields & signatures
        self.register_type_params_and_signatures(&modules);
        // step 3: merge extend blocks
        self.merge_extend_blocks(&modules);
        // step 4: validate interface implementations
        self.validate_implementations(&modules);
        // step 5: validate function bodies

        if !self.errors.is_empty() {
            return Err(self.errors);
        }

        todo!("phase 5 not yet implemented")
    }

    fn register_modules_and_imports(&mut self, modules: &[Module]) {
        for module in modules {
            if self.module_ids.contains_key(&module.name) {
                self.errors.push(ValidationError::DuplicateModule {
                    name: module.name.clone(),
                });
                continue;
            }

            let id = ModuleId(self.next_module_id());
            self.module_ids.insert(module.name.clone(), id);
            self.hir.modules.insert(
                id,
                HirModule {
                    id,
                    name: module.name.clone(),
                    structs: Vec::new(),
                    interfaces: Vec::new(),
                    functions: Vec::new(),
                    imports: Vec::new(),
                },
            );
        }

        for module in modules {
            let importer = match self.module_ids.get(&module.name) {
                Some(id) => *id,
                None => continue,
            };

            for item in &module.program.items {
                let Item::Import(import) = item else { continue };

                if import.module == module.name {
                    self.errors.push(ValidationError::SelfImport {
                        module: module.name.clone(),
                    });
                    continue;
                }

                let source = match self.module_ids.get(&import.module) {
                    Some(id) => *id,
                    None => {
                        self.errors.push(ValidationError::UnknownImportModule {
                            in_module: module.name.clone(),
                            target: import.module.clone(),
                        });
                        continue;
                    }
                };

                match &import.symbols {
                    ImportSymbols::Glob => {
                        self.hir
                            .modules
                            .get_mut(&importer)
                            .unwrap()
                            .imports
                            .push(HirImport::Glob(source));
                    }
                    ImportSymbols::Named(symbols) => {
                        self.pending_named_imports.push(PendingNamedImport {
                            importer,
                            source,
                            symbols: symbols.clone(),
                        });
                    }
                }
            }
        }
    }

    fn register_type_names(&mut self, modules: &[Module]) {
        for module in modules {
            let module_id = match self.module_ids.get(&module.name) {
                Some(id) => *id,
                None => continue,
            };

            let mut scope = ModuleScope::default();
            scope.globs = self.hir.modules[&module_id]
                .imports
                .iter()
                .filter_map(|i| match i {
                    HirImport::Glob(id) => Some(*id),
                    HirImport::Named(_, _) => None,
                })
                .collect();

            for item in &module.program.items {
                match item {
                    Item::Struct(decl) => {
                        if scope.direct.contains_key(&decl.name) {
                            self.errors.push(ValidationError::DuplicateName {
                                module: module.name.clone(),
                                name: decl.name.clone(),
                            });
                            continue;
                        }
                        let id = TypeId(self.next_type_id());
                        self.hir.structs.insert(
                            id,
                            HirStruct {
                                id,
                                module: module_id,
                                name: decl.name.clone(),
                                type_params: Vec::new(),
                                fields: Vec::new(),
                                methods: Vec::new(),
                                specialised_methods: Vec::new(),
                                implements: Vec::new(),
                            },
                        );
                        self.hir
                            .modules
                            .get_mut(&module_id)
                            .unwrap()
                            .structs
                            .push(id);
                        scope.direct.insert(decl.name.clone(), ScopeEntry::Type(id));
                    }
                    Item::Interface(decl) => {
                        if scope.direct.contains_key(&decl.name) {
                            self.errors.push(ValidationError::DuplicateName {
                                module: module.name.clone(),
                                name: decl.name.clone(),
                            });
                            continue;
                        }
                        let id = TypeId(self.next_type_id());
                        self.hir.interfaces.insert(
                            id,
                            HirInterface {
                                id,
                                module: module_id,
                                name: decl.name.clone(),
                                type_params: Vec::new(),
                                fields: Vec::new(),
                                methods: Vec::new(),
                                extends: Vec::new(),
                            },
                        );
                        self.hir
                            .modules
                            .get_mut(&module_id)
                            .unwrap()
                            .interfaces
                            .push(id);
                        scope.direct.insert(decl.name.clone(), ScopeEntry::Type(id));
                    }
                    Item::Function(decl) => {
                        if scope.direct.contains_key(&decl.name) {
                            self.errors.push(ValidationError::DuplicateName {
                                module: module.name.clone(),
                                name: decl.name.clone(),
                            });
                            continue;
                        }
                        let id = FnId(self.next_fn_id());
                        // Skeleton — signature filled in phase 2,
                        // body in phase 5.
                        self.hir.functions.insert(
                            id,
                            HirFunction {
                                id,
                                module: module_id,
                                name: decl.name.clone(),
                                owner: None,
                                has_self: false,
                                type_params: Vec::new(),
                                params: Vec::new(),
                                return_type: ResolvedType::Null,
                                body: None,
                            },
                        );
                        self.hir
                            .modules
                            .get_mut(&module_id)
                            .unwrap()
                            .functions
                            .push(id);
                        scope
                            .direct
                            .insert(decl.name.clone(), ScopeEntry::Function(id));
                        self.pending_function_bodies.insert(id, decl.clone());
                    }
                    Item::TypeAlias(decl) => {
                        if scope.direct.contains_key(&decl.name) {
                            self.errors.push(ValidationError::DuplicateName {
                                module: module.name.clone(),
                                name: decl.name.clone(),
                            });
                            continue;
                        }
                        self.module_aliases
                            .entry(module_id)
                            .or_default()
                            .insert(decl.name.clone(), decl.clone());
                        scope.direct.insert(
                            decl.name.clone(),
                            ScopeEntry::Alias {
                                source: module_id,
                                name: decl.name.clone(),
                            },
                        );
                    }
                    Item::Extend(_) | Item::Import(_) => {}
                }
            }

            self.module_scopes.insert(module_id, scope);
        }

        // 1b: Finalize pending named imports now that every source module's
        //     direct scope exists.
        let pending = std::mem::take(&mut self.pending_named_imports);
        for p in pending {
            let importer_name = self.hir.modules[&p.importer].name.clone();
            let source_name = self.hir.modules[&p.source].name.clone();
            let mut resolved_symbols: Vec<HirImportSymbol> = Vec::new();

            for sym in p.symbols {
                let local = sym.alias.clone().unwrap_or_else(|| sym.name.clone());

                let resolved = self
                    .module_scopes
                    .get(&p.source)
                    .and_then(|s| s.direct.get(&sym.name))
                    .cloned();

                let (entry, hir_symbol) = match resolved {
                    Some(ScopeEntry::Type(id)) => (
                        ScopeEntry::Type(id),
                        HirImportSymbol::Type(id, local.clone()),
                    ),
                    Some(ScopeEntry::Function(id)) => (
                        ScopeEntry::Function(id),
                        HirImportSymbol::Function(id, local.clone()),
                    ),
                    Some(ScopeEntry::Alias { source, name }) => (
                        ScopeEntry::Alias {
                            source,
                            name: name.clone(),
                        },
                        HirImportSymbol::Alias {
                            source,
                            original: name,
                            local: local.clone(),
                        },
                    ),
                    None => {
                        self.errors.push(ValidationError::UnknownImportSymbol {
                            in_module: importer_name.clone(),
                            from_module: source_name.clone(),
                            symbol: sym.name.clone(),
                        });
                        continue;
                    }
                };

                let importer_scope = self.module_scopes.get_mut(&p.importer).unwrap();

                if importer_scope.direct.contains_key(&local) {
                    if importer_scope.imported_names.contains(&local) {
                        self.errors.push(ValidationError::DuplicateImport {
                            in_module: importer_name.clone(),
                            local_name: local.clone(),
                        });
                        continue;
                    }
                    // Local definition shadows the import; record in HIR but
                    // skip adding to the lookup scope.
                } else {
                    importer_scope.direct.insert(local.clone(), entry);
                    importer_scope.imported_names.insert(local.clone());
                }

                resolved_symbols.push(hir_symbol);
            }

            if !resolved_symbols.is_empty() {
                self.hir
                    .modules
                    .get_mut(&p.importer)
                    .unwrap()
                    .imports
                    .push(HirImport::Named(p.source, resolved_symbols));
            }
        }
    }

    fn register_type_params_and_signatures(&mut self, modules: &[Module]) {
        // 2a: Mint type params for structs/interfaces, mint method FnIds with
        //     skeleton HirFunctions, and mint method/top-level fn generics.
        //     After this pass every HIR entity has its `type_params` filled.
        for module in modules {
            let module_id = match self.module_ids.get(&module.name) {
                Some(id) => *id,
                None => continue,
            };

            for item in &module.program.items {
                match item {
                    Item::Struct(decl) => {
                        let type_id = match self.find_type_id(module_id, &decl.name) {
                            Some(id) => id,
                            None => continue,
                        };
                        let scope_name = format!("{}::{}", module.name, decl.name);
                        let scope = self.mint_generics(&decl.generics, scope_name);
                        let ids: Vec<TypeParamId> = scope.iter().map(|(_, id)| *id).collect();
                        self.hir.structs.get_mut(&type_id).unwrap().type_params = ids;
                        self.type_generics.insert(type_id, scope);

                        self.mint_methods(
                            module_id,
                            type_id,
                            &module.name,
                            &decl.name,
                            &decl.methods,
                            /*is_struct=*/ true,
                        );
                    }
                    Item::Interface(decl) => {
                        let type_id = match self.find_type_id(module_id, &decl.name) {
                            Some(id) => id,
                            None => continue,
                        };
                        let scope_name = format!("{}::{}", module.name, decl.name);
                        let scope = self.mint_generics(&decl.generics, scope_name);
                        let ids: Vec<TypeParamId> = scope.iter().map(|(_, id)| *id).collect();
                        self.hir.interfaces.get_mut(&type_id).unwrap().type_params = ids;
                        self.type_generics.insert(type_id, scope);

                        self.mint_methods(
                            module_id,
                            type_id,
                            &module.name,
                            &decl.name,
                            &decl.methods,
                            /*is_struct=*/ false,
                        );
                    }
                    Item::Function(decl) => {
                        let fn_id = match self.find_fn_id(module_id, &decl.name) {
                            Some(id) => id,
                            None => continue,
                        };
                        let scope_name = format!("{}::{}", module.name, decl.name);
                        let scope = self.mint_generics(&decl.generics, scope_name);
                        let ids: Vec<TypeParamId> = scope.iter().map(|(_, id)| *id).collect();
                        self.hir.functions.get_mut(&fn_id).unwrap().type_params = ids;
                        self.fn_generics.insert(fn_id, scope);
                    }
                    Item::TypeAlias(_) | Item::Extend(_) | Item::Import(_) => {}
                }
            }
        }

        // 2b: Resolve generic bounds for structs, interfaces, and free
        //     functions. Now that every type's arity is known, arity checks
        //     work; bounds within one scope can reference siblings.
        for module in modules {
            let module_id = match self.module_ids.get(&module.name) {
                Some(id) => *id,
                None => continue,
            };
            for item in &module.program.items {
                match item {
                    Item::Struct(decl) => {
                        let type_id = match self.find_type_id(module_id, &decl.name) {
                            Some(id) => id,
                            None => continue,
                        };
                        let scope_name = format!("{}::{}", module.name, decl.name);
                        self.resolve_generic_bounds(
                            module_id,
                            &decl.generics,
                            self.type_generics
                                .get(&type_id)
                                .cloned()
                                .unwrap_or_default(),
                            &scope_name,
                        );
                    }
                    Item::Interface(decl) => {
                        let type_id = match self.find_type_id(module_id, &decl.name) {
                            Some(id) => id,
                            None => continue,
                        };
                        let scope_name = format!("{}::{}", module.name, decl.name);
                        self.resolve_generic_bounds(
                            module_id,
                            &decl.generics,
                            self.type_generics
                                .get(&type_id)
                                .cloned()
                                .unwrap_or_default(),
                            &scope_name,
                        );
                    }
                    _ => {}
                }
            }
        }

        // 2b (cont.): Resolve generic bounds for every function/method.
        let pending = std::mem::take(&mut self.pending_function_bodies);
        for (fn_id, decl) in &pending {
            let func = self.hir.functions[fn_id].clone();
            let owner_scope = func
                .owner
                .and_then(|t| self.type_generics.get(&t))
                .cloned()
                .unwrap_or_default();
            let own_scope = self.fn_generics.get(fn_id).cloned().unwrap_or_default();
            let module_name = self.hir.modules[&func.module].name.clone();
            let scope_name = format!("{}::{}", module_name, func.name);
            // Bounds resolve in `[owner, own]` so a method's own bounds can
            // reference owner generics.
            let mut combined = Vec::new();
            if !owner_scope.is_empty() {
                combined.push(owner_scope.clone());
            }
            combined.push(own_scope.clone());
            self.resolve_generic_bounds_with_scope(
                func.module,
                &decl.generics,
                combined,
                &own_scope,
                &scope_name,
            );
        }

        // 2c: Resolve struct/interface field types.
        for module in modules {
            let module_id = match self.module_ids.get(&module.name) {
                Some(id) => *id,
                None => continue,
            };
            for item in &module.program.items {
                match item {
                    Item::Struct(decl) => {
                        let type_id = match self.find_type_id(module_id, &decl.name) {
                            Some(id) => id,
                            None => continue,
                        };
                        let fields = self.resolve_fields(
                            module_id,
                            type_id,
                            &decl.fields,
                            decl.is_extern,
                            &module.name,
                            &decl.name,
                        );
                        self.hir.structs.get_mut(&type_id).unwrap().fields = fields;
                    }
                    Item::Interface(decl) => {
                        let type_id = match self.find_type_id(module_id, &decl.name) {
                            Some(id) => id,
                            None => continue,
                        };
                        let fields = self.resolve_fields(
                            module_id,
                            type_id,
                            &decl.fields,
                            /*is_extern=*/ false,
                            &module.name,
                            &decl.name,
                        );
                        self.hir.interfaces.get_mut(&type_id).unwrap().fields = fields;
                    }
                    _ => {}
                }
            }
        }

        // 2d: Resolve function/method signatures. Bodies stay raw AST in
        //     `pending_function_bodies` for phase 5.
        for (fn_id, decl) in &pending {
            let func = self.hir.functions[fn_id].clone();
            let owner_scope = func
                .owner
                .and_then(|t| self.type_generics.get(&t))
                .cloned()
                .unwrap_or_default();
            let own_scope = self.fn_generics.get(fn_id).cloned().unwrap_or_default();
            let mut generics = Vec::new();
            if !owner_scope.is_empty() {
                generics.push(owner_scope);
            }
            generics.push(own_scope);

            let module_name = self.hir.modules[&func.module].name.clone();
            let fn_label = format!("{}::{}", module_name, func.name);
            self.resolve_function_signature(*fn_id, decl, func.module, generics, &fn_label);
        }

        // Restore pending bodies for phase 5.
        self.pending_function_bodies = pending;
    }

    fn merge_extend_blocks(&mut self, modules: &[Module]) {
        for module in modules {
            let module_id = match self.module_ids.get(&module.name) {
                Some(id) => *id,
                None => continue,
            };

            for item in &module.program.items {
                let Item::Extend(decl) = item else { continue };

                let extend_label = format!("{}::extend", module.name);
                let extend_scope = self.mint_generics(&decl.generic_params, extend_label.clone());

                self.resolve_generic_bounds_with_scope(
                    module_id,
                    &decl.generic_params,
                    vec![extend_scope.clone()],
                    &extend_scope,
                    &extend_label,
                );

                let target_ctx = TypeResolveCtx {
                    module: module_id,
                    generics: vec![extend_scope.clone()],
                    local_subst: HashMap::new(),
                    allow_pointer: false,
                };
                let target_resolved = self.resolve_type_expr(&decl.target, &target_ctx);

                let (target_id, target_args) = match target_resolved {
                    ResolvedType::Struct(id, args) => (id, args),
                    ResolvedType::Null => continue, // resolution already errored
                    _ => {
                        self.errors.push(ValidationError::InvalidExtendTarget {
                            module: module.name.clone(),
                        });
                        continue;
                    }
                };

                let target_name = self.hir.structs[&target_id].name.clone();
                let target_label = format!("{}::{}", module.name, target_name);

                let mut seen: HashSet<String> = HashSet::new();
                for method in &decl.methods {
                    if !seen.insert(method.name.clone()) {
                        self.errors.push(ValidationError::DuplicateMethod {
                            type_name: target_label.clone(),
                            method: method.name.clone(),
                        });
                        continue;
                    }

                    let fn_id = FnId(self.next_fn_id());
                    self.hir.functions.insert(
                        fn_id,
                        HirFunction {
                            id: fn_id,
                            module: module_id,
                            name: method.name.clone(),
                            owner: Some(target_id),
                            has_self: method.has_self_param,
                            type_params: Vec::new(),
                            params: Vec::new(),
                            return_type: ResolvedType::Null,
                            body: None,
                        },
                    );

                    let method_label = format!("{}::{}", target_label, method.name);
                    let method_scope = self.mint_generics(&method.generics, method_label.clone());
                    let ids: Vec<TypeParamId> = method_scope.iter().map(|(_, id)| *id).collect();
                    self.hir.functions.get_mut(&fn_id).unwrap().type_params = ids;
                    self.fn_generics.insert(fn_id, method_scope.clone());

                    let combined = vec![extend_scope.clone(), method_scope.clone()];
                    self.resolve_generic_bounds_with_scope(
                        module_id,
                        &method.generics,
                        combined.clone(),
                        &method_scope,
                        &method_label,
                    );

                    self.resolve_function_signature(
                        fn_id,
                        method,
                        module_id,
                        combined,
                        &method_label,
                    );

                    self.hir
                        .structs
                        .get_mut(&target_id)
                        .unwrap()
                        .specialised_methods
                        .push((target_args.clone(), fn_id));

                    self.pending_function_bodies.insert(fn_id, method.clone());
                }

                if !decl.implements.is_empty() {
                    let mut resolved_impls: Vec<ResolvedType> = Vec::new();
                    for impl_expr in &decl.implements {
                        resolved_impls.push(self.resolve_type_expr(impl_expr, &target_ctx));
                    }
                    self.pending_extend_implements
                        .push((target_id, resolved_impls));
                }
            }
        }
    }

    fn validate_implementations(&mut self, modules: &[Module]) {
        // 4a: Resolve struct/interface declared implements clauses.
        for module in modules {
            let module_id = match self.module_ids.get(&module.name) {
                Some(id) => *id,
                None => continue,
            };
            for item in &module.program.items {
                match item {
                    Item::Struct(decl) => {
                        let type_id = match self.find_type_id(module_id, &decl.name) {
                            Some(id) => id,
                            None => continue,
                        };
                        let type_label = format!("{}::{}", module.name, decl.name);
                        let scope = self
                            .type_generics
                            .get(&type_id)
                            .cloned()
                            .unwrap_or_default();
                        let resolved =
                            self.resolve_implements_list(module_id, &decl.implements, scope, &type_label);
                        self.hir
                            .structs
                            .get_mut(&type_id)
                            .unwrap()
                            .implements
                            .extend(resolved);
                    }
                    Item::Interface(decl) => {
                        let type_id = match self.find_type_id(module_id, &decl.name) {
                            Some(id) => id,
                            None => continue,
                        };
                        let type_label = format!("{}::{}", module.name, decl.name);
                        let scope = self
                            .type_generics
                            .get(&type_id)
                            .cloned()
                            .unwrap_or_default();
                        let resolved =
                            self.resolve_implements_list(module_id, &decl.implements, scope, &type_label);
                        self.hir
                            .interfaces
                            .get_mut(&type_id)
                            .unwrap()
                            .extends
                            .extend(resolved);
                    }
                    _ => {}
                }
            }
        }

        // 4a (cont.): drain extend-contributed implements.
        let pending = std::mem::take(&mut self.pending_extend_implements);
        for (target_id, resolveds) in pending {
            let target_label = self
                .hir
                .structs
                .get(&target_id)
                .map(|s| {
                    let mname = self.hir.modules[&s.module].name.clone();
                    format!("{}::{}", mname, s.name)
                })
                .unwrap_or_default();
            let mut filtered: Vec<(TypeId, Vec<ResolvedType>)> = Vec::new();
            for r in resolveds {
                match r {
                    ResolvedType::Interface(id, args) => filtered.push((id, args)),
                    ResolvedType::Null => {}
                    _ => {
                        self.errors.push(ValidationError::ImplementsNotInterface {
                            type_name: target_label.clone(),
                        });
                    }
                }
            }
            if let Some(s) = self.hir.structs.get_mut(&target_id) {
                s.implements.extend(filtered);
            }
        }

        // 4b: For each struct, verify it provides every required field and
        //     non-default method from each (transitively) implemented interface.
        let struct_ids: Vec<TypeId> = self.hir.structs.keys().cloned().collect();
        for sid in struct_ids {
            let struct_def = self.hir.structs[&sid].clone();
            let required = self.collect_required_interfaces(&struct_def.implements);
            for (interface_id, interface_args) in required {
                self.check_struct_implements_interface(&struct_def, interface_id, &interface_args);
            }
        }
    }

    fn resolve_implements_list(
        &mut self,
        module: ModuleId,
        list: &[TypeExpr],
        scope: Vec<(String, TypeParamId)>,
        type_label: &str,
    ) -> Vec<(TypeId, Vec<ResolvedType>)> {
        let ctx = TypeResolveCtx {
            module,
            generics: vec![scope],
            local_subst: HashMap::new(),
            allow_pointer: false,
        };
        let mut out = Vec::new();
        for expr in list {
            match self.resolve_type_expr(expr, &ctx) {
                ResolvedType::Interface(id, args) => out.push((id, args)),
                ResolvedType::Null => {}
                _ => {
                    self.errors.push(ValidationError::ImplementsNotInterface {
                        type_name: type_label.to_string(),
                    });
                }
            }
        }
        out
    }

    fn collect_required_interfaces(
        &self,
        direct: &[(TypeId, Vec<ResolvedType>)],
    ) -> Vec<(TypeId, Vec<ResolvedType>)> {
        let mut result: Vec<(TypeId, Vec<ResolvedType>)> = Vec::new();
        let mut visited: HashSet<TypeId> = HashSet::new();
        let mut stack: Vec<(TypeId, Vec<ResolvedType>)> = direct.to_vec();
        while let Some((id, args)) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }
            result.push((id, args.clone()));
            if let Some(iface) = self.hir.interfaces.get(&id) {
                let mut subst: HashMap<TypeParamId, ResolvedType> = HashMap::new();
                for (tp, arg) in iface.type_params.iter().zip(args.iter()) {
                    subst.insert(*tp, arg.clone());
                }
                for (parent_id, parent_args) in &iface.extends {
                    let substituted: Vec<ResolvedType> =
                        parent_args.iter().map(|a| substitute(a, &subst)).collect();
                    stack.push((*parent_id, substituted));
                }
            }
        }
        result
    }

    fn check_struct_implements_interface(
        &mut self,
        struct_def: &HirStruct,
        interface_id: TypeId,
        interface_args: &[ResolvedType],
    ) {
        let interface = match self.hir.interfaces.get(&interface_id).cloned() {
            Some(i) => i,
            None => return,
        };

        let module_name = self.hir.modules[&struct_def.module].name.clone();
        let type_label = format!("{}::{}", module_name, struct_def.name);
        let iface_module_name = self.hir.modules[&interface.module].name.clone();
        let iface_label = format!("{}::{}", iface_module_name, interface.name);

        let mut subst: HashMap<TypeParamId, ResolvedType> = HashMap::new();
        for (tp, arg) in interface.type_params.iter().zip(interface_args.iter()) {
            subst.insert(*tp, arg.clone());
        }

        for iface_field in &interface.fields {
            let expected_ty = substitute(&iface_field.ty, &subst);
            let struct_field = struct_def.fields.iter().find(|f| f.name == iface_field.name);
            match struct_field {
                None => {
                    self.errors.push(ValidationError::MissingInterfaceMember {
                        type_name: type_label.clone(),
                        interface: iface_label.clone(),
                        member: iface_field.name.clone(),
                    });
                }
                Some(sf) => {
                    if sf.ty != expected_ty || sf.is_pointer != iface_field.is_pointer {
                        self.errors.push(ValidationError::InterfaceMemberMismatch {
                            type_name: type_label.clone(),
                            interface: iface_label.clone(),
                            member: iface_field.name.clone(),
                        });
                    }
                }
            }
        }

        for &iface_method_id in &interface.methods {
            let iface_method = match self.hir.functions.get(&iface_method_id).cloned() {
                Some(m) => m,
                None => continue,
            };

            let has_default = self
                .pending_function_bodies
                .get(&iface_method_id)
                .is_some_and(|d| d.body.is_some());
            if has_default {
                continue;
            }

            let provider = self.find_struct_method(struct_def, &iface_method.name);
            match provider {
                None => {
                    self.errors.push(ValidationError::MissingInterfaceMember {
                        type_name: type_label.clone(),
                        interface: iface_label.clone(),
                        member: iface_method.name.clone(),
                    });
                }
                Some((provider_fn_id, extend_subst)) => {
                    let provider_fn = self.hir.functions.get(&provider_fn_id).cloned().unwrap();
                    if !signatures_match(&iface_method, &provider_fn, &subst, &extend_subst) {
                        self.errors.push(ValidationError::InterfaceMemberMismatch {
                            type_name: type_label.clone(),
                            interface: iface_label.clone(),
                            member: iface_method.name.clone(),
                        });
                    }
                }
            }
        }
    }

    /// Locate a struct method satisfying the given name. Returns the method's
    /// `FnId` and a substitution mapping from extend-generic ids onto the
    /// struct's own type params (empty for inline methods). Only inline
    /// methods and "universal" extends — those whose target args bijectively
    /// map the struct's type params — are considered, since only they apply
    /// to the struct's abstract self.
    fn find_struct_method(
        &self,
        struct_def: &HirStruct,
        name: &str,
    ) -> Option<(FnId, HashMap<TypeParamId, ResolvedType>)> {
        for fn_id in &struct_def.methods {
            if self.hir.functions.get(fn_id).map(|f| f.name.as_str()) == Some(name) {
                return Some((*fn_id, HashMap::new()));
            }
        }
        for (target_args, fn_id) in &struct_def.specialised_methods {
            if self.hir.functions.get(fn_id).map(|f| f.name.as_str()) != Some(name) {
                continue;
            }
            if let Some(mapping) = universal_extend_mapping(target_args, &struct_def.type_params) {
                return Some((*fn_id, mapping));
            }
        }
        None
    }

    fn resolve_function_signature(
        &mut self,
        fn_id: FnId,
        decl: &FunctionDecl,
        module: ModuleId,
        generics: Vec<Vec<(String, TypeParamId)>>,
        fn_label: &str,
    ) {
        let ctx = TypeResolveCtx {
            module,
            generics,
            local_subst: HashMap::new(),
            allow_pointer: decl.is_extern,
        };

        let mut params: Vec<HirParam> = Vec::new();
        let mut seen_params: HashSet<String> = HashSet::new();
        for p in &decl.params {
            if !seen_params.insert(p.name.clone()) {
                self.errors.push(ValidationError::DuplicateParam {
                    function: fn_label.to_string(),
                    param: p.name.clone(),
                });
                continue;
            }
            if p.is_pointer && !decl.is_extern {
                self.errors.push(ValidationError::PointerOutsideExtern {
                    location: format!("{}({})", fn_label, p.name),
                });
            }
            let ty = self.resolve_type_expr(&p.ty, &ctx);
            params.push(HirParam {
                name: p.name.clone(),
                ty,
                is_pointer: p.is_pointer,
            });
        }

        let return_type = match &decl.return_type {
            Some(ty) => self.resolve_type_expr(ty, &ctx),
            None => ResolvedType::Null,
        };

        let f = self.hir.functions.get_mut(&fn_id).unwrap();
        f.params = params;
        f.return_type = return_type;
    }

    fn mint_methods(
        &mut self,
        module_id: ModuleId,
        owner: TypeId,
        module_name: &str,
        owner_name: &str,
        methods: &[FunctionDecl],
        is_struct: bool,
    ) {
        let mut seen: HashSet<String> = HashSet::new();
        for method in methods {
            if !seen.insert(method.name.clone()) {
                self.errors.push(ValidationError::DuplicateMethod {
                    type_name: format!("{}::{}", module_name, owner_name),
                    method: method.name.clone(),
                });
                continue;
            }

            let fn_id = FnId(self.next_fn_id());
            self.hir.functions.insert(
                fn_id,
                HirFunction {
                    id: fn_id,
                    module: module_id,
                    name: method.name.clone(),
                    owner: Some(owner),
                    has_self: method.has_self_param,
                    type_params: Vec::new(),
                    params: Vec::new(),
                    return_type: ResolvedType::Null,
                    body: None,
                },
            );

            if is_struct {
                self.hir
                    .structs
                    .get_mut(&owner)
                    .unwrap()
                    .methods
                    .push(fn_id);
            } else {
                self.hir
                    .interfaces
                    .get_mut(&owner)
                    .unwrap()
                    .methods
                    .push(fn_id);
            }

            let scope_name = format!("{}::{}::{}", module_name, owner_name, method.name);
            let scope = self.mint_generics(&method.generics, scope_name);
            let ids: Vec<TypeParamId> = scope.iter().map(|(_, id)| *id).collect();
            self.hir.functions.get_mut(&fn_id).unwrap().type_params = ids;
            self.fn_generics.insert(fn_id, scope);

            self.pending_function_bodies.insert(fn_id, method.clone());
        }
    }

    fn mint_generics(
        &mut self,
        generics: &[GenericParam],
        scope_name: String,
    ) -> Vec<(String, TypeParamId)> {
        let mut scope = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for gp in generics {
            if !seen.insert(gp.name.clone()) {
                self.errors.push(ValidationError::DuplicateGenericParam {
                    scope: scope_name.clone(),
                    name: gp.name.clone(),
                });
                continue;
            }
            let id = TypeParamId(self.next_tp_id());
            self.hir.type_params.insert(
                id,
                TypeParamInfo {
                    id,
                    name: gp.name.clone(),
                    bounds: Vec::new(),
                },
            );
            scope.push((gp.name.clone(), id));
        }
        scope
    }

    /// Resolve bounds for a single-scope generic list (e.g. a struct/interface's
    /// own type params). Bounds within the same list can reference each other.
    fn resolve_generic_bounds(
        &mut self,
        module: ModuleId,
        ast_params: &[GenericParam],
        scope: Vec<(String, TypeParamId)>,
        scope_name: &str,
    ) {
        self.resolve_generic_bounds_with_scope(
            module,
            ast_params,
            vec![scope.clone()],
            &scope,
            scope_name,
        );
    }

    /// Resolve bounds for a generic list, given the full scope chain to use
    /// during resolution and the specific scope whose `TypeParamId`s should be
    /// updated with the resolved bounds.
    fn resolve_generic_bounds_with_scope(
        &mut self,
        module: ModuleId,
        ast_params: &[GenericParam],
        generics: Vec<Vec<(String, TypeParamId)>>,
        target_scope: &[(String, TypeParamId)],
        scope_name: &str,
    ) {
        for (i, gp) in ast_params.iter().enumerate() {
            let tp_id = match target_scope.get(i).map(|(_, id)| *id) {
                Some(id) => id,
                None => continue, // skipped (duplicate generic param)
            };
            let ctx = TypeResolveCtx {
                module,
                generics: generics.clone(),
                local_subst: HashMap::new(),
                allow_pointer: false,
            };
            let mut bounds: Vec<TypeId> = Vec::new();
            for bound_expr in &gp.bounds {
                let resolved = self.resolve_type_expr(bound_expr, &ctx);
                match resolved {
                    ResolvedType::Interface(id, _) => bounds.push(id),
                    ResolvedType::Null => {}
                    _ => {
                        self.errors.push(ValidationError::BoundNotInterface {
                            scope: scope_name.to_string(),
                            type_param: gp.name.clone(),
                        });
                    }
                }
            }
            if let Some(info) = self.hir.type_params.get_mut(&tp_id) {
                info.bounds = bounds;
            }
        }
    }

    fn resolve_fields(
        &mut self,
        module: ModuleId,
        type_id: TypeId,
        fields: &[ast::FieldDecl],
        is_extern: bool,
        module_name: &str,
        type_name: &str,
    ) -> Vec<HirField> {
        let scope = self
            .type_generics
            .get(&type_id)
            .cloned()
            .unwrap_or_default();
        let ctx = TypeResolveCtx {
            module,
            generics: vec![scope],
            local_subst: HashMap::new(),
            allow_pointer: is_extern,
        };

        let mut out = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for fd in fields {
            if !seen.insert(fd.name.clone()) {
                self.errors.push(ValidationError::DuplicateField {
                    type_name: format!("{}::{}", module_name, type_name),
                    field: fd.name.clone(),
                });
                continue;
            }
            if fd.is_pointer && !is_extern {
                self.errors.push(ValidationError::PointerOutsideExtern {
                    location: format!("{}::{}.{}", module_name, type_name, fd.name),
                });
            }
            let ty = self.resolve_type_expr(&fd.ty, &ctx);
            out.push(HirField {
                name: fd.name.clone(),
                ty,
                is_pointer: fd.is_pointer,
            });
        }
        out
    }

    fn resolve_type_expr(&mut self, expr: &TypeExpr, ctx: &TypeResolveCtx) -> ResolvedType {
        match expr {
            TypeExpr::Primitive(p) => resolve_primitive(p),
            TypeExpr::Union(types) => {
                let resolved: Vec<_> = types
                    .iter()
                    .map(|t| self.resolve_type_expr(t, ctx))
                    .collect();
                ResolvedType::Union(resolved)
            }
            TypeExpr::Named(name, args) => self.resolve_named(name, args, ctx),
            TypeExpr::Function(params, ret) => {
                let params: Vec<_> = params
                    .iter()
                    .map(|p| self.resolve_type_expr(p, ctx))
                    .collect();
                let ret = self.resolve_type_expr(ret, ctx);
                ResolvedType::Function(params, Box::new(ret))
            }
            TypeExpr::ExternFunction(params, ret) => {
                // ExternParam.is_pointer is currently lost — `ResolvedType`
                // has no per-param pointer slot. Acceptable for phase 2;
                // revisit if extern function pointer types ever flow through
                // type checking.
                let params: Vec<_> = params
                    .iter()
                    .map(|p| self.resolve_type_expr(&p.ty, ctx))
                    .collect();
                let ret = self.resolve_type_expr(ret, ctx);
                ResolvedType::Function(params, Box::new(ret))
            }
            TypeExpr::Pointer(inner) => {
                if !ctx.allow_pointer {
                    let module_name = self.hir.modules[&ctx.module].name.clone();
                    self.errors.push(ValidationError::PointerOutsideExtern {
                        location: format!("{} (type expression)", module_name),
                    });
                    return ResolvedType::Null;
                }
                // Pointer-ness is captured at the declaration level via the
                // `is_pointer` flag on `HirField`/`HirParam`. Nested pointer
                // types are flattened here.
                self.resolve_type_expr(inner, ctx)
            }
        }
    }

    fn resolve_named(
        &mut self,
        name: &str,
        args: &[TypeExpr],
        ctx: &TypeResolveCtx,
    ) -> ResolvedType {
        // 1. Local substitution (alias body inlining).
        if args.is_empty() {
            if let Some(ty) = ctx.local_subst.get(name) {
                return ty.clone();
            }
        }

        // 2. Generic scopes, innermost first (decision: shadow on collision).
        for scope in ctx.generics.iter().rev() {
            if let Some(id) = scope.iter().find(|(n, _)| n == name).map(|(_, id)| *id) {
                if !args.is_empty() {
                    let module_name = self.hir.modules[&ctx.module].name.clone();
                    self.errors.push(ValidationError::GenericArgsOnTypeParam {
                        module: module_name,
                        name: name.to_string(),
                    });
                    return ResolvedType::Null;
                }
                return ResolvedType::TypeParam(id);
            }
        }

        // 3. Module scope (direct then globs).
        let entry = self.lookup_in_scope(ctx.module, name);
        let entry = match entry {
            Some(e) => e,
            None => {
                let module_name = self.hir.modules[&ctx.module].name.clone();
                self.errors.push(ValidationError::UnknownType {
                    module: module_name,
                    name: name.to_string(),
                });
                return ResolvedType::Null;
            }
        };

        match entry {
            ScopeEntry::Type(id) => {
                let resolved_args: Vec<_> = args
                    .iter()
                    .map(|a| self.resolve_type_expr(a, ctx))
                    .collect();

                let expected_arity = if let Some(s) = self.hir.structs.get(&id) {
                    s.type_params.len()
                } else if let Some(i) = self.hir.interfaces.get(&id) {
                    i.type_params.len()
                } else {
                    return ResolvedType::Null;
                };

                if resolved_args.len() != expected_arity {
                    let module_name = self.hir.modules[&ctx.module].name.clone();
                    self.errors.push(ValidationError::GenericArityMismatch {
                        module: module_name,
                        name: name.to_string(),
                        expected: expected_arity,
                        actual: resolved_args.len(),
                    });
                    return ResolvedType::Null;
                }

                if self.hir.structs.contains_key(&id) {
                    ResolvedType::Struct(id, resolved_args)
                } else {
                    ResolvedType::Interface(id, resolved_args)
                }
            }
            ScopeEntry::Function(_) => {
                let module_name = self.hir.modules[&ctx.module].name.clone();
                self.errors
                    .push(ValidationError::ExpectedTypeFoundFunction {
                        module: module_name,
                        name: name.to_string(),
                    });
                ResolvedType::Null
            }
            ScopeEntry::Alias {
                source,
                name: alias_name,
            } => {
                let alias_decl = self
                    .module_aliases
                    .get(&source)
                    .and_then(|m| m.get(&alias_name))
                    .cloned();
                let alias_decl = match alias_decl {
                    Some(d) => d,
                    None => return ResolvedType::Null,
                };

                if alias_decl.generics.len() != args.len() {
                    let module_name = self.hir.modules[&ctx.module].name.clone();
                    self.errors.push(ValidationError::GenericArityMismatch {
                        module: module_name,
                        name: alias_name.clone(),
                        expected: alias_decl.generics.len(),
                        actual: args.len(),
                    });
                    return ResolvedType::Null;
                }

                let resolved_args: Vec<_> = args
                    .iter()
                    .map(|a| self.resolve_type_expr(a, ctx))
                    .collect();
                let mut local_subst = HashMap::new();
                for (param, arg) in alias_decl.generics.iter().zip(resolved_args.into_iter()) {
                    local_subst.insert(param.name.clone(), arg);
                }

                let alias_ctx = TypeResolveCtx {
                    module: source,
                    generics: Vec::new(),
                    local_subst,
                    allow_pointer: false,
                };
                self.resolve_type_expr(&alias_decl.ty, &alias_ctx)
            }
        }
    }

    fn lookup_in_scope(&self, module: ModuleId, name: &str) -> Option<ScopeEntry> {
        let scope = self.module_scopes.get(&module)?;
        if let Some(entry) = scope.direct.get(name) {
            return Some(entry.clone());
        }
        for source in &scope.globs {
            if let Some(other) = self.module_scopes.get(source) {
                if let Some(entry) = other.direct.get(name) {
                    return Some(entry.clone());
                }
            }
        }
        None
    }

    fn find_type_id(&self, module: ModuleId, name: &str) -> Option<TypeId> {
        match self.module_scopes.get(&module)?.direct.get(name)? {
            ScopeEntry::Type(id) => Some(*id),
            _ => None,
        }
    }

    fn find_fn_id(&self, module: ModuleId, name: &str) -> Option<FnId> {
        match self.module_scopes.get(&module)?.direct.get(name)? {
            ScopeEntry::Function(id) => Some(*id),
            _ => None,
        }
    }

    // util
    fn next_module_id(&mut self) -> u32 {
        let id = self.next_module_id;
        self.next_module_id += 1;
        id
    }

    fn next_type_id(&mut self) -> u32 {
        let id = self.next_type_id;
        self.next_type_id += 1;
        id
    }

    fn next_fn_id(&mut self) -> u32 {
        let id = self.next_fn_id;
        self.next_fn_id += 1;
        id
    }

    fn next_tp_id(&mut self) -> u32 {
        let id = self.next_tp_id;
        self.next_tp_id += 1;
        id
    }
}

fn substitute(ty: &ResolvedType, subst: &HashMap<TypeParamId, ResolvedType>) -> ResolvedType {
    match ty {
        ResolvedType::TypeParam(id) => subst.get(id).cloned().unwrap_or_else(|| ty.clone()),
        ResolvedType::Struct(id, args) => ResolvedType::Struct(
            *id,
            args.iter().map(|a| substitute(a, subst)).collect(),
        ),
        ResolvedType::Interface(id, args) => ResolvedType::Interface(
            *id,
            args.iter().map(|a| substitute(a, subst)).collect(),
        ),
        ResolvedType::Union(types) => {
            ResolvedType::Union(types.iter().map(|t| substitute(t, subst)).collect())
        }
        ResolvedType::Function(params, ret) => ResolvedType::Function(
            params.iter().map(|p| substitute(p, subst)).collect(),
            Box::new(substitute(ret, subst)),
        ),
        ResolvedType::Primitive(_) | ResolvedType::Null => ty.clone(),
    }
}

/// Returns Some(mapping) if `target_args` is a bijective list of distinct
/// `TypeParam(_)` values matching `struct_type_params` 1:1 by position.
/// The mapping sends each extend generic id to a `ResolvedType::TypeParam` of
/// the corresponding struct generic id, so the extend method's signature can
/// be re-expressed in the struct's own generic vocabulary.
fn universal_extend_mapping(
    target_args: &[ResolvedType],
    struct_type_params: &[TypeParamId],
) -> Option<HashMap<TypeParamId, ResolvedType>> {
    if target_args.len() != struct_type_params.len() {
        return None;
    }
    let mut seen: HashSet<TypeParamId> = HashSet::new();
    let mut mapping: HashMap<TypeParamId, ResolvedType> = HashMap::new();
    for (i, arg) in target_args.iter().enumerate() {
        match arg {
            ResolvedType::TypeParam(p) => {
                if !seen.insert(*p) {
                    return None;
                }
                mapping.insert(*p, ResolvedType::TypeParam(struct_type_params[i]));
            }
            _ => return None,
        }
    }
    Some(mapping)
}

fn signatures_match(
    iface_method: &crate::hir::HirFunction,
    provider: &crate::hir::HirFunction,
    iface_subst: &HashMap<TypeParamId, ResolvedType>,
    provider_subst: &HashMap<TypeParamId, ResolvedType>,
) -> bool {
    if iface_method.has_self != provider.has_self {
        return false;
    }
    if iface_method.params.len() != provider.params.len() {
        return false;
    }
    if iface_method.type_params.len() != provider.type_params.len() {
        return false;
    }

    let mut method_subst = iface_subst.clone();
    for (iface_tp, provider_tp) in iface_method
        .type_params
        .iter()
        .zip(provider.type_params.iter())
    {
        method_subst.insert(*iface_tp, ResolvedType::TypeParam(*provider_tp));
    }

    for (i_param, p_param) in iface_method.params.iter().zip(provider.params.iter()) {
        let expected_ty = substitute(&i_param.ty, &method_subst);
        let provided_ty = substitute(&p_param.ty, provider_subst);
        if expected_ty != provided_ty {
            return false;
        }
        if i_param.is_pointer != p_param.is_pointer {
            return false;
        }
    }

    let expected_ret = substitute(&iface_method.return_type, &method_subst);
    let provided_ret = substitute(&provider.return_type, provider_subst);
    expected_ret == provided_ret
}

fn resolve_primitive(p: &ast::PrimitiveType) -> ResolvedType {
    match p {
        ast::PrimitiveType::Int8 => ResolvedType::Primitive(PrimitiveType::Int8),
        ast::PrimitiveType::Int16 => ResolvedType::Primitive(PrimitiveType::Int16),
        ast::PrimitiveType::Int32 => ResolvedType::Primitive(PrimitiveType::Int32),
        ast::PrimitiveType::Int64 => ResolvedType::Primitive(PrimitiveType::Int64),
        ast::PrimitiveType::Uint8 => ResolvedType::Primitive(PrimitiveType::Uint8),
        ast::PrimitiveType::Uint16 => ResolvedType::Primitive(PrimitiveType::Uint16),
        ast::PrimitiveType::Uint32 => ResolvedType::Primitive(PrimitiveType::Uint32),
        ast::PrimitiveType::Uint64 => ResolvedType::Primitive(PrimitiveType::Uint64),
        ast::PrimitiveType::Float32 => ResolvedType::Primitive(PrimitiveType::Float32),
        ast::PrimitiveType::Float64 => ResolvedType::Primitive(PrimitiveType::Float64),
        ast::PrimitiveType::Bool => ResolvedType::Primitive(PrimitiveType::Bool),
        ast::PrimitiveType::String => ResolvedType::Primitive(PrimitiveType::String),
        ast::PrimitiveType::Char => ResolvedType::Primitive(PrimitiveType::Char),
        ast::PrimitiveType::Null => ResolvedType::Null,
    }
}
