use std::collections::{HashMap, HashSet};

use crate::{
    hir::{
        BinaryOperator, ExprKind, FnId, Hir, HirBlock, HirCapture, HirLiteral, HirParam,
        HirStatement, PrimitiveType, ResolvedType, TypeId, TypeParamId, TypedExpr, UnaryOperator,
    },
    mir::{
        Abi, AssignValue, BinOp, BlockId, Callee, Layout, LocalId, MirBlock, MirConst, MirField,
        MirFnId, MirFunction, MirLocal, MirProgram, MirType, MirTypeDef, MirTypeId, Operand, Stmt,
        StructKind, Terminator, TrapReason, UnOp, UnionVariant, VTable,
    },
};

/// HIR -> MIR lowerer.
///
/// Drives a worklist of monomorphization jobs starting from `main` and
/// produces a `MirProgram` with all generics specialised away, all
/// control flow flattened to basic blocks, and all allocations explicit.
pub struct Lower {
    /// Input. Assumed already validated (every name resolves, every type
    /// is concrete or a known type-param, every method dispatchable).
    hir: Hir,
    /// Output, built incrementally as functions are lowered.
    mir: MirProgram,

    /// Concrete (HirTypeId, type args) -> MirTypeId. Keyed on the *fully
    /// substituted* arg list so `Foo<T>` under T=i32 and `Foo<i32>` collide.
    struct_cache: HashMap<(TypeId, Vec<ResolvedType>), MirTypeId>,
    /// Same idea for interfaces. They get an empty-fields struct in MIR;
    /// dispatch goes through vtables instead.
    interface_cache: HashMap<(TypeId, Vec<ResolvedType>), MirTypeId>,
    /// Unions are keyed on the *canonicalised* (sorted, flattened, deduped)
    /// variant list so `A | B` and `B | A` map to the same MIR type.
    union_cache: HashMap<Vec<ResolvedType>, MirTypeId>,

    /// Function monomorphisations already discovered. Keyed on
    /// (HirFnId, type args) — same key shape as `struct_cache`.
    fn_cache: HashMap<(FnId, Vec<ResolvedType>), MirFnId>,
    /// Pending lowering jobs. We pre-allocate the `MirFnId` and insert
    /// into `fn_cache` before pushing here, so recursive calls reuse the
    /// id instead of looping forever.
    fn_queue: Vec<MonoJob>,

    next_type_id: u32,
    next_fn_id: u32,

    /// Per-function lowering state. `Some` only while a function body
    /// is being built; `None` between jobs.
    cur: Option<FnCtx>,

    /// Vtables we've already emitted, deduped on (struct, interface).
    /// A struct/interface pair only needs one vtable for the whole program.
    vtable_done: HashSet<(MirTypeId, MirTypeId)>,
}

/// One pending function-lowering job in the worklist.
struct MonoJob {
    /// MirFnId allocated up-front so callers can reference it before the
    /// body is lowered.
    target_id: MirFnId,
    hir_fn: FnId,
    /// Already substituted under the *caller's* type environment. No
    /// `TypeParam` should remain.
    type_args: Vec<ResolvedType>,
    /// Receiver's MIR type for methods. `None` for free functions.
    self_type: Option<MirTypeId>,
}

/// State that lives only while one function body is being lowered.
struct FnCtx {
    locals: HashMap<LocalId, MirLocal>,
    blocks: HashMap<BlockId, MirBlock>,
    next_local: u32,
    next_block: u32,
    /// Block where the next `emit` / `terminate` lands.
    cur_block: BlockId,
    /// Blocks whose terminator has been set. Prevents `emit`/`terminate`
    /// from clobbering an already-terminated block (e.g. statements
    /// after a `Return`).
    terminated: HashSet<BlockId>,

    /// Lexical scope stack for `Variable` lookup. Innermost is last.
    scopes: Vec<HashMap<String, LocalId>>,
    /// Generic substitution for this monomorphisation: every TypeParamId
    /// the function (or its receiver) declared maps to a concrete
    /// ResolvedType.
    type_subst: HashMap<TypeParamId, ResolvedType>,
    /// Stack of active loops; `break`/`continue` index the top.
    loops: Vec<LoopFrame>,
}

/// Where `break` and `continue` jump to inside a loop body.
struct LoopFrame {
    continue_to: BlockId,
    break_to: BlockId,
}

impl FnCtx {
    fn new() -> Self {
        Self {
            locals: HashMap::new(),
            blocks: HashMap::new(),
            next_local: 0,
            next_block: 0,
            cur_block: BlockId(0),
            terminated: HashSet::new(),
            scopes: Vec::new(),
            type_subst: HashMap::new(),
            loops: Vec::new(),
        }
    }
}

impl Lower {
    pub fn new(hir: Hir) -> Self {
        Self {
            hir,
            mir: MirProgram::new(),
            struct_cache: HashMap::new(),
            interface_cache: HashMap::new(),
            union_cache: HashMap::new(),
            fn_cache: HashMap::new(),
            fn_queue: Vec::new(),
            next_type_id: 0,
            next_fn_id: 0,
            cur: None,
            vtable_done: HashSet::new(),
        }
    }

    /// Drive the whole pass: locate `main`, queue it, then drain the
    /// worklist until no more reachable functions appear.
    pub fn lower(mut self) -> Result<MirProgram, String> {
        // Entry point convention (see validator tests, main.rs:90):
        // the entry function is `fn main()` inside the module named "main".
        let main_module = self
            .hir
            .modules
            .values()
            .find(|m| m.name == "main")
            .ok_or_else(|| "no `main` module found".to_string())?
            .id;

        let main_id = self
            .hir
            .functions
            .iter()
            .find(|(_, f)| {
                f.module == main_module
                    && f.owner.is_none()
                    && f.name == "main"
                    && f.type_params.is_empty()
                    && f.params.is_empty()
                    && !f.has_self
            })
            .map(|(id, _)| *id)
            .ok_or_else(|| "no `main` function in `main` module".to_string())?;

        // Queue main with no type args / no receiver. Everything else
        // gets discovered transitively as bodies are lowered.
        let entry_mid = self.monomorphize_fn(main_id, &[], None);
        self.mir.entry = entry_mid;

        // Drain the worklist. Lowering a body may push new jobs, which
        // we keep popping until the program reaches a fixed point.
        while let Some(job) = self.fn_queue.pop() {
            self.lower_function(job);
        }

        Ok(self.mir)
    }
}

impl Lower {
    fn cur(&self) -> &FnCtx {
        self.cur.as_ref().expect("not inside a function")
    }
    fn cur_mut(&mut self) -> &mut FnCtx {
        self.cur.as_mut().expect("not inside a function")
    }

    fn fresh_type_id(&mut self) -> MirTypeId {
        let id = MirTypeId(self.next_type_id);
        self.next_type_id += 1;
        id
    }
    fn fresh_fn_id(&mut self) -> MirFnId {
        let id = MirFnId(self.next_fn_id);
        self.next_fn_id += 1;
        id
    }

    /// Allocate a new local in the current function and register it.
    fn new_local(&mut self, name: Option<String>, ty: MirType) -> LocalId {
        let id = LocalId(self.cur().next_local);
        self.cur_mut().next_local += 1;
        self.cur_mut().locals.insert(id, MirLocal { id, name, ty });
        id
    }

    /// Allocate an unnamed local whose type comes from a HIR type
    /// (after substitution + interning).
    fn new_temp(&mut self, ty: &ResolvedType) -> LocalId {
        let r = self.subst(ty);
        let mty = self.intern_type(&r);
        self.new_local(None, mty)
    }

