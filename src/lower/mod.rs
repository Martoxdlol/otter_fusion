pub mod builder;
pub mod closures;
pub mod expr;
pub mod ffi;
pub mod layout;
pub mod mangling;
pub mod mono;
pub mod stmt;
pub mod subst;
pub mod unions;
pub mod vtables;

use crate::hir::{FnId, Hir, HirBlock, ResolvedType, TypeId};
use crate::lower::builder::FnBuilder;
use crate::lower::layout::compute_struct_layout;
use crate::lower::mangling::{name_function, name_struct};
use crate::lower::subst::Subst;
use crate::mir::*;
use std::collections::{HashMap, VecDeque};

pub struct Lower {
    pub hir: Hir,
    pub mir: MirProgram,

    // Estas son las funciones y tipos monomorfizados que ya hemos generado.
    // (Genéricos resueltos, en el proximo tp no va a haber genéricos de mierda!!!!)
    // Las claves incluyen los argumentos de tipo concretos para distinguir.
    pub mono_fns: HashMap<(FnId, Vec<ResolvedType>), MirFnId>,
    pub mono_types: HashMap<(TypeId, Vec<ResolvedType>), MirTypeId>,
    pub mono_unions: HashMap<Vec<ResolvedType>, MirTypeId>,
    // pub mono_closures: HashMap<ClosureKey, MirTypeId>,

    //
    pub entry_mir_fn: Option<MirFnId>, // el main monomorfizado, para arrancar la ejecución

    // monomorfizar es atravezar un grafo con DFS o BFS
    pub worklist: VecDeque<MonoTask>,
    next_fn_id: u32,
    next_type_id: u32,

    // Ids generados para los tipos cores del lenguaje (lo necesitamos para transformar syntaxis del lenguaje)
    pub iterator_interface: Option<TypeId>, // of:core -> Iterator<T>
    pub entry_struct: Option<TypeId>,       // of:core -> Entry<K,V>

    /// HIR return type of the function currently being lowered, post-subst.
    /// Read by `Return` to coerce the operand into the declared type.
    pub current_return_type: Option<ResolvedType>,
}

#[derive(Debug, Clone)]
pub enum MonoTask {
    /// Lower a function body for a given concrete type-arg list.
    Function {
        fn_id: FnId,
        type_args: Vec<ResolvedType>,
        mir_id: MirFnId,
    },
    /// Build a vtable for a (struct instance, interface instance) pair.
    /// Both sides are already monomorphized at the time this is enqueued.
    VTable {
        struct_mir: MirTypeId,
        interface_mir: MirTypeId,
        struct_hir: TypeId,
        struct_args: Vec<ResolvedType>,
        iface_hir: TypeId,
        iface_args: Vec<ResolvedType>,
    },
}

impl Lower {
    pub fn new(hir: Hir) -> Self {
        let mut s = Self {
            hir,
            mir: MirProgram::new(),
            mono_fns: HashMap::new(),
            mono_types: HashMap::new(),
            mono_unions: HashMap::new(),
            // mono_closures: HashMap::new(),
            worklist: VecDeque::new(),
            next_fn_id: 0,
            next_type_id: 0,
            iterator_interface: None,
            entry_struct: None,
            entry_mir_fn: None,
            current_return_type: None,
        };
        s.locate_core_types();
        s
    }

    pub fn lower(mut self) -> Result<MirProgram, String> {
        let main_id = self
            .find_main()
            .ok_or_else(|| "no `main` function found".to_string())?;

        // obtener la función con el genérico resuelto
        // esto mete cosas en la cola
        let mir_id = self.mono_fn(main_id, vec![]);
        self.entry_mir_fn = Some(mir_id); // insertar como main

        // empezamos a recorrer el grafo (Y si, es BFS)
        while let Some(task) = self.worklist.pop_front() {
            match task {
                MonoTask::Function {
                    fn_id,
                    type_args,
                    mir_id,
                } => {
                    let f = self.lower_function(fn_id, type_args, mir_id);
                    self.mir.functions.insert(mir_id, f);
                }
                MonoTask::VTable {
                    struct_mir,
                    interface_mir,
                    struct_hir,
                    struct_args,
                    iface_hir,
                    iface_args,
                } => {
                    self.process_vtable_task(
                        struct_mir,
                        interface_mir,
                        struct_hir,
                        struct_args,
                        iface_hir,
                        iface_args,
                    );
                }
            }
        }
        self.mir.entry = self
            .entry_mir_fn
            .ok_or_else(|| "internal: entry not set".to_string())?;
        Ok(self.mir)
    }

