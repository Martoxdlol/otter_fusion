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

use crate::ast::PrimitiveType;
use crate::hir::{FnId, Hir, HirBlock, HirStatement, ResolvedType, TypeId, TypeParamId};
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
                MonoTask::VTable { .. } => {
                    // TODO: implement vtables
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

        // f.type_params -> vec de params. Esos params pueden tener genéricos
        // type_args -> vec de tipos concretos. No pueden tener genéricos.
        let subst = Subst::new(f.type_params.clone(), type_args.clone());

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

        if let Some(body) = &(&f.body).clone() {
            let trailing: Option<Operand> = self.lower_block(body, &subst, &mut b);
            // implicit return of the trailing expression
            let term = match trailing {
                Some(op) => Terminator::Return(Some(op)),
                None => Terminator::Return(None),
            };
            // Only terminate if current block isn't already terminated.
            // (An early `return` inside the body would have set a terminator
            // and switched to a fresh unreachable block.)
            if matches!(
                b.blocks[&b.current_block].terminator,
                Terminator::Unreachable
            ) {
                b.terminate(term);
            }
        } else {
            panic!("cannot handle bodyless/extern yet");
        }

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
            ResolvedType::Primitive(p) => MirType::Primitive(p.clone()),

            ResolvedType::Struct(hir_id, args) => {
                let _is_extern = self.hir.structs[hir_id].is_extern;
                let mid = self.get_or_create_struct(*hir_id, args.clone());
                // TODO: Hacer algo con is_extern
                MirType::ManagedRef(mid)
            }

            ResolvedType::Interface(hir_id, args) => {
                todo!("interface lowering not implemented yet")
                // let mid = self.get_or_create_interface(*hir_id, args.clone());
                // MirType::ManagedRef(mid) // dispatched virtually at call sites
            }

            ResolvedType::Union(variants) => {
                // Phase 2 implements the T|null specialisation here;
                // for now everything goes through the tagged path.
                // let mid = self.get_or_create_union(variants.clone());
                // MirType::Union(mid)
                todo!("union lowering not implemented yet")
            }

            ResolvedType::Function(args, ret) => {
                // Default: assume closure (managed). FFI lowering (Phase 4)
                // overrides this to FnPtr at extern boundaries.
                // let mid = self.get_or_create_function_type(args, ret);
                // MirType::Closure(mid)
                todo!("function type lowering not implemented yet")
            }

            ResolvedType::Null => {
                panic!("Null must be part of a union type");
            }

            ResolvedType::TypeParam(id) => {
                panic!("non-concrete TypeParam({:?}) reached lowering", id);
            }
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
            .map(|f| self.lower_type(&f.ty, &subst))
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

        MirTypeDef::Struct {
            name,
            fields,
            layout,
            kind,
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