    /// Allocate an unnamed local with an already-known MIR type. Used
    /// when we synthesise locals (tags, bools, etc.) without a HIR
    /// counterpart.
    fn new_temp_mty(&mut self, mty: MirType) -> LocalId {
        self.new_local(None, mty)
    }

    /// Create a fresh block. Terminator starts as `Unreachable` so any
    /// block we forget to terminate stays trivially well-formed; real
    /// terminators overwrite it via `terminate`.
    fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.cur().next_block);
        self.cur_mut().next_block += 1;
        self.cur_mut().blocks.insert(
            id,
            MirBlock {
                id,
                stmts: Vec::new(),
                terminator: Terminator::Unreachable,
            },
        );
        id
    }

    /// Append a statement to `cur_block`, unless that block is already
    /// terminated (in which case we silently drop — caller is producing
    /// dead code, e.g. statements after a `return`).
    fn emit(&mut self, s: Stmt) {
        let b = self.cur().cur_block;
        if self.cur().terminated.contains(&b) {
            return;
        }
        self.cur_mut().blocks.get_mut(&b).unwrap().stmts.push(s);
    }

    /// Set the terminator on `cur_block`. Idempotent on already-terminated
    /// blocks so callers don't have to track the state themselves.
    fn terminate(&mut self, t: Terminator) {
        let b = self.cur().cur_block;
        if self.cur().terminated.contains(&b) {
            return;
        }
        self.cur_mut().blocks.get_mut(&b).unwrap().terminator = t;
        self.cur_mut().terminated.insert(b);
    }

    fn push_scope(&mut self) {
        self.cur_mut().scopes.push(HashMap::new());
    }
    fn pop_scope(&mut self) {
        self.cur_mut().scopes.pop();
    }
    /// Bind `name` to `id` in the innermost scope.
    fn bind(&mut self, name: &str, id: LocalId) {
        let scopes = &mut self.cur_mut().scopes;
        scopes
            .last_mut()
            .expect("no active scope")
            .insert(name.to_string(), id);
    }
    /// Resolve `name` walking scopes outward; panics if not found.
    fn lookup(&self, name: &str) -> LocalId {
        for s in self.cur().scopes.iter().rev() {
            if let Some(id) = s.get(name) {
                return *id;
            }
        }
        panic!("unknown variable `{name}` (validator should have caught this)");
    }
}

impl Lower {
    /// Substitute every `TypeParam` in `ty` against the current function's
    /// type environment. Must be run on every HIR type before interning,
    /// otherwise `intern_type` will hit the unreachable! arm.
    fn subst(&self, ty: &ResolvedType) -> ResolvedType {
        let table = &self.cur().type_subst;
        subst_with(ty, table)
    }
}

/// Standalone substitution against an arbitrary table. Used by
/// `intern_struct` (which builds a per-struct table) and by
/// `iface_method_names` (which threads parent-iface args through
/// child-iface params).
fn subst_with(ty: &ResolvedType, table: &HashMap<TypeParamId, ResolvedType>) -> ResolvedType {
    match ty {
        ResolvedType::TypeParam(p) => table
            .get(p)
            .cloned()
            .unwrap_or_else(|| panic!("unresolved type param {:?}", p)),
        ResolvedType::Struct(id, args) => {
            ResolvedType::Struct(*id, args.iter().map(|a| subst_with(a, table)).collect())
        }
        ResolvedType::Interface(id, args) => {
            ResolvedType::Interface(*id, args.iter().map(|a| subst_with(a, table)).collect())
        }
        ResolvedType::Union(parts) => {
            ResolvedType::Union(parts.iter().map(|a| subst_with(a, table)).collect())
        }
        ResolvedType::Function(args, ret) => ResolvedType::Function(
            args.iter().map(|a| subst_with(a, table)).collect(),
            Box::new(subst_with(ret, table)),
        ),
        ResolvedType::Primitive(_) | ResolvedType::Null => ty.clone(),
    }
}

impl Lower {
    /// Map a fully-substituted ResolvedType to a MirType, allocating
    /// MirTypeDefs as needed. Idempotent via the per-shape caches.
    fn intern_type(&mut self, ty: &ResolvedType) -> MirType {
        match ty {
            ResolvedType::Primitive(p) => MirType::Primitive(p.clone()),
            ResolvedType::Struct(id, args) => {
                let mid = self.intern_struct(*id, args);
                MirType::ManagedRef(mid)
            }
            ResolvedType::Interface(id, args) => {
                let mid = self.intern_interface(*id, args);
                MirType::ManagedRef(mid)
            }
            ResolvedType::Union(parts) => self.intern_union(parts),
            ResolvedType::Function(args, ret) => {
                // Function-typed values lower to plain fn-pointers here.
                // Concrete closures created by `FunctionLiteral` get their
                // own MirTypeDef::Closure at the literal site.
                let margs: Vec<MirType> = args.iter().map(|a| self.intern_type(a)).collect();
                let mret = self.intern_type(ret);
                MirType::FnPtr(margs, Box::new(mret))
            }
            // Bare `null` only shows up as a union variant; the union
            // arm handles it. If we get here it's effectively unused.
            ResolvedType::Null => MirType::Primitive(PrimitiveType::Bool),
            ResolvedType::TypeParam(_) => unreachable!("subst must run first"),
        }
    }

    /// Intern a concrete struct instantiation. Recursive struct fields
    /// (Foo containing Foo via a managed ref) terminate because we
    /// insert into the cache *before* recursing on the fields.
    fn intern_struct(&mut self, id: TypeId, args: &[ResolvedType]) -> MirTypeId {
        let key = (id, args.to_vec());
        if let Some(m) = self.struct_cache.get(&key) {
            return *m;
        }

        // Reserve an id and cache it now so any recursive reference back
        // to this same struct (through ManagedRef) finds it instead of
        // looping. The MirTypeDef itself is filled in below.
        let mid = self.fresh_type_id();
        self.struct_cache.insert(key, mid);

        // Cloned because we need to mutate `self` (for child intern calls)
        // while reading the struct's definition.
        let s = self.hir.structs[&id].clone();

        // Build the per-struct subst from the struct's declared params
        // to the concrete args we were called with. Field types use
        // *this* table, not the surrounding function's environment.
        let mut local_subst: HashMap<TypeParamId, ResolvedType> = HashMap::new();
        for (p, a) in s.type_params.iter().zip(args.iter()) {
            local_subst.insert(*p, a.clone());
        }

        let mut fields = Vec::with_capacity(s.fields.len());
        for f in &s.fields {
            let resolved = subst_with(&f.ty, &local_subst);
            // `is_pointer` fields wrap the field type in a raw pointer —
            // these come from extern/FFI declarations.
            let mty = if f.is_pointer {
                MirType::Pointer(Box::new(self.intern_type(&resolved)))
            } else {
                self.intern_type(&resolved)
            };
            fields.push(MirField {
                name: f.name.clone(),
                ty: mty,
            });
        }

        let layout = compute_layout(&fields);
        let kind = if s.is_extern {
            StructKind::Extern
        } else {
            StructKind::Managed
        };

        self.mir.types.insert(
            mid,
            MirTypeDef::Struct {
                name: format!("{}<{}>", s.name, fmt_args(args)),
                fields,
                layout,
                kind,
            },
        );

        // Eagerly emit a vtable for every interface this struct
        // implements. Doing it here means later code that coerces this
        // struct to an interface always finds a vtable ready.
        for (iid, iargs) in s.implements.clone() {
            let iargs_subst: Vec<ResolvedType> =
                iargs.iter().map(|t| subst_with(t, &local_subst)).collect();
            let iface_mid = self.intern_interface(iid, &iargs_subst);
            self.emit_vtable(id, args, mid, iid, &iargs_subst, iface_mid);
        }

        mid
    }