    // get id for and record cache entry for a function with specific type args
    pub fn mono_fn(&mut self, fn_id: FnId, type_args: Vec<ResolvedType>) -> MirFnId {
        // calculate key (to check/set cache)
        let key = (fn_id, type_args.clone());
        if let Some(&mid) = self.mono_fns.get(&key) {
            // use cache
            return mid;
        }
        let mid = self.alloc_fn_id();
        self.mono_fns.insert(key, mid);
        // add to queue for lowering
        self.worklist.push_back(MonoTask::Function {
            fn_id,
            type_args,
            mir_id: mid,
        });
        mid
    }

    pub fn lower_function(
        &mut self,
        fn_id: FnId,
        type_args: Vec<ResolvedType>,
        mir_id: MirFnId,
    ) -> MirFunction {
        let f = self.hir.functions[&fn_id].clone();

        // extern function: import if body-less, export if body-bearing.
        // Imports → forward-declared stubs (Abi::Extern, no blocks).
        // Exports → fully lowered body, but Abi::Extern and no `self`.
        if f.is_extern {
            let body_present = f
                .body
                .as_ref()
                .map(|b| !(b.statements.is_empty() && b.returns.is_none()))
                .unwrap_or(false);
            if !body_present {
                return self.lower_extern_import(fn_id, type_args, mir_id);
            }
            return self.lower_extern_export(fn_id, type_args, mir_id);
        }

        // Build the substitution. Callers pass type_args =
        //   [owner_type_args..., method_resolved_args..., explicit_args...].
        // For methods we bind the owner's type params from the first
        // segment, then the method's own from the rest.
        let mut mappings: std::collections::HashMap<crate::hir::TypeParamId, ResolvedType> =
            std::collections::HashMap::new();

        let owner_arity = f
            .owner
            .map(|oid| self.hir.structs[&oid].type_params.len())
            .unwrap_or(0);
        if owner_arity > 0 {
            let owner_params = self.hir.structs[&f.owner.unwrap()].type_params.clone();
            for (tp, ta) in owner_params.iter().zip(type_args.iter()) {
                mappings.insert(*tp, ta.clone());
            }
            // Universal extend: each `target_args[i]` is a TypeParam alias
            // of the struct's i-th param. Bind those aliases to the same
            // concrete type so body references through them resolve.
            if let Some(owner_id) = f.owner {
                let owner_struct = self.hir.structs[&owner_id].clone();
                for (target_args, sm_id) in &owner_struct.specialised_methods {
                    if *sm_id != fn_id {
                        continue;
                    }
                    if target_args.len() != owner_arity {
                        continue;
                    }
                    let all_tp = target_args
                        .iter()
                        .all(|a| matches!(a, ResolvedType::TypeParam(_)));
                    if !all_tp {
                        continue;
                    }
                    for (a, ta) in target_args.iter().zip(type_args.iter()) {
                        if let ResolvedType::TypeParam(id) = a {
                            mappings.insert(*id, ta.clone());
                        }
                    }
                    break;
                }
            }
        }
        // Method's own type params consume the rest of type_args.
        let rest = type_args.iter().skip(owner_arity);
        for (tp, ta) in f.type_params.iter().zip(rest) {
            mappings.insert(*tp, ta.clone());
        }

        let subst = Subst { mappings };

        // Concrete return type for `Return`-statement coercion.
        let prev_ret = self.current_return_type.replace(subst.apply(&f.return_type));

        let ret_ty = self.lower_type(&f.return_type, &subst);

        let name = name_function(&self.hir, fn_id, &type_args);

        let abi = Abi::Otter;

        let mut b = FnBuilder::new(mir_id, name, abi, ret_ty);

        // Definir y bindear parámetros
        for p in &f.params {
            let ty = self.lower_type(&p.ty, &subst);
            let local = b.new_local(Some(p.name.clone()), ty);
            b.params.push(local);
            b.bind(p.name.clone(), local);
        }

        // handle self param
        if f.has_self {
            // Member of struct
            let owner = f.owner.expect("has_self => owner present");
            // Owner's concrete type args come from the method's enclosing
            // monomorphization. For inherent methods the owner's type_params
            // are a prefix of the function's type_params, so we recover
            // them from `subst`.
            let st = &self.hir.structs[&owner];
            let owner_args: Vec<ResolvedType> = st
                .type_params
                .iter()
                .map(|tp| {
                    subst
                        .mappings
                        .get(tp)
                        .cloned()
                        .expect("specialized methods not implemented yet")
                })
                .collect();
            let mir_struct = self.get_or_create_struct(owner, owner_args);
            let self_ty = MirType::ManagedRef(mir_struct);
            let local = b.new_local(Some("self".to_string()), self_ty);
            // self is the first parameter, before any user-declared ones.
            // We pushed user params already, so insert at index 0 and rebind.
            b.params.insert(0, local);
            b.bind("self".to_string(), local);
        }

        let body = f.body.as_ref().expect("body-having function (extern handled above)");
        let trailing_src_ty = body.returns.as_ref().map(|e| subst.apply(&e.ty));
        let trailing: Option<Operand> = self.lower_block(body, &subst, &mut b);

        // implicit return of the trailing expression — widen to the declared
        // return type if needed (e.g. trailing `42` in a `-> i32 | null` fn).
        let ret_target = self.current_return_type.clone().expect("set above");
        let term = match (trailing, trailing_src_ty) {
            (Some(op), Some(src)) => {
                let coerced = self.coerce_to(op, &src, &ret_target, &mut b);
                Terminator::Return(Some(coerced))
            }
            (Some(op), None) => Terminator::Return(Some(op)),
            (None, _) => Terminator::Return(None),
        };
        // An early `return` inside the body would have set a terminator
        // and switched to a fresh unreachable block.
        b.terminate_if_open(term);

        self.current_return_type = prev_ret;
        b.finish()
    }

