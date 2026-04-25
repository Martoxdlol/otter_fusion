use std::collections::{HashMap, HashSet};

use crate::{
    ast::{FunctionDecl, ImportSymbol, ImportSymbols, Item, Module, TypeAliasDecl},
    hir::{
        FnId, Hir, HirFunction, HirImport, HirImportSymbol, HirInterface, HirModule, HirStruct,
        ModuleId, ResolvedType, TypeId,
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

pub struct Validator {
    modules: Vec<Module>,
    hir: Hir,
    errors: Vec<ValidationError>,

    module_ids: HashMap<String, ModuleId>,
    module_scopes: HashMap<ModuleId, ModuleScope>,
    /// Side table of type aliases — never materialized in HIR; inlined at
    /// resolution time in later phases.
    module_aliases: HashMap<ModuleId, HashMap<String, TypeAliasDecl>>,

    pending_named_imports: Vec<PendingNamedImport>,
    /// Top-level functions whose `HirFunction` was reserved in phase 1 but
    /// whose signature is filled in phase 2.
    pending_functions: Vec<(FnId, ModuleId, FunctionDecl)>,

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
            pending_functions: Vec::new(),
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
        // step 2: register type params
        // step 3: merge extend blocks
        // step 4: validate interface implementations
        // step 5: validate function bodies (type check, resolve members, etc)

        if !self.errors.is_empty() {
            return Err(self.errors);
        }

        todo!("phases 2-5 not yet implemented")
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
                None => continue, // duplicate module — already errored
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
        // 1a: For each module, mint ids for structs/interfaces/functions and
        //     record aliases. Detect intra-module name collisions and build
        //     the per-module direct scope.
        for module in modules {
            let module_id = match self.module_ids.get(&module.name) {
                Some(id) => *id,
                None => continue,
            };

            let mut scope = ModuleScope::default();

            // Glob sources were written to HirModule.imports in phase 0.
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
                        // Skeleton: signature is filled in phase 2. `return_type`
                        // is a placeholder until then.
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
                        self.pending_functions.push((id, module_id, decl.clone()));
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
                        // Two named imports landing on the same local name.
                        self.errors.push(ValidationError::DuplicateImport {
                            in_module: importer_name.clone(),
                            local_name: local.clone(),
                        });
                        continue;
                    }
                    // Local definition exists with the same name; per the
                    // shadowing rule the local wins. Drop the import from
                    // the scope but still record it in HIR for downstream
                    // tooling.
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