    /// Interfaces are represented in MIR as zero-field managed structs.
    /// All real dispatch info lives in `mir.vtables`, keyed by the pair
    /// (concrete struct, interface).
    fn intern_interface(&mut self, id: TypeId, args: &[ResolvedType]) -> MirTypeId {
        let key = (id, args.to_vec());
        if let Some(m) = self.interface_cache.get(&key) {
            return *m;
        }
        let mid = self.fresh_type_id();
        self.interface_cache.insert(key, mid);

        let iface = self.hir.interfaces[&id].clone();
        // Fat-ref payload at runtime is just a managed pointer; the
        // type-id slot at offset −1 carries the concrete type.
        let layout = Layout {
            size: pointer_size(),
            align: pointer_size(),
        };
        self.mir.types.insert(
            mid,
            MirTypeDef::Struct {
                name: format!("iface:{}<{}>", iface.name, fmt_args(args)),
                fields: Vec::new(),
                layout,
                kind: StructKind::Managed,
            },
        );
        mid
    }

    /// Intern a union type. Canonicalisation (flatten + sort + dedupe)
    /// makes equivalent unions share a MirTypeId regardless of how they
    /// were spelled in source. Also applies the `T | null` -> NullableRef
    /// optimisation.
    fn intern_union(&mut self, parts: &[ResolvedType]) -> MirType {
        // Flatten nested unions and re-substitute each part defensively
        // (callers should already have, but cheap to repeat).
        let mut flat: Vec<ResolvedType> = Vec::new();
        for p in parts {
            let p = self.subst(p);
            match p {
                ResolvedType::Union(inner) => flat.extend(inner),
                other => flat.push(other),
            }
        }
        // Content-based ordering via Debug formatting is enough for
        // determinism; the exact order doesn't matter as long as it's
        // stable across the program.
        flat.sort_by(|a, b| canonical_key(a).cmp(&canonical_key(b)));
        flat.dedup();

        // T | null special case: skip the union representation and use
        // the null-pointer-optimised NullableRef layout instead.
        if flat.len() == 2 && flat.iter().any(|t| matches!(t, ResolvedType::Null)) {
            let payload = flat
                .iter()
                .find(|t| !matches!(t, ResolvedType::Null))
                .unwrap()
                .clone();
            if let MirType::ManagedRef(mid) = self.intern_type(&payload) {
                return MirType::NullableRef(mid);
            }
        }

        if let Some(m) = self.union_cache.get(&flat) {
            return MirType::Union(*m);
        }

        let mid = self.fresh_type_id();
        self.union_cache.insert(flat.clone(), mid);

        // Variant tags are positions in the canonical vector. The same
        // ordering is used by `is`/`as` lowering to look up tags.
        let mut variants = Vec::with_capacity(flat.len());
        for (i, p) in flat.iter().enumerate() {
            variants.push(UnionVariant {
                tag: i as u16,
                ty: self.intern_type(p),
            });
        }
        let layout = compute_union_layout(&variants);
        self.mir
            .types
            .insert(mid, MirTypeDef::Union { variants, layout });
        MirType::Union(mid)
    }
}

/// Stringly canonical key for ordering / equality of ResolvedTypes.
/// Debug derives are content-based, so this is stable for a given HIR.
fn canonical_key(t: &ResolvedType) -> String {
    format!("{:?}", t)
}

/// Pretty-print type-arg list for synthesised MIR names like
/// `Foo<i32>` or `add<i32>`. Purely cosmetic.
fn fmt_args(args: &[ResolvedType]) -> String {
    args.iter()
        .map(|t| format!("{:?}", t))
        .collect::<Vec<_>>()
        .join(",")
}

/// Pointer / managed-ref size on the target. Hard-coded to 64-bit.
fn pointer_size() -> u32 {
    8
}

/// Alignment for a MIR type. All reference-shaped things are
/// pointer-aligned; primitives align to their size.
fn align_of(t: &MirType) -> u32 {
    match t {
        MirType::Primitive(p) => prim_size(p),
        MirType::ManagedRef(_)
        | MirType::Pointer(_)
        | MirType::FnPtr(_, _)
        | MirType::Closure(_)
        | MirType::NullableRef(_) => pointer_size(),
        MirType::Union(_) => pointer_size(),
    }
}

/// Storage size for a MIR type. For reference / pointer / union shapes
/// this matches alignment — they're all word-sized handles. Concrete
/// struct/union sizes live on their MirTypeDef::layout, not here.
fn size_of(t: &MirType) -> u32 {
    align_of(t)
}

fn prim_size(p: &PrimitiveType) -> u32 {
    match p {
        PrimitiveType::Int8 | PrimitiveType::Uint8 | PrimitiveType::Bool => 1,
        PrimitiveType::Int16 | PrimitiveType::Uint16 => 2,
        PrimitiveType::Int32
        | PrimitiveType::Uint32
        | PrimitiveType::Float32
        | PrimitiveType::Char => 4,
        PrimitiveType::Int64 | PrimitiveType::Uint64 | PrimitiveType::Float64 => 8,
        // Strings are GC-managed refs.
        PrimitiveType::String => pointer_size(),
    }
}

/// Round `off` up to the next multiple of `align`.
fn align_up(off: u32, align: u32) -> u32 {
    if align == 0 {
        off
    } else {
        (off + align - 1) / align * align
    }
}

/// Lay out a struct's fields with the standard "align each field, sum
/// them, then align the total" rule. No reordering / packing.
fn compute_layout(fields: &[MirField]) -> Layout {
    let mut align: u32 = 1;
    let mut size: u32 = 0;
    for f in fields {
        let fa = align_of(&f.ty).max(1);
        let fs = size_of(&f.ty);
        if fa > align {
            align = fa;
        }
        size = align_up(size, fa);
        size += fs;
    }
    let align = align.max(1);
    let size = align_up(size, align);
    Layout { size, align }
}

/// Union layout: 2-byte tag at offset 0, payload at the next aligned
/// offset, total size = align_up(payload_off + max_payload).
fn compute_union_layout(variants: &[UnionVariant]) -> Layout {
    // Start at pointer alignment so refs in payload stay aligned even
    // for unions of small primitives.
    let mut align: u32 = pointer_size();
    let mut payload_max: u32 = 0;
    for v in variants {
        let a = align_of(&v.ty).max(1);
        let s = size_of(&v.ty);
        if a > align {
            align = a;
        }
        if s > payload_max {
            payload_max = s;
        }
    }
    let tag = 2u32;
    let payload_off = align_up(tag, align);
    let size = align_up(payload_off + payload_max, align);
    Layout { size, align }
}

impl Lower {
    /// Get-or-create the MirFnId for `(fid, type_args)`. If new, queues
    /// a job for later body lowering. Caching before queueing is what
    /// breaks recursion cycles — a recursive call resolves to the same
    /// id we just allocated for the outer call.
    fn monomorphize_fn(
        &mut self,
        fid: FnId,
        type_args: &[ResolvedType],
        self_mid: Option<MirTypeId>,
    ) -> MirFnId {
        // Substitute under the *caller's* environment so that e.g.
        // `foo<T>` called from inside `bar<U=i32>` keys on `foo<i32>`.
        let resolved: Vec<ResolvedType> = type_args.iter().map(|t| self.subst(t)).collect();
        let key = (fid, resolved.clone());
        if let Some(m) = self.fn_cache.get(&key) {
            return *m;
        }
        let mid = self.fresh_fn_id();
        self.fn_cache.insert(key, mid);
        self.fn_queue.push(MonoJob {
            target_id: mid,
            hir_fn: fid,
            type_args: resolved,
            self_type: self_mid,
        });
        mid
    }