    /// Body-less function = extern import. Lowers to an `Abi::Extern`
    /// MirFunction with declared params, lowered return type, and no blocks
    /// — codegen treats this as a forward declaration of the C symbol.
    /// The name stays unmangled so the linker matches the C side.
    fn lower_extern_import(
        &mut self,
        fn_id: FnId,
        type_args: Vec<ResolvedType>,
        mir_id: MirFnId,
    ) -> MirFunction {
        let f = self.hir.functions[&fn_id].clone();
        let subst = Subst::new(f.type_params.clone(), type_args);
        let ret_ty = self.lower_type(&f.return_type, &subst);
        let name = f.name.clone();
        let mut b = FnBuilder::new(mir_id, name, Abi::Extern, ret_ty);
        for p in &f.params {
            let inner = self.lower_type(&p.ty, &subst);
            let ty = if p.is_pointer {
                MirType::Pointer(Box::new(inner))
            } else {
                inner
            };
            let local = b.new_local(Some(p.name.clone()), ty);
            b.params.push(local);
        }
        let mut out = b.finish();
        out.blocks.clear();
        out
    }

    /// Body-bearing extern = exported callback. Same shape as an Otter
    /// function except for `Abi::Extern`, no implicit `self`, no generics
    /// (validator-enforced), and pointer-aware param typing.
    fn lower_extern_export(
        &mut self,
        fn_id: FnId,
        type_args: Vec<ResolvedType>,
        mir_id: MirFnId,
    ) -> MirFunction {
        let f = self.hir.functions[&fn_id].clone();
        let subst = Subst::new(f.type_params.clone(), type_args);

        let prev_ret = self
            .current_return_type
            .replace(subst.apply(&f.return_type));
        let ret_ty = self.lower_type(&f.return_type, &subst);

        let mut b = FnBuilder::new(mir_id, f.name.clone(), Abi::Extern, ret_ty);
        for p in &f.params {
            let inner = self.lower_type(&p.ty, &subst);
            let ty = if p.is_pointer {
                MirType::Pointer(Box::new(inner))
            } else {
                inner
            };
            let local = b.new_local(Some(p.name.clone()), ty);
            b.params.push(local);
            b.bind(p.name.clone(), local);
        }

        let body = f
            .body
            .as_ref()
            .expect("extern export must have a body — caller checked");
        let trailing_src_ty = body.returns.as_ref().map(|e| subst.apply(&e.ty));
        let trailing = self.lower_block(body, &subst, &mut b);

        let ret_target = self.current_return_type.clone().expect("set above");
        let term = match (trailing, trailing_src_ty) {
            (Some(op), Some(src)) => {
                let coerced = self.coerce_to(op, &src, &ret_target, &mut b);
                Terminator::Return(Some(coerced))
            }
            (Some(op), None) => Terminator::Return(Some(op)),
            (None, _) => Terminator::Return(None),
        };
        b.terminate_if_open(term);

        self.current_return_type = prev_ret;
        b.finish()
    }

