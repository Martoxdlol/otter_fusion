pub mod errors;
pub mod pass_extend;
pub mod pass_interface;
pub mod pass_register;
pub mod pass_typecheck;
pub mod resolve;
pub mod scope;

use std::collections::{HashMap, HashSet};

use crate::ast::{Block, ExtendDecl, Module};
use crate::hir::{self, FnId, Hir, ModuleId, TypeId, TypeParamId};

use errors::ValidationError;
use scope::{ModuleScope, TypeAliasInfo, TypeKind, VisibleNames};

pub struct Validator {
    modules: Vec<Module>,
    hir: Hir,

    // ID counters
    next_module_id: u32,
    next_type_id: u32,
    next_fn_id: u32,
    next_tp_id: u32,

    // Built during passes 0-2
    pub(crate) module_scopes: HashMap<ModuleId, ModuleScope>,
    pub(crate) visible_names: HashMap<ModuleId, VisibleNames>,
    pub(crate) module_name_to_id: HashMap<String, ModuleId>,
    pub(crate) type_kinds: HashMap<TypeId, TypeKind>,

    // Type aliases
    pub(crate) type_aliases: HashMap<TypeId, TypeAliasInfo>,
    pub(crate) alias_expanding: HashSet<TypeId>,
    pub(crate) resolved_alias_bodies: HashMap<TypeId, hir::ResolvedType>,

    // Active type param scope (set contextually during resolution)
    pub(crate) type_param_scope: HashMap<String, TypeParamId>,

    // Extend blocks deferred to pass 3
    pub(crate) pending_extends: Vec<(ModuleId, ExtendDecl)>,

    // AST bodies kept for pass 5
    pub(crate) ast_fn_bodies: HashMap<FnId, Block>,

    // Accumulated errors
    pub(crate) errors: Vec<ValidationError>,

    // Built-in type IDs
    pub(crate) list_type_id: Option<TypeId>,
    pub(crate) map_type_id: Option<TypeId>,
    pub(crate) list_tp_id: Option<TypeParamId>,
    pub(crate) map_key_tp_id: Option<TypeParamId>,
    pub(crate) map_val_tp_id: Option<TypeParamId>,

    // Std module ID
    pub(crate) std_module_id: Option<ModuleId>,
}

impl Validator {
    pub fn new(modules: Vec<Module>) -> Self {
        Self {
            modules,
            hir: Hir::default(),

            next_module_id: 0,
            next_type_id: 0,
            next_fn_id: 0,
            next_tp_id: 0,

            module_scopes: HashMap::new(),
            visible_names: HashMap::new(),
            module_name_to_id: HashMap::new(),
            type_kinds: HashMap::new(),

            type_aliases: HashMap::new(),
            alias_expanding: HashSet::new(),
            resolved_alias_bodies: HashMap::new(),

            type_param_scope: HashMap::new(),

            pending_extends: Vec::new(),

            ast_fn_bodies: HashMap::new(),

            errors: Vec::new(),

            list_type_id: None,
            map_type_id: None,
            list_tp_id: None,
            map_key_tp_id: None,
            map_val_tp_id: None,

            std_module_id: None,
        }
    }

    pub fn validate(mut self) -> Result<Hir, Vec<ValidationError>> {
        // Register the std module
        self.register_std_module();

        // Step 0: register modules and resolve imports
        self.pass_register_modules();
        if !self.errors.is_empty() {
            return Err(self.errors);
        }

        // Step 1: register type shapes (methods, implements clauses)
        self.pass_register_type_shapes();
        if !self.errors.is_empty() {
            return Err(self.errors);
        }

        // Step 2: register type params, resolve field types and signatures
        self.pass_register_type_params();
        if !self.errors.is_empty() {
            return Err(self.errors);
        }

        // Step 3: merge extend blocks
        self.pass_merge_extends();
        if !self.errors.is_empty() {
            return Err(self.errors);
        }

        // Step 4: validate interface implementations
        self.pass_validate_interfaces();
        if !self.errors.is_empty() {
            return Err(self.errors);
        }

        // Step 5: type-check function bodies
        self.pass_typecheck_bodies();
        if !self.errors.is_empty() {
            return Err(self.errors);
        }

        Ok(self.hir)
    }