    /// Lower the body of one queued monomorphisation.
    fn lower_function(&mut self, job: MonoJob) {
        let hir_fn = self.hir.functions[&job.hir_fn].clone();

        // Fresh per-function context — locals/blocks/scopes all start empty.
        self.cur = Some(FnCtx::new());

        // Build subst from the function's declared params to the args
        // this monomorphisation was queued with.
        let mut subst = HashMap::new();
        for (p, a) in hir_fn.type_params.iter().zip(job.type_args.iter()) {
            subst.insert(*p, a.clone());
        }
        self.cur_mut().type_subst = subst;

        // Top-level scope for the function body.
        self.push_scope();

        // Parameters become the first locals, in order. `self` is implicit
        // and goes first when present.
        let mut params: Vec<LocalId> = Vec::new();
        if hir_fn.has_self {
            let self_mid = job.self_type.expect("method needs self_type");
            let id = self.new_local(Some("self".into()), MirType::ManagedRef(self_mid));
            self.bind("self", id);
            params.push(id);
        }
        for p in &hir_fn.params {
            let resolved = self.subst(&p.ty);
            let mty = if p.is_pointer {
                MirType::Pointer(Box::new(self.intern_type(&resolved)))
            } else {
                self.intern_type(&resolved)
            };
            let id = self.new_local(Some(p.name.clone()), mty);
            self.bind(&p.name, id);
            params.push(id);
        }

        let ret_resolved = self.subst(&hir_fn.return_type);
        let return_type = self.intern_type(&ret_resolved);

        // First block of the body. Lowering walks statements appending
        // here, splitting/branching as needed.
        let entry = self.new_block();
        self.cur_mut().cur_block = entry;

        let abi = Abi::Otter;

        if let Some(body) = hir_fn.body.clone() {
            // Bodies can fall through into an implicit return value
            // (the `returns` slot of HirBlock). If they do, that's our
            // final Return operand.
            let implicit = self.lower_block(&body);
            let cur_b = self.cur().cur_block;
            if !self.cur().terminated.contains(&cur_b) {
                self.terminate(Terminator::Return(implicit));
            }
        }
        // Bodyless functions (extern declarations / abstract methods)
        // just keep the entry block's placeholder Unreachable terminator.

        self.pop_scope();

        let ctx = self.cur.take().unwrap();
        self.mir.functions.insert(
            job.target_id,
            MirFunction {
                id: job.target_id,
                name: format!("{}<{}>", hir_fn.name, fmt_args(&job.type_args)),
                abi,
                params,
                locals: ctx.locals,
                blocks: ctx.blocks,
                entry,
                return_type,
            },
        );
    }
}

impl Lower {
    /// Lower a HirBlock, opening a fresh lexical scope. Returns the
    /// implicit-return operand (the value of the trailing `returns`
    /// expression) if one exists.
    fn lower_block(&mut self, block: &HirBlock) -> Option<Operand> {
        self.push_scope();
        for s in &block.statements {
            self.lower_stmt(s);
        }
        let result = block.returns.as_ref().map(|e| self.lower_expr(e));
        self.pop_scope();
        result
    }

    /// Like `lower_block` but discards the implicit return — for places
    /// where the value isn't usable (e.g. a `while` body).
    fn lower_block_inline(&mut self, block: &HirBlock) {
        self.push_scope();
        for s in &block.statements {
            self.lower_stmt(s);
        }
        if let Some(e) = &block.returns {
            let _ = self.lower_expr(e);
        }
        self.pop_scope();
    }

    fn lower_stmt(&mut self, s: &HirStatement) {
        match s {
            // Bare expression statement — evaluate for side-effects, drop value.
            HirStatement::Expr(e) => {
                let _ = self.lower_expr(e);
            }

            HirStatement::VarDecl(name, ty, init) => {
                let resolved = self.subst(ty);
                let mty = self.intern_type(&resolved);
                let id = self.new_local(Some(name.clone()), mty);
                if let Some(e) = init {
                    let op = self.lower_expr(e);
                    self.emit(Stmt::Assign(id, AssignValue::Use(op)));
                }
                // Bind only after evaluating the initialiser, so an init
                // expression can't accidentally see the variable it
                // initialises.
                self.bind(name, id);
            }

            HirStatement::Return(opt) => {
                let op = opt.as_ref().map(|e| self.lower_expr(e));
                self.terminate(Terminator::Return(op));
                // Anything after the return goes into a fresh dead
                // block — we keep emitting into a valid block id, even
                // if its content will be unreachable.
                let dead = self.new_block();
                self.cur_mut().cur_block = dead;
            }

            HirStatement::While(cond, body) => self.lower_while(cond, body),

            HirStatement::For(_, _, _) => unimplemented!("for-in is out of scope"),

            HirStatement::Break => {
                let to = self
                    .cur()
                    .loops
                    .last()
                    .expect("break outside loop")
                    .break_to;
                self.terminate(Terminator::Goto(to));
                let dead = self.new_block();
                self.cur_mut().cur_block = dead;
            }
            HirStatement::Continue => {
                let to = self
                    .cur()
                    .loops
                    .last()
                    .expect("continue outside loop")
                    .continue_to;
                self.terminate(Terminator::Goto(to));
                let dead = self.new_block();
                self.cur_mut().cur_block = dead;
            }
        }
    }

    /// Standard while-loop CFG:
    ///
    ///     pre  -> header (cond?) -> body -> header (back-edge)
    ///                          -> exit
    ///
    /// `continue` jumps to header, `break` to exit.
    fn lower_while(&mut self, cond: &TypedExpr, body: &HirBlock) {
        let header = self.new_block();
        let body_b = self.new_block();
        let exit = self.new_block();

        // Pre-header falls into the header.
        self.terminate(Terminator::Goto(header));

        // Header evaluates the condition fresh on each iteration.
        self.cur_mut().cur_block = header;
        let c = self.lower_expr(cond);
        self.terminate(Terminator::CondBr(c, body_b, exit));

        // Body, with a loop frame so break/continue resolve correctly.
        self.cur_mut().cur_block = body_b;
        self.cur_mut().loops.push(LoopFrame {
            continue_to: header,
            break_to: exit,
        });
        self.lower_block_inline(body);
        self.cur_mut().loops.pop();

        // Back-edge to header — but only if the body didn't already
        // terminate (e.g. via a return inside it).
        let cur_b = self.cur().cur_block;
        if !self.cur().terminated.contains(&cur_b) {
            self.terminate(Terminator::Goto(header));
        }

        // Continue codegen in the exit block.
        self.cur_mut().cur_block = exit;
    }
}

