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

use crate::hir::{FnId, Hir, ResolvedType, TypeId, TypeParamId};
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
                    // let f = self.lower_function(fn_id, type_args, mir_id);
                    //self.mir.functions.insert(mir_id, f);
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