    pub fn lower_block(
        &mut self,
        block: &HirBlock,
        subst: &Subst,
        b: &mut FnBuilder,
    ) -> Option<Operand> {
        b.push_scope();
        for s in &block.statements {
            self.lower_stmt(s, subst, b);
        }
        let trailing = block.returns.as_ref().map(|e| self.lower_expr(e, subst, b));
        b.pop_scope();
        trailing
    }

    pub fn lower_type(&mut self, ty: &ResolvedType, subst: &Subst) -> MirType {
        let ty = subst.apply(ty); // remove every TypeParam first
        self.lower_concrete_type(&ty)
    }

    pub fn lower_concrete_type(&mut self, ty: &ResolvedType) -> MirType {
        match ty {
            ResolvedType::Primitive(p) => MirType::Primitive(p.clone()),

            ResolvedType::Struct(hir_id, args) => {
                let _is_extern = self.hir.structs[hir_id].is_extern;
                let mid = self.get_or_create_struct(*hir_id, args.clone());
                // TODO: Hacer algo con is_extern
                MirType::ManagedRef(mid)
            }

            ResolvedType::Interface(hir_id, args) => {
                let mid = self.get_or_create_interface(*hir_id, args.clone());
                MirType::ManagedRef(mid)
            }

            ResolvedType::Union(variants) => {
                // T | null where T is managed → NullableRef (no MirTypeDef).
                if let Some(t_mid) = self.try_nullable_ref(variants) {
                    MirType::NullableRef(t_mid)
                } else {
                    let uid = self.get_or_create_union(variants.clone());
                    MirType::Union(uid)
                }
            }

            ResolvedType::Function(args, ret) => {
                let arg_mirs: Vec<MirType> = args
                    .iter()
                    .map(|a| self.lower_concrete_type(a))
                    .collect();
                let ret_mir = self.lower_concrete_type(ret);
                MirType::FnPtr(arg_mirs, Box::new(ret_mir))
            }

            ResolvedType::Null => MirType::Unit,

            ResolvedType::TypeParam(id) => {
                panic!("non-concrete TypeParam({:?}) reached lowering", id);
            }
        }
    }

    pub fn get_or_create_interface(
        &mut self,
        hir_id: TypeId,
        args: Vec<ResolvedType>,
    ) -> MirTypeId {
        let key = (hir_id, args);
        if let Some(&mid) = self.mono_types.get(&key) {
            return mid;
        }
        let mid = self.alloc_type_id();
        self.mono_types.insert(key.clone(), mid);
        self.mir.types.insert(
            mid,
            MirTypeDef::Interface {
                name: String::new(),
                fields: vec![],
                method_slots: vec![],
                extends: vec![],
            },
        );

        let (hir_id, args) = key;
        let def = self.build_interface_def(hir_id, &args);
        self.mir.types.insert(mid, def);
        mid
    }