impl Lower {
    /// Lower an expression into an Operand. Most non-trivial expressions
    /// materialise a fresh local and return Copy(local); pure constants
    /// return Const directly.
    fn lower_expr(&mut self, e: &TypedExpr) -> Operand {
        match &e.kind {
            ExprKind::Literal(lit) => Operand::Const(lower_literal(lit, &e.ty)),

            ExprKind::Variable(name) => {
                // Locals win over free functions of the same name —
                // shadowing-by-let is normal here.
                if let Some(id) = self.lookup_opt(name) {
                    Operand::Copy(id)
                } else if let Some(fid) = self.resolve_free_fn(name) {
                    // Bare function reference used as a value — pin it
                    // to a MirFnId const and let codegen turn it into a
                    // function pointer.
                    let mid = self.monomorphize_fn(fid, &[], None);
                    Operand::Const(MirConst::Fn(mid))
                } else {
                    panic!("unknown name `{name}`");
                }
            }

            ExprKind::BinaryOp(l, op, r) => {
                // && and || need real branching to short-circuit, the
                // others are a single Bin instruction.
                if matches!(op, BinaryOperator::And | BinaryOperator::Or) {
                    return self.lower_short_circuit(l, op, r);
                }
                let a = self.lower_expr(l);
                let b = self.lower_expr(r);
                let dst = self.new_temp(&e.ty);
                self.emit(Stmt::Assign(dst, AssignValue::Bin(map_binop(op), a, b)));
                Operand::Copy(dst)
            }

            ExprKind::UnaryOp(op, x) => {
                let v = self.lower_expr(x);
                let dst = self.new_temp(&e.ty);
                self.emit(Stmt::Assign(dst, AssignValue::Un(map_unop(op), v)));
                Operand::Copy(dst)
            }

            ExprKind::If(cond, then_b, else_b) => {
                self.lower_if(cond, then_b, else_b.as_deref(), &e.ty)
            }

            ExprKind::Block(b) => self.lower_block_value(b, &e.ty),

            ExprKind::Call(callee, type_args, args) => {
                self.lower_call(callee, type_args, args, &e.ty)
            }

            ExprKind::StructInit(id, type_args, fields) => {
                self.lower_struct_init(*id, type_args, fields, &e.ty)
            }

            ExprKind::Member(base, name) => self.lower_member(base, name, &e.ty),

            ExprKind::As(x, target) => self.lower_as(x, target, &e.ty),
            ExprKind::Is(x, target) => self.lower_is(x, target),

            ExprKind::FunctionLiteral(tps, params, captures, body) => {
                self.lower_closure(tps, params, captures, body, &e.ty)
            }

            ExprKind::LiteralList(_) | ExprKind::LiteralMap(_) => {
                unimplemented!("list/map literals are out of scope")
            }
        }
    }

    /// Scope-walk variant of `lookup` that returns None instead of panicking.
    /// Used to disambiguate variables vs. free functions of the same name.
    fn lookup_opt(&self, name: &str) -> Option<LocalId> {
        for s in self.cur().scopes.iter().rev() {
            if let Some(id) = s.get(name) {
                return Some(*id);
            }
        }
        None
    }

    /// Find a free function by name. The validator guarantees uniqueness
    /// within scope, so the first match is the right one.
    fn resolve_free_fn(&self, name: &str) -> Option<FnId> {
        self.hir
            .functions
            .iter()
            .find(|(_, f)| f.owner.is_none() && !f.has_self && f.name == name)
            .map(|(id, _)| *id)
    }

    /// Short-circuit lowering for `&&` / `||`.
    ///
    ///     a && b   ≡   if a { b } else { false }
    ///     a || b   ≡   if a { true } else { b }
    fn lower_short_circuit(
        &mut self,
        l: &TypedExpr,
        op: &BinaryOperator,
        r: &TypedExpr,
    ) -> Operand {
        let bool_ty = MirType::Primitive(PrimitiveType::Bool);
        let dst = self.new_temp_mty(bool_ty);

        let a = self.lower_expr(l);
        let then_b = self.new_block();
        let else_b = self.new_block();
        let join = self.new_block();
        self.terminate(Terminator::CondBr(a, then_b, else_b));

        match op {
            BinaryOperator::And => {
                // a was true -> result is whatever b evaluates to.
                self.cur_mut().cur_block = then_b;
                let v = self.lower_expr(r);
                self.emit(Stmt::Assign(dst, AssignValue::Use(v)));
                self.terminate(Terminator::Goto(join));

                // a was false -> short-circuit to false without evaluating b.
                self.cur_mut().cur_block = else_b;
                self.emit(Stmt::Assign(
                    dst,
                    AssignValue::Use(Operand::Const(MirConst::Bool(false))),
                ));
                self.terminate(Terminator::Goto(join));
            }
            BinaryOperator::Or => {
                // a was true -> short-circuit to true.
                self.cur_mut().cur_block = then_b;
                self.emit(Stmt::Assign(
                    dst,
                    AssignValue::Use(Operand::Const(MirConst::Bool(true))),
                ));
                self.terminate(Terminator::Goto(join));

                // a was false -> result is whatever b evaluates to.
                self.cur_mut().cur_block = else_b;
                let v = self.lower_expr(r);
                self.emit(Stmt::Assign(dst, AssignValue::Use(v)));
                self.terminate(Terminator::Goto(join));
            }
            _ => unreachable!(),
        }

        self.cur_mut().cur_block = join;
        Operand::Copy(dst)
    }

    /// `if`-as-expression. Both arms write into a shared `dst` local; the
    /// no-else case writes `null` on the fallthrough path so `dst` is
    /// always defined at the join.
    fn lower_if(
        &mut self,
        cond: &TypedExpr,
        then_b: &HirBlock,
        else_b: Option<&HirBlock>,
        ty: &ResolvedType,
    ) -> Operand {
        let dst = self.new_temp(ty);

        let c = self.lower_expr(cond);
        let then_blk = self.new_block();
        let else_blk = self.new_block();
        let join = self.new_block();
        self.terminate(Terminator::CondBr(c, then_blk, else_blk));

        // Then-arm.
        self.cur_mut().cur_block = then_blk;
        let t = self.lower_block(then_b);
        if let Some(op) = t {
            self.emit(Stmt::Assign(dst, AssignValue::Use(op)));
        }
        let cur_b = self.cur().cur_block;
        if !self.cur().terminated.contains(&cur_b) {
            self.terminate(Terminator::Goto(join));
        }

        // Else-arm. Missing `else` defaults the value to null (the type
        // is `T | null` in that case, by validator construction).
        self.cur_mut().cur_block = else_blk;
        if let Some(eb) = else_b {
            let v = self.lower_block(eb);
            if let Some(op) = v {
                self.emit(Stmt::Assign(dst, AssignValue::Use(op)));
            }
        } else {
            self.emit(Stmt::Assign(
                dst,
                AssignValue::Use(Operand::Const(MirConst::Null)),
            ));
        }
        let cur_b = self.cur().cur_block;
        if !self.cur().terminated.contains(&cur_b) {
            self.terminate(Terminator::Goto(join));
        }

        self.cur_mut().cur_block = join;
        Operand::Copy(dst)
    }

    /// `{ ... }` used in expression position — yields the block's
    /// implicit return value, or null if it has none.
    fn lower_block_value(&mut self, b: &HirBlock, ty: &ResolvedType) -> Operand {
        let dst = self.new_temp(ty);
        let v = self.lower_block(b);
        match v {
            Some(op) => {
                self.emit(Stmt::Assign(dst, AssignValue::Use(op)));
            }
            None => {
                self.emit(Stmt::Assign(
                    dst,
                    AssignValue::Use(Operand::Const(MirConst::Null)),
                ));
            }
        }
        Operand::Copy(dst)
    }

    /// Lower a function call. Three paths:
    /// - `recv.method(...)` -> dispatch on recv's type (static or virtual)
    /// - bare ident naming a free fn -> static call
    /// - everything else (closure, function-pointer value) -> indirect
    fn lower_call(
        &mut self,
        callee: &TypedExpr,
        type_args: &[ResolvedType],
        args: &[TypedExpr],
        ret_ty: &ResolvedType,
    ) -> Operand {
        let resolved_args: Vec<ResolvedType> = type_args.iter().map(|t| self.subst(t)).collect();

        // Method call: `Member(recv, name)` is the callee. The receiver
        // is lowered here (once) and threaded into lower_method_call,
        // which decides between Static and Virtual dispatch.
        if let ExprKind::Member(recv, method_name) = &callee.kind {
            let recv_op = self.lower_expr(recv);
            let recv_ty = self.subst(&recv.ty);
            let arg_ops: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
            return self.lower_method_call(
                recv_op,
                recv_ty,
                method_name,
                &resolved_args,
                arg_ops,
                ret_ty,
            );
        }

        // Bare identifier naming a free function — static call. We
        // check `lookup_opt` first so a local with the same name still
        // wins (and falls through to indirect dispatch below).
        if let ExprKind::Variable(name) = &callee.kind {
            if self.lookup_opt(name).is_none() {
                if let Some(fid) = self.resolve_free_fn(name) {
                    let arg_ops: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                    let mid = self.monomorphize_fn(fid, &resolved_args, None);
                    let dst = self.new_temp(ret_ty);
                    self.emit(Stmt::Assign(
                        dst,
                        AssignValue::Call(Callee::Static(mid), arg_ops),
                    ));
                    return Operand::Copy(dst);
                }
            }
        }

        // Anything else — closure value, fn-pointer in a local, etc.
        let f = self.lower_expr(callee);
        let arg_ops: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
        let dst = self.new_temp(ret_ty);
        self.emit(Stmt::Assign(
            dst,
            AssignValue::Call(Callee::Indirect(f), arg_ops),
        ));
        Operand::Copy(dst)
    }