    /// Register the "std" module as a virtual module that users import from explicitly.
    /// List and Map types are still tracked internally for literal type inference.
    fn register_std_module(&mut self) {
        let std_id = self.next_module_id();
        self.std_module_id = Some(std_id);
        self.module_name_to_id.insert("std".to_string(), std_id);

        let mut std_scope = ModuleScope::default();

        // List<T>
        let list_id = self.next_type_id();
        let list_tp = self.next_tp_id();
        self.list_type_id = Some(list_id);
        self.list_tp_id = Some(list_tp);
        self.type_kinds.insert(list_id, TypeKind::Struct);
        self.hir.type_params.insert(
            list_tp,
            hir::TypeParamInfo {
                id: list_tp,
                name: "T".to_string(),
                bounds: vec![],
            },
        );
        self.hir.structs.insert(
            list_id,
            hir::HirStruct {
                id: list_id,
                module: std_id,
                name: "List".to_string(),
                type_params: vec![list_tp],
                fields: vec![],
                methods: vec![],
                implements: vec![],
            },
        );
        std_scope.types.insert("List".to_string(), list_id);

        // Map<K, V>
        let map_id = self.next_type_id();
        let map_k_tp = self.next_tp_id();
        let map_v_tp = self.next_tp_id();
        self.map_type_id = Some(map_id);
        self.map_key_tp_id = Some(map_k_tp);
        self.map_val_tp_id = Some(map_v_tp);
        self.type_kinds.insert(map_id, TypeKind::Struct);
        self.hir.type_params.insert(
            map_k_tp,
            hir::TypeParamInfo {
                id: map_k_tp,
                name: "K".to_string(),
                bounds: vec![],
            },
        );
        self.hir.type_params.insert(
            map_v_tp,
            hir::TypeParamInfo {
                id: map_v_tp,
                name: "V".to_string(),
                bounds: vec![],
            },
        );
        self.hir.structs.insert(
            map_id,
            hir::HirStruct {
                id: map_id,
                module: std_id,
                name: "Map".to_string(),
                type_params: vec![map_k_tp, map_v_tp],
                fields: vec![],
                methods: vec![],
                implements: vec![],
            },
        );
        std_scope.types.insert("Map".to_string(), map_id);

        // print(value: str): null
        let print_id = self.next_fn_id();
        self.hir.functions.insert(
            print_id,
            hir::HirFunction {
                id: print_id,
                module: std_id,
                name: "print".to_string(),
                owner: None,
                has_self: false,
                type_params: vec![],
                params: vec![hir::HirParam {
                    name: "value".to_string(),
                    ty: hir::ResolvedType::Primitive(hir::PrimitiveType::String),
                }],
                return_type: hir::ResolvedType::Null,
                body: None,
            },
        );
        std_scope.functions.insert("print".to_string(), print_id);

        // Register std as a proper HirModule
        self.hir.modules.insert(
            std_id,
            hir::HirModule {
                id: std_id,
                name: "std".to_string(),
                structs: vec![list_id, map_id],
                interfaces: vec![],
                functions: vec![print_id],
                imports: vec![],
            },
        );

        self.module_scopes.insert(std_id, std_scope);

        // Std module's own visible names (just its own scope)
        let mut std_visible = VisibleNames::default();
        let scope = self.module_scopes.get(&std_id).unwrap();
        for (name, &id) in &scope.types {
            std_visible.types.insert(name.clone(), id);
        }
        for (name, &id) in &scope.functions {
            std_visible.functions.insert(name.clone(), id);
        }
        self.visible_names.insert(std_id, std_visible);
    }

    // ID allocation
    pub(crate) fn next_module_id(&mut self) -> ModuleId {
        let id = ModuleId(self.next_module_id);
        self.next_module_id += 1;
        id
    }

    pub(crate) fn next_type_id(&mut self) -> TypeId {
        let id = TypeId(self.next_type_id);
        self.next_type_id += 1;
        id
    }

    pub(crate) fn next_fn_id(&mut self) -> FnId {
        let id = FnId(self.next_fn_id);
        self.next_fn_id += 1;
        id
    }

    pub(crate) fn next_tp_id(&mut self) -> TypeParamId {
        let id = TypeParamId(self.next_tp_id);
        self.next_tp_id += 1;
        id
    }
}
