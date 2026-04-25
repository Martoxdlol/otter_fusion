use std::collections::HashMap;

use crate::{
    ast::{ImportSymbol, ImportSymbols, Item, Module},
    hir::{Hir, HirImport, HirModule, ModuleId},
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
}

/// Per-module record of named imports that we cannot resolve until type and
/// function names are registered (phase 1).
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
    pending_named_imports: Vec<PendingNamedImport>,

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
            pending_named_imports: Vec::new(),
            next_module_id: 0,
            next_type_id: 0,
            next_fn_id: 0,
            next_tp_id: 0,
        }
    }

    pub fn validate(mut self) -> Result<Hir, Vec<ValidationError>> {
        // step 0: register modules & resolve imports
        self.register_modules_and_imports();

        // step 1: register type names
        // step 2: register type params
        // step 3: merge extend blocks
        // step 4: validate interface implementations
        // step 5: validate function bodies (type check, resolve members, etc)

        if !self.errors.is_empty() {
            return Err(self.errors);
        }

        todo!("phases 1-5 not yet implemented")
    }

    fn register_modules_and_imports(&mut self) {
        // 0a: mint a ModuleId per module, build the name -> id map and HIR
        // module skeletons. Modules with duplicate names are reported and
        // skipped so they don't shadow the first occurrence.
        let module_names: Vec<String> = self.modules.iter().map(|m| m.name.clone()).collect();

        for name in module_names {
            if self.module_ids.contains_key(&name) {
                self.errors
                    .push(ValidationError::DuplicateModule { name });
                continue;
            }

            let id = ModuleId(self.next_module_id());
            self.module_ids.insert(name.clone(), id);
            self.hir.modules.insert(
                id,
                HirModule {
                    id,
                    name,
                    structs: Vec::new(),
                    interfaces: Vec::new(),
                    functions: Vec::new(),
                    imports: Vec::new(),
                },
            );
        }

        // 0b: walk imports per module. Glob imports go straight to HIR.
        // Named imports are deferred — `HirImportSymbol` requires classifying
        // each symbol as Type/Function, but type/function names won't exist
        // until phase 1.
        let modules = std::mem::take(&mut self.modules);

        for module in &modules {
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

        self.modules = modules;
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