    /// Method-call dispatch chooses between static (concrete struct
    /// receiver, look up the impl directly) and virtual (interface
    /// receiver, go through the vtable slot).
    fn lower_method_call(
        &mut self,
        recv: Operand,
        recv_ty: ResolvedType,
        method_name: &str,
        type_args: &[ResolvedType],
        arg_ops: Vec<Operand>,
        ret_ty: &ResolvedType,
    ) -> Operand {
        let dst = self.new_temp(ret_ty);

        match recv_ty {
            ResolvedType::Struct(sid, sargs) => {
                // Static dispatch: the validator already picked the
                // unique impl, we just need to monomorphise it.
                let self_mid = self.intern_struct(sid, &sargs);
                let fid = self
                    .resolve_struct_method(sid, &sargs, method_name)
                    .unwrap_or_else(|| panic!("no method `{method_name}` on struct {:?}", sid));
                let mid = self.monomorphize_fn(fid, type_args, Some(self_mid));
                // First arg is the receiver, then the explicit args.
                let mut all_args = Vec::with_capacity(arg_ops.len() + 1);
                all_args.push(recv);
                all_args.extend(arg_ops);
                self.emit(Stmt::Assign(
                    dst,
                    AssignValue::Call(Callee::Static(mid), all_args),
                ));
            }
            ResolvedType::Interface(iid, iargs) => {
                // Virtual dispatch through (interface_mid, slot).
                let iface_mid = self.intern_interface(iid, &iargs);
                let slot = self.iface_method_slot(iid, &iargs, method_name);
                let mut all_args = Vec::with_capacity(arg_ops.len() + 1);
                all_args.push(recv.clone());
                all_args.extend(arg_ops);
                self.emit(Stmt::Assign(
                    dst,
                    AssignValue::Call(Callee::Virtual(recv, iface_mid, slot), all_args),
                ));
            }
            other => panic!("method call on non-aggregate: {:?}", other),
        }
        Operand::Copy(dst)
    }

    /// Find a method named `method_name` on a struct. Searches inline
    /// methods first, then `extend`-block contributions. The validator
    /// guarantees at most one match.
    fn resolve_struct_method(
        &self,
        sid: TypeId,
        _sargs: &[ResolvedType],
        method_name: &str,
    ) -> Option<FnId> {
        let s = self.hir.structs.get(&sid)?;
        for &fid in &s.methods {
            let f = &self.hir.functions[&fid];
            if f.name == method_name {
                return Some(fid);
            }
        }
        for (_targs, fid) in &s.specialised_methods {
            let f = &self.hir.functions[fid];
            if f.name == method_name {
                return Some(*fid);
            }
        }
        None
    }

    /// Look up the vtable slot index for a method on an interface.
    /// Slot order is the same lexicographic flattening used by
    /// `emit_vtable`, so callers and impls always agree.
    fn iface_method_slot(&self, iid: TypeId, iargs: &[ResolvedType], method_name: &str) -> u32 {
        let names = self.iface_method_names(iid, iargs);
        names
            .iter()
            .position(|n| n == method_name)
            .unwrap_or_else(|| panic!("no slot for `{method_name}` on iface {:?}", iid))
            as u32
    }

    /// Flatten an interface's full method name set, including methods
    /// inherited from parent interfaces. Sorted + deduped so the
    /// resulting order is deterministic.
    fn iface_method_names(&self, iid: TypeId, iargs: &[ResolvedType]) -> Vec<String> {
        let iface = &self.hir.interfaces[&iid];

        // Build the iface-local subst so parent type-args expressed in
        // terms of *this* interface's params get resolved.
        let mut subst: HashMap<TypeParamId, ResolvedType> = HashMap::new();
        for (p, a) in iface.type_params.iter().zip(iargs.iter()) {
            subst.insert(*p, a.clone());
        }

        let mut names: Vec<String> = Vec::new();
        for &fid in &iface.methods {
            names.push(self.hir.functions[&fid].name.clone());
        }
        // Recurse into parent interfaces, threading the substituted args.
        for (parent_id, parent_args) in &iface.extends {
            let resolved: Vec<ResolvedType> =
                parent_args.iter().map(|t| subst_with(t, &subst)).collect();
            for n in self.iface_method_names(*parent_id, &resolved) {
                if !names.contains(&n) {
                    names.push(n);
                }
            }
        }
        names.sort();
        names.dedup();
        names
    }

    /// Emit a vtable entry for a (struct, interface) pair: one MirFnId
    /// per slot, indexed in the same order as `iface_method_names`.
    /// Idempotent via `vtable_done`.
    fn emit_vtable(
        &mut self,
        struct_id: TypeId,
        struct_args: &[ResolvedType],
        struct_mid: MirTypeId,
        iface_id: TypeId,
        iface_args: &[ResolvedType],
        iface_mid: MirTypeId,
    ) {
        // `insert` returns false if the pair was already present —
        // skip in that case so we don't re-monomorphise the impls.
        if !self.vtable_done.insert((struct_mid, iface_mid)) {
            return;
        }
        let names = self.iface_method_names(iface_id, iface_args);
        let mut slots: Vec<MirFnId> = Vec::with_capacity(names.len());
        for name in &names {
            let fid = self
                .resolve_struct_method(struct_id, struct_args, name)
                .unwrap_or_else(|| {
                    panic!(
                        "struct {:?} missing impl of `{}` for iface {:?}",
                        struct_id, name, iface_id
                    )
                });
            // Method type-args are empty here — interface methods at
            // the slot level can't carry their own generics.
            let mid = self.monomorphize_fn(fid, &[], Some(struct_mid));
            slots.push(mid);
        }
        self.mir.vtables.insert(
            (struct_mid, iface_mid),
            VTable {
                struct_ty: struct_mid,
                interface_ty: iface_mid,
                slots,
            },
        );
    }

    /// Lower `Foo<T> { a: ..., b: ... }`. Field-init exprs get reordered
    /// to match the struct's declaration order (the source can write
    /// them in any order; the MIR allocator wants them in field order).
    fn lower_struct_init(
        &mut self,
        id: TypeId,
        type_args: &[ResolvedType],
        fields: &[(String, TypedExpr)],
        ty: &ResolvedType,
    ) -> Operand {
        let resolved_args: Vec<ResolvedType> = type_args.iter().map(|t| self.subst(t)).collect();
        let mid = self.intern_struct(id, &resolved_args);

        let def = self.hir.structs[&id].clone();
        let mut ordered: Vec<Operand> = Vec::with_capacity(def.fields.len());
        for f in &def.fields {
            let (_, expr) = fields
                .iter()
                .find(|(n, _)| n == &f.name)
                .expect("validator ensures all fields present");
            ordered.push(self.lower_expr(expr));
        }

        // AllocStruct is the canonical allocation site. Whether it goes
        // on the GC heap or as a header-bearing FFI object depends on
        // the struct's StructKind, decided when we interned the type.
        let dst = self.new_temp(ty);
        self.emit(Stmt::Assign(dst, AssignValue::AllocStruct(mid, ordered)));
        Operand::Copy(dst)
    }