    fn build_interface_def(&mut self, hir_id: TypeId, args: &[ResolvedType]) -> MirTypeDef {
        let i = self.hir.interfaces[&hir_id].clone();
        let subst = Subst::new(i.type_params.clone(), args.to_vec());

        let fields: Vec<MirField> = i
            .fields
            .iter()
            .map(|f| MirField {
                name: f.name.clone(),
                ty: self.lower_type(&f.ty, &subst),
                offset: 0,
            })
            .collect();

        let method_slots: Vec<InterfaceMethodSlot> = i
            .methods
            .iter()
            .map(|fn_id| {
                let f = &self.hir.functions[fn_id].clone();
                let params = f
                    .params
                    .iter()
                    .map(|p| self.lower_type(&p.ty, &subst))
                    .collect();
                let return_type = self.lower_type(&f.return_type, &subst);
                InterfaceMethodSlot {
                    name: f.name.clone(),
                    params,
                    return_type,
                }
            })
            .collect();

        let parents = i.extends.clone();
        let mut extends: Vec<MirTypeId> = Vec::with_capacity(parents.len());
        for (pid, pargs) in parents {
            let pargs_concrete: Vec<ResolvedType> =
                pargs.iter().map(|a| subst.apply(a)).collect();
            extends.push(self.get_or_create_interface(pid, pargs_concrete));
        }

        let name = {
            let module = &self.hir.modules[&i.module];
            if args.is_empty() {
                format!("{}::{}", module.name, i.name)
            } else {
                let inner: Vec<String> = args
                    .iter()
                    .map(|a| crate::lower::mangling::name_type(&self.hir, a, &[]))
                    .collect();
                format!("{}::{}<{}>", module.name, i.name, inner.join(","))
            }
        };

        MirTypeDef::Interface {
            name,
            fields,
            method_slots,
            extends,
        }
    }

    pub fn get_or_create_struct(&mut self, hir_id: TypeId, args: Vec<ResolvedType>) -> MirTypeId {
        // struct cache key
        let key = (hir_id, args);

        // use cache if available
        if let Some(&mid) = self.mono_types.get(&key) {
            return mid;
        }
        // Allocate id
        let mid = self.alloc_type_id();
        self.mono_types.insert(key.clone(), mid);

        // we do this because we may need to reference it before building it
        self.mir.types.insert(
            mid,
            MirTypeDef::Struct {
                name: String::new(),
                fields: vec![],
                layout: Layout { size: 0, align: 1 },
                kind: StructKind::Managed,
            },
        ); // insert empty struct

        let (hir_id, args) = key;
        let def = self.build_struct_def(hir_id, &args);
        self.mir.types.insert(mid, def);
        mid
    }

    pub fn build_struct_def(&mut self, hir_id: TypeId, args: &[ResolvedType]) -> MirTypeDef {
        let s = self.hir.structs[&hir_id].clone();
        let subst = Subst::new(s.type_params.clone(), args.to_vec());

        let field_types: Vec<MirType> = s
            .fields
            .iter()
            .map(|f| {
                let inner = self.lower_type(&f.ty, &subst);
                if f.is_pointer && s.is_extern {
                    MirType::Pointer(Box::new(inner))
                } else {
                    inner
                }
            })
            .collect();

        let (offsets, layout) = compute_struct_layout(&self.mir, &field_types);

        let fields: Vec<MirField> = s
            .fields
            .iter()
            .zip(field_types)
            .zip(offsets)
            .map(|((hf, ty), offset)| MirField {
                name: hf.name.clone(),
                ty,
                offset,
            })
            .collect();

        let kind = if s.is_extern {
            StructKind::Extern
        } else {
            StructKind::Managed
        };

        // module + name + (args)
        let name = name_struct(&self.hir, &s, args);

        let def = MirTypeDef::Struct {
            name,
            fields,
            layout,
            kind,
        };

        // Enqueue vtable construction for every interface this struct
        // implements. The struct itself is already cached in `mono_types`
        // (placeholder inserted before build_struct_def runs).
        let struct_mir = self.mono_types[&(hir_id, args.to_vec())];
        let implements: Vec<(TypeId, Vec<ResolvedType>)> = s.implements.clone();
        for (iface_id, iface_args) in implements {
            let iface_concrete: Vec<ResolvedType> =
                iface_args.iter().map(|a| subst.apply(a)).collect();
            let iface_mir = self.get_or_create_interface(iface_id, iface_concrete.clone());
            self.worklist.push_back(MonoTask::VTable {
                struct_mir,
                interface_mir: iface_mir,
                struct_hir: hir_id,
                struct_args: args.to_vec(),
                iface_hir: iface_id,
                iface_args: iface_concrete,
            });
        }

        def
    }