    /// `obj.field` field read. Walks through ManagedRef / Pointer wrappers
    /// to find the underlying struct MirTypeDef and uses its declared
    /// field order to pick the index.
    fn lower_member(&mut self, base: &TypedExpr, name: &str, ty: &ResolvedType) -> Operand {
        let recv = self.lower_expr(base);
        let base_ty = self.subst(&base.ty);
        let mty = self.intern_type(&base_ty);

        let mid = match mty {
            MirType::ManagedRef(id) => id,
            MirType::Pointer(inner) => match *inner {
                MirType::ManagedRef(id) => id,
                _ => panic!("member on non-aggregate pointer"),
            },
            other => panic!("member on non-aggregate {:?}", other),
        };

        // Field index is resolved by name against the MirTypeDef we
        // already built when the struct was interned.
        let idx = match self.mir.types.get(&mid) {
            Some(MirTypeDef::Struct { fields, .. }) => fields
                .iter()
                .position(|f| f.name == name)
                .unwrap_or_else(|| panic!("unknown field `{name}`"))
                as u32,
            _ => panic!("member access on non-struct mir type"),
        };

        let dst = self.new_temp(ty);
        self.emit(Stmt::Assign(dst, AssignValue::Field(recv, idx)));
        Operand::Copy(dst)
    }

    /// `x as T` — runtime check + payload extraction, trapping on
    /// mismatch. The exact lowering depends on the scrutinee's
    /// representation:
    /// - Union: tag check via UnionTag/UnionPayload.
    /// - NullableRef: null check.
    /// - Otherwise: validator already proved it; pass-through.
    fn lower_as(&mut self, x: &TypedExpr, target: &ResolvedType, ty: &ResolvedType) -> Operand {
        let value = self.lower_expr(x);
        let scrut_ty = self.subst(&x.ty);
        let target = self.subst(target);

        let scrut_mty = self.intern_type(&scrut_ty);

        let dst = self.new_temp(ty);

        let ok_blk = self.new_block();
        let fail_blk = self.new_block();

        match scrut_mty {
            MirType::Union(union_mid) => {
                // Compare the runtime tag against the constant tag for
                // the target type within this canonicalised union.
                let tag_local = self.new_temp_mty(MirType::Primitive(PrimitiveType::Uint16));
                self.emit(Stmt::Assign(
                    tag_local,
                    AssignValue::UnionTag(value.clone()),
                ));

                let target_tag = self.union_tag_for(&scrut_ty, &target);
                let target_const = self.new_temp_mty(MirType::Primitive(PrimitiveType::Uint16));
                self.emit(Stmt::Assign(
                    target_const,
                    AssignValue::Use(Operand::Const(MirConst::Int(
                        target_tag as i64,
                        PrimitiveType::Uint16,
                    ))),
                ));
                let cmp = self.new_temp_mty(MirType::Primitive(PrimitiveType::Bool));
                self.emit(Stmt::Assign(
                    cmp,
                    AssignValue::Bin(
                        BinOp::Eq,
                        Operand::Copy(tag_local),
                        Operand::Copy(target_const),
                    ),
                ));
                self.terminate(Terminator::CondBr(Operand::Copy(cmp), ok_blk, fail_blk));

                // On match: extract the payload typed as the target's
                // MIR id (falling back to the union's id for primitives).
                self.cur_mut().cur_block = ok_blk;
                let target_mid = match self.intern_type(&target) {
                    MirType::ManagedRef(m) | MirType::Union(m) | MirType::NullableRef(m) => m,
                    _ => union_mid,
                };
                self.emit(Stmt::Assign(
                    dst,
                    AssignValue::UnionPayload(value, target_mid),
                ));
            }
            MirType::NullableRef(_) => {
                // `as null` succeeds when value == null, `as T`
                // succeeds when value != null.
                let cmp = self.new_temp_mty(MirType::Primitive(PrimitiveType::Bool));
                if matches!(target, ResolvedType::Null) {
                    self.emit(Stmt::Assign(
                        cmp,
                        AssignValue::Bin(BinOp::Eq, value.clone(), Operand::Const(MirConst::Null)),
                    ));
                } else {
                    self.emit(Stmt::Assign(
                        cmp,
                        AssignValue::Bin(BinOp::Neq, value.clone(), Operand::Const(MirConst::Null)),
                    ));
                }
                self.terminate(Terminator::CondBr(Operand::Copy(cmp), ok_blk, fail_blk));

                self.cur_mut().cur_block = ok_blk;
                self.emit(Stmt::Assign(dst, AssignValue::Use(value)));
            }
            _ => {
                // Static types — validator already proved the cast,
                // emit it as a pass-through so the trap block is
                // unreachable but still syntactically present.
                self.terminate(Terminator::Goto(ok_blk));
                self.cur_mut().cur_block = ok_blk;
                self.emit(Stmt::Assign(dst, AssignValue::Use(value)));
            }
        }

        // Both ok_blk and the fail trap rejoin here; only ok_blk
        // actually flows through.
        let join = self.new_block();
        let cur_b = self.cur().cur_block;
        if !self.cur().terminated.contains(&cur_b) {
            self.terminate(Terminator::Goto(join));
        }

        self.cur_mut().cur_block = fail_blk;
        self.terminate(Terminator::Trap(TrapReason::AsMismatch));

        self.cur_mut().cur_block = join;
        Operand::Copy(dst)
    }

    /// `x is T` — same shape as `as` but produces a bool instead of
    /// trapping on mismatch.
    fn lower_is(&mut self, x: &TypedExpr, target: &ResolvedType) -> Operand {
        let value = self.lower_expr(x);
        let scrut_ty = self.subst(&x.ty);
        let target = self.subst(target);

        let scrut_mty = self.intern_type(&scrut_ty);
        let dst = self.new_temp_mty(MirType::Primitive(PrimitiveType::Bool));

        match scrut_mty {
            MirType::Union(_) => {
                // Tag equality against the target's slot in the union.
                let target_tag = self.union_tag_for(&scrut_ty, &target);
                let tag_local = self.new_temp_mty(MirType::Primitive(PrimitiveType::Uint16));
                self.emit(Stmt::Assign(tag_local, AssignValue::UnionTag(value)));
                let target_const = self.new_temp_mty(MirType::Primitive(PrimitiveType::Uint16));
                self.emit(Stmt::Assign(
                    target_const,
                    AssignValue::Use(Operand::Const(MirConst::Int(
                        target_tag as i64,
                        PrimitiveType::Uint16,
                    ))),
                ));
                self.emit(Stmt::Assign(
                    dst,
                    AssignValue::Bin(
                        BinOp::Eq,
                        Operand::Copy(tag_local),
                        Operand::Copy(target_const),
                    ),
                ));
            }
            MirType::NullableRef(_) => {
                // `is null` / `is T` reduce to a null comparison.
                let op = if matches!(target, ResolvedType::Null) {
                    BinOp::Eq
                } else {
                    BinOp::Neq
                };
                self.emit(Stmt::Assign(
                    dst,
                    AssignValue::Bin(op, value, Operand::Const(MirConst::Null)),
                ));
            }
            _ => {
                // For monomorphic types the answer is statically known —
                // bake it in as a constant bool.
                let answer = canonical_key(&scrut_ty) == canonical_key(&target);
                self.emit(Stmt::Assign(
                    dst,
                    AssignValue::Use(Operand::Const(MirConst::Bool(answer))),
                ));
            }
        }
        Operand::Copy(dst)
    }

    /// Return the tag value for `target` inside the canonicalised
    /// variant list of `scrut`'s union type. Mirrors the logic of
    /// `intern_union` (flatten + sort by canonical_key + dedupe).
    fn union_tag_for(&mut self, scrut: &ResolvedType, target: &ResolvedType) -> u16 {
        let parts = match scrut {
            ResolvedType::Union(parts) => parts.clone(),
            _ => return 0,
        };
        let mut flat: Vec<ResolvedType> = Vec::new();
        for p in &parts {
            match p {
                ResolvedType::Union(inner) => flat.extend(inner.clone()),
                other => flat.push(other.clone()),
            }
        }
        flat.sort_by(|a, b| canonical_key(a).cmp(&canonical_key(b)));
        flat.dedup();
        flat.iter()
            .position(|p| canonical_key(p) == canonical_key(target))
            .unwrap_or(0) as u16
    }

    /// Lower a `FunctionLiteral`. Three artefacts are produced:
    ///   - a synthetic env struct holding the captured values,
    ///   - a fresh MirFunction whose first param is `env: ManagedRef(env)`
    ///     and whose body reads captures via `Field(env, i)`,
    ///   - an `AllocClosure` instruction at the literal site that bundles
    ///     the function id with the capture values.
    fn lower_closure(
        &mut self,
        _tps: &[TypeParamId],
        params: &[HirParam],
        captures: &[HirCapture],
        body: &HirBlock,
        ty: &ResolvedType,
    ) -> Operand {
        // Build the env struct: one field per capture, in declaration order.
        let mut env_fields: Vec<MirField> = Vec::with_capacity(captures.len());
        for c in captures {
            let resolved = self.subst(&c.ty);
            let mty = self.intern_type(&resolved);
            env_fields.push(MirField {
                name: c.name.clone(),
                ty: mty,
            });
        }
        let env_mid = self.fresh_type_id();
        let env_layout = compute_layout(&env_fields);
        self.mir.types.insert(
            env_mid,
            MirTypeDef::Struct {
                name: format!("closure_env#{}", env_mid.0),
                fields: env_fields.clone(),
                layout: env_layout,
                kind: StructKind::Managed,
            },
        );

        // Capture operands sourced from the *outer* scope — these are
        // what AllocClosure stuffs into the env at the literal site.
        let env_ops: Vec<Operand> = captures
            .iter()
            .map(|c| Operand::Copy(self.lookup(&c.name)))
            .collect();

        let body_mid = self.fresh_fn_id();

        // Stash the outer FnCtx so we can lower the closure body in a
        // fresh context, then restore on the way out.
        let outer_subst = self.cur().type_subst.clone();
        let saved_cur = self.cur.take().expect("must be inside a function");

        self.cur = Some(FnCtx::new());
        self.cur_mut().type_subst = outer_subst;
        self.push_scope();

        // Implicit env parameter goes first.
        let env_local = self.new_local(Some("__env".into()), MirType::ManagedRef(env_mid));
        let mut body_params = vec![env_local];

        // User-declared parameters follow.
        for p in params {
            let resolved = self.subst(&p.ty);
            let mty = if p.is_pointer {
                MirType::Pointer(Box::new(self.intern_type(&resolved)))
            } else {
                self.intern_type(&resolved)
            };
            let id = self.new_local(Some(p.name.clone()), mty);
            self.bind(&p.name, id);
            body_params.push(id);
        }

        // Entry block must exist before we can emit the env-field reads.
        let entry = self.new_block();
        self.cur_mut().cur_block = entry;

        // Materialise each capture as a local read out of the env struct,
        // then bind it under its source name so the body sees it as if
        // it were a normal variable.
        for (i, c) in captures.iter().enumerate() {
            let mty = env_fields[i].ty.clone();
            let id = self.new_local(Some(c.name.clone()), mty);
            self.emit(Stmt::Assign(
                id,
                AssignValue::Field(Operand::Copy(env_local), i as u32),
            ));
            self.bind(&c.name, id);
        }

        // Body itself, with the same fall-through-implies-return rule
        // as a regular function.
        let implicit = self.lower_block(body);
        let cur_b = self.cur().cur_block;
        if !self.cur().terminated.contains(&cur_b) {
            self.terminate(Terminator::Return(implicit));
        }

        // Closure return type comes from the literal's HIR type, which
        // is always Function(_, ret).
        let ret_resolved = match ty {
            ResolvedType::Function(_, ret) => self.subst(ret),
            _ => ResolvedType::Null,
        };
        let return_type = self.intern_type(&ret_resolved);

        self.pop_scope();
        let body_ctx = self.cur.take().unwrap();
        self.mir.functions.insert(
            body_mid,
            MirFunction {
                id: body_mid,
                name: format!("closure#{}", body_mid.0),
                abi: Abi::Otter,
                params: body_params,
                locals: body_ctx.locals,
                blocks: body_ctx.blocks,
                entry,
                return_type,
            },
        );

        // Back to the outer function — emit the AllocClosure.
        self.cur = Some(saved_cur);

        let closure_ty = self.intern_type(&self.subst(ty));
        let dst = self.new_temp_mty(closure_ty);
        self.emit(Stmt::Assign(
            dst,
            AssignValue::AllocClosure(body_mid, env_ops),
        ));
        Operand::Copy(dst)
    }
}

/// Map a HirLiteral plus its inferred ResolvedType to a MirConst. The
/// type tells us which numeric primitive variant to tag the constant
/// with (i32 vs i64, f32 vs f64, etc.).
fn lower_literal(lit: &HirLiteral, ty: &ResolvedType) -> MirConst {
    match lit {
        HirLiteral::Int(v) => {
            // Defaults exist so unannotated integer literals lower
            // without panicking; in practice the validator should always
            // hand us a Primitive type.
            let p = primitive_of(ty).unwrap_or(PrimitiveType::Int32);
            MirConst::Int(*v, p)
        }
        HirLiteral::Float(v) => {
            let p = primitive_of(ty).unwrap_or(PrimitiveType::Float64);
            MirConst::Float(*v, p)
        }
        HirLiteral::String(s) => MirConst::String(s.clone()),
        HirLiteral::Char(c) => MirConst::Char(*c),
        HirLiteral::Bool(b) => MirConst::Bool(*b),
        HirLiteral::Null => MirConst::Null,
    }
}

fn primitive_of(ty: &ResolvedType) -> Option<PrimitiveType> {
    match ty {
        ResolvedType::Primitive(p) => Some(p.clone()),
        _ => None,
    }
}

/// 1-to-1 mapping from HIR binary operators to MIR ones.
fn map_binop(op: &BinaryOperator) -> BinOp {
    match op {
        BinaryOperator::Add => BinOp::Add,
        BinaryOperator::Sub => BinOp::Sub,
        BinaryOperator::Mul => BinOp::Mul,
        BinaryOperator::Div => BinOp::Div,
        BinaryOperator::Mod => BinOp::Mod,
        BinaryOperator::And => BinOp::And,
        BinaryOperator::Or => BinOp::Or,
        BinaryOperator::Eq => BinOp::Eq,
        BinaryOperator::Neq => BinOp::Neq,
        BinaryOperator::Lt => BinOp::Lt,
        BinaryOperator::Le => BinOp::Le,
        BinaryOperator::Gt => BinOp::Gt,
        BinaryOperator::Ge => BinOp::Ge,
    }
}

fn map_unop(op: &UnaryOperator) -> UnOp {
    match op {
        UnaryOperator::Neg => UnOp::Neg,
        UnaryOperator::Not => UnOp::Not,
    }
}