    pub fn process_vtable_task(
        &mut self,
        struct_mir: MirTypeId,
        interface_mir: MirTypeId,
        struct_hir: TypeId,
        struct_args: Vec<ResolvedType>,
        iface_hir: TypeId,
        iface_args: Vec<ResolvedType>,
    ) {
        if self.mir.vtables.contains_key(&(struct_mir, interface_mir)) {
            return;
        }
        let iface = self.hir.interfaces[&iface_hir].clone();
        let mut slots = Vec::with_capacity(iface.methods.len());
        for iface_fn_id in &iface.methods {
            let method_name = self.hir.functions[iface_fn_id].name.clone();
            let (impl_fn_id, method_args) =
                self.resolve_method_with_args(struct_hir, &struct_args, &method_name);
            let mut all_args = struct_args.clone();
            all_args.extend(method_args);
            let mir_fn = self.mono_fn(impl_fn_id, all_args);
            slots.push(mir_fn);
        }
        self.mir.vtables.insert(
            (struct_mir, interface_mir),
            VTable {
                struct_ty: struct_mir,
                interface_ty: interface_mir,
                slots,
            },
        );

        let iface_subst = Subst::new(iface.type_params.clone(), iface_args);
        let parents: Vec<(TypeId, Vec<ResolvedType>)> = iface.extends.clone();
        for (parent_id, parent_args) in parents {
            let parent_concrete: Vec<ResolvedType> =
                parent_args.iter().map(|a| iface_subst.apply(a)).collect();
            let parent_mir =
                self.get_or_create_interface(parent_id, parent_concrete.clone());
            if self.mir.vtables.contains_key(&(struct_mir, parent_mir)) {
                continue;
            }
            self.worklist.push_back(MonoTask::VTable {
                struct_mir,
                interface_mir: parent_mir,
                struct_hir,
                struct_args: struct_args.clone(),
                iface_hir: parent_id,
                iface_args: parent_concrete,
            });
        }
    }

    // utils

    // main named function (we don't actually have a way of knowing if it is the entrypoint module :/ )
    pub fn find_main(&mut self) -> Option<FnId> {
        self.hir
            .functions
            .iter()
            .find(|(_, f)| f.name == "main" && f.owner.is_none() && f.type_params.is_empty())
            .map(|(id, _)| *id)
    }

    pub fn alloc_fn_id(&mut self) -> MirFnId {
        let id = MirFnId(self.next_fn_id);
        self.next_fn_id += 1;
        id
    }
    pub fn alloc_type_id(&mut self) -> MirTypeId {
        let id = MirTypeId(self.next_type_id);
        self.next_type_id += 1;
        id
    }

    fn locate_core_types(&mut self) {
        // Walk hir.interfaces / hir.structs by (module name, type name)
        // looking for "of:core"::Iterator and "of:core"::Entry; stash the
        // TypeId. These are used by the for-in desugar (Phase 3) and the
        // map iteration (also Phase 3).
        for (id, iface) in &self.hir.interfaces {
            if iface.name == "Iterator" {
                if let Some(m) = self.hir.modules.get(&iface.module) {
                    if m.name == "of:core" {
                        self.iterator_interface = Some(*id);
                    }
                }
            }
        }
        for (id, st) in &self.hir.structs {
            if st.name == "Entry" {
                if let Some(m) = self.hir.modules.get(&st.module) {
                    if m.name == "of:core" {
                        self.entry_struct = Some(*id);
                    }
                }
            }
        }
    }
}
