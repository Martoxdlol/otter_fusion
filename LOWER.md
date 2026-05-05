# Lower: HIR → MIR Implementation Guide

This document is the implementation guide for `src/lower.rs`. It assumes the
HIR shape in `src/hir.rs` and the MIR shape in `src/mir.rs` are fixed, and
that the input `Hir` has already passed `validator.rs` (so every type is
resolved, every method is dispatchable, every variable has a known type).

The lowerer's job is to produce a `MirProgram` whose generics are gone,
whose control flow is a CFG of basic blocks, whose dispatch is either a
`Static` `MirFnId`, an `Indirect` operand, or a `Virtual` slot, and whose
allocations are explicit.

> Out of scope for this guide: `for-in` over `Iterator<T>`, list literals
> (`ExprKind::LiteralList`), and map literals (`ExprKind::LiteralMap`).
> Treat them as `unimplemented!()` call sites — everything else below is
> the final shape.

---

## 1. Required prerequisite: hashable `ResolvedType`

`ResolvedType` already derives `Hash + Eq + PartialEq` (`hir.rs:58`). The
monomorphization caches below key on `(HirId, Vec<ResolvedType>)`, so do
not break those derives.

If you change `ResolvedType` to carry a `Box<...>` of something that isn't
`Hash`, you'll get cryptic trait errors in the lowerer. Keep it hashable.

---

## 2. State carried by `Lower`

Replace the current empty `Lower` body with the following state. Keep the
public API as it is (`Lower::new(hir) -> Self`, `Lower::lower(self) -> MirProgram`).

```rust
pub struct Lower {
    hir: Hir,
    mir: MirProgram,

    // -------- Type interning (monomorphization for types) --------
    // Concrete (HirTypeId, type args) → MirTypeId.
    struct_cache:    HashMap<(TypeId, Vec<ResolvedType>), MirTypeId>,
    interface_cache: HashMap<(TypeId, Vec<ResolvedType>), MirTypeId>,
    union_cache:     HashMap<Vec<ResolvedType>, MirTypeId>, // canonicalized
    closure_cache:   HashMap<ClosureKey, MirTypeId>,        // see §9

    // -------- Function monomorphization --------
    fn_cache: HashMap<(FnId, Vec<ResolvedType>), MirFnId>,
    fn_queue: Vec<MonoJob>,                  // FIFO worklist

    // -------- ID allocators --------
    next_type_id: u32,
    next_fn_id:   u32,

    // -------- Per-function lowering state (reset each function) --------
    cur: Option<FnCtx>,
}

struct MonoJob {
    target_id:    MirFnId,                    // pre-allocated
    hir_fn:       FnId,
    type_args:    Vec<ResolvedType>,          // already substituted (no TypeParam)
    self_type:    Option<MirTypeId>,          // for methods
}

struct FnCtx {
    locals:      HashMap<LocalId, MirLocal>,
    blocks:      HashMap<BlockId, MirBlock>,
    next_local:  u32,
    next_block:  u32,
    cur_block:   BlockId,                     // where statements append

    // Lexical scope for `Variable` lookup.
    scopes:      Vec<HashMap<String, LocalId>>,

    // Generic substitution for the current monomorphization.
    type_subst:  HashMap<TypeParamId, ResolvedType>,

    // Loop targets for break/continue.
    loops:       Vec<LoopFrame>,
}

struct LoopFrame { continue_to: BlockId, break_to: BlockId }
```

`ClosureKey` is described in §9.

`Lower::new` should initialize `mir = MirProgram::new()` and all the maps
to empty. Do not allocate any IDs yet.

---

## 3. Top-level driver

`Lower::lower` is a fixed-point worklist:

1. Find `main` in `self.hir.functions`. It's the unique free function whose
   `name == "main"`, no `owner`, no `type_params`, no params. Allocate a
   `MirFnId` for it, set `self.mir.entry`, push a `MonoJob` for it.
2. Loop: while `self.fn_queue` is non-empty, pop a job and call
   `self.lower_function(job)`. Lowering a body discovers new
   `Call(...)` / `StructInit(...)` / `As(_, Interface(...))` /
   `Member(_, m)` interface calls; each of those calls
   `intern_*` / `monomorphize_fn` which may push new jobs.
3. Once empty, return `self.mir`.

The order matters: do **not** eagerly emit every struct in `self.hir`. Only
the reachable monomorphizations end up in the MIR, and that's the whole
point of this pass.

---

## 4. Resolving generic types

Every place the HIR mentions `ResolvedType` may carry a `TypeParam`. The
substitution table lives in `FnCtx::type_subst`. Implement a single helper:

```rust
fn subst(&self, ty: &ResolvedType) -> ResolvedType {
    match ty {
        ResolvedType::TypeParam(p) => {
            // Must be present — validator guarantees it.
            self.cur().type_subst[p].clone()
        }
        ResolvedType::Struct(id, args)    => ResolvedType::Struct(*id,    args.iter().map(|a| self.subst(a)).collect()),
        ResolvedType::Interface(id, args) => ResolvedType::Interface(*id, args.iter().map(|a| self.subst(a)).collect()),
        ResolvedType::Union(parts)        => ResolvedType::Union(parts.iter().map(|a| self.subst(a)).collect()),
        ResolvedType::Function(args, ret) => ResolvedType::Function(
            args.iter().map(|a| self.subst(a)).collect(),
            Box::new(self.subst(ret)),
        ),
        ResolvedType::Primitive(_) | ResolvedType::Null => ty.clone(),
    }
}
```

Always run `subst` on every `ResolvedType` you read out of the HIR before
doing anything with it. After `subst`, no `TypeParam` may remain — if one
does, it's a validator bug, panic.

---

## 5. Lowering types: `intern_type`

This is the workhorse. It maps a fully-substituted `ResolvedType` to a
`MirType`.

```rust
fn intern_type(&mut self, ty: &ResolvedType) -> MirType {
    match ty {
        ResolvedType::Primitive(p) => MirType::Primitive(p.clone()),

        ResolvedType::Struct(id, args) => {
            let mid = self.intern_struct(*id, args);
            MirType::ManagedRef(mid)            // see §5.1 for extern
        }

        ResolvedType::Interface(id, args) => {
            let mid = self.intern_interface(*id, args);
            MirType::ManagedRef(mid)
        }

        ResolvedType::Union(parts) => self.intern_union(parts),

        ResolvedType::Function(args, ret) => {
            // Default to managed closure. FFI-typed function values are
            // produced by extern fn pointers — handled at call sites and
            // when the source had `extern` on the declaration. See §10.
            let mid = self.intern_closure_type(args, ret);
            MirType::Closure(mid)
        }

        ResolvedType::Null      => MirType::Primitive(PrimitiveType::Bool), // unreachable in well-typed
        ResolvedType::TypeParam(_) => unreachable!("subst must run first"),
    }
}
```

### 5.1 Structs

```rust
fn intern_struct(&mut self, id: TypeId, args: &[ResolvedType]) -> MirTypeId {
    let key = (id, args.to_vec());
    if let Some(m) = self.struct_cache.get(&key) { return *m; }

    let mid = self.fresh_type_id();
    self.struct_cache.insert(key.clone(), mid);   // insert *before* recursing

    let s = self.hir.structs[&id].clone();
    // Push a fresh subst for this struct's params so field types can use them.
    let mut local_subst = HashMap::new();
    for (p, a) in s.type_params.iter().zip(args) {
        local_subst.insert(*p, a.clone());
    }
    let with_subst = |this: &mut Self, ty: &ResolvedType| {
        // temporarily layer local_subst on top of cur subst
        // simplest: substitute manually here without touching FnCtx
        substitute_with(ty, &local_subst, &this.fn_subst_or_empty())
    };

    let mut fields = Vec::with_capacity(s.fields.len());
    for f in &s.fields {
        let resolved = with_subst(self, &f.ty);
        let mty = if f.is_pointer {
            MirType::Pointer(Box::new(self.intern_type(&resolved)))
        } else {
            self.intern_type(&resolved)
        };
        fields.push(MirField { name: f.name.clone(), ty: mty });
    }

    let layout = compute_layout(&fields);          // see §5.4
    let kind = if s.is_extern { StructKind::Extern } else { StructKind::Managed };
    self.mir.types.insert(mid, MirTypeDef::Struct {
        name: format!("{}<{}>", s.name, fmt_args(args)),
        fields, layout, kind,
    });
    mid
}
```

Insert the `MirTypeId` into the cache **before** recursing on the fields,
otherwise a struct that recursively references itself through a managed
ref will loop. The placeholder is fine because we only insert the
`MirTypeDef` after the fields are built — readers of `mir.types` get the
final value, and the `mid` is stable.

### 5.2 Interfaces

Treat interfaces as `MirTypeDef::Struct` with no fields and `StructKind::Managed`,
named `iface:Foo<...>`. The MIR doesn't store interface members directly —
those live in vtables (§7). This way `MirType::ManagedRef(mid)` is uniform
for both struct refs and interface fat refs (the fat pointer is implicit:
the managed ref always carries the type id at offset −1, see `mir.rs:43`).

### 5.3 Unions

```rust
fn intern_union(&mut self, parts: &[ResolvedType]) -> MirType {
    // Flatten nested unions, dedupe, canonical sort. The canonicalized
    // vector is the cache key.
    let mut flat = Vec::new();
    for p in parts {
        let p = self.subst(p);                    // substitute again to be safe
        match p {
            ResolvedType::Union(inner) => flat.extend(inner),
            other                      => flat.push(other),
        }
    }
    flat.sort_by(canonical_order);
    flat.dedup();

    // T | null special case → NullableRef. This is the layout
    // optimization called out in MIR.md §4.
    if flat.len() == 2 && flat.iter().any(|t| matches!(t, ResolvedType::Null)) {
        let payload = flat.iter().find(|t| !matches!(t, ResolvedType::Null)).unwrap();
        if let MirType::ManagedRef(mid) = self.intern_type(payload) {
            return MirType::NullableRef(mid);
        }
    }

    if let Some(m) = self.union_cache.get(&flat) {
        return MirType::Union(*m);
    }

    let mid = self.fresh_type_id();
    self.union_cache.insert(flat.clone(), mid);

    let mut variants = Vec::with_capacity(flat.len());
    for (i, p) in flat.iter().enumerate() {
        variants.push(UnionVariant {
            tag: i as u16,
            ty: self.intern_type(p),
        });
    }
    let layout = compute_union_layout(&variants);
    self.mir.types.insert(mid, MirTypeDef::Union { variants, layout });
    MirType::Union(mid)
}
```

The canonical order must be deterministic and content-based. Sort by a
discriminator on `ResolvedType` (e.g., tag-then-payload textual form) so
`A | B` and `B | A` map to the same `MirTypeId`. Two unions only share a
`MirTypeId` when their canonicalized variant lists are equal.

The tag value for a variant is its **index in the canonicalized vector**.
Save this; `is`/`as` lowering looks it up by computing the canonicalized
form of the queried type and finding its position.

### 5.4 Layout

`compute_layout` and `compute_union_layout` use the standard alignment
rule:

- `align = max(field aligns, 1)` (or `align(usize)` for unions, see
  `mir.rs:71`).
- `size = sum of fields, each padded up to next alignment boundary,
  rounded up to align`.

Primitives: `i8/u8` is 1, `i16/u16` is 2, `i32/u32/f32/char` is 4,
`i64/u64/f64` is 8, `bool` is 1, `string` is `ManagedRef` (pointer-sized).
`ManagedRef`, `Pointer`, `FnPtr`, `Closure`, `NullableRef` are pointer-sized.
`Union(mid)` is `compute_union_layout` of the referenced variants
(tag is `u16`, but pad to align of largest payload; size is `2 + max payload size`,
rounded to align). The union's tag lives at offset 0, payload starts at the
next aligned offset.

This is enough for codegen; you don't need clever packing.

---

## 6. Lowering function bodies

```rust
fn lower_function(&mut self, job: MonoJob) {
    let hir_fn = self.hir.functions[&job.hir_fn].clone();
    self.cur = Some(FnCtx::new());

    // Build subst from the function's own type params + the receiver's args.
    let mut subst = HashMap::new();
    for (p, a) in hir_fn.type_params.iter().zip(&job.type_args) {
        subst.insert(*p, a.clone());
    }
    self.cur_mut().type_subst = subst;

    // Allocate parameter locals.
    let mut params = Vec::new();
    self.push_scope();
    if hir_fn.has_self {
        let self_ty = MirType::ManagedRef(job.self_type.expect("method needs self"));
        let id = self.new_local(Some("self".into()), self_ty);
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

    // Allocate entry block, lower the body into it.
    let entry = self.new_block();
    self.cur_mut().cur_block = entry;

    let body = hir_fn.body.expect("validator rejected abstract bodies in lower");
    let result = self.lower_block(&body);

    // Terminate however the body left us — see §6.1.
    self.terminate_with_return(&hir_fn.return_type, result);

    let ret_resolved = self.subst(&hir_fn.return_type);
    let return_type = self.intern_type(&ret_resolved);

    let abi = if /* hir_fn was declared extern */ { Abi::Extern } else { Abi::Otter };
    let ctx = self.cur.take().unwrap();
    self.mir.functions.insert(job.target_id, MirFunction {
        id: job.target_id,
        name: format!("{}<{}>", hir_fn.name, fmt_args(&job.type_args)),
        abi,
        params,
        locals: ctx.locals,
        blocks: ctx.blocks,
        entry,
        return_type,
    });
}
```

(HIR doesn't currently carry an `abi` field. Either add one when you fold
`extern` into `HirFunction`, or maintain a side set of `extern` `FnId`s in
`Hir`. Treat absence as `Abi::Otter`.)

### 6.1 Block return discipline

`HirBlock { statements, returns }`. Each `HirStatement::Return(_)` and the
implicit `returns` slot both eventually become a `Terminator::Return(_)`
in some basic block. Rules:

- Lower statements in order; each may install new blocks and reposition
  `cur_block`.
- After lowering the last statement, if `cur_block`'s terminator is still
  unset (i.e. the block "fell through"), emit code for `returns` if it
  exists, else emit `Terminator::Return(None)`.
- If lowering produced an unreachable trailing block (e.g. every branch
  already returned), it's fine to leave it un-terminated as long as you
  later fix it up. The cheap trick is: any block left without a
  terminator at the end of a function gets `Terminator::Unreachable`.

Use a `Option<Terminator>` slot per `MirBlock` during construction; only
when finalizing do you panic if it's still `None` after the unreachable
patch above.

---

## 7. Statement lowering

```rust
fn lower_stmt(&mut self, s: &HirStatement) {
    match s {
        HirStatement::Expr(e) => { let _ = self.lower_expr(e); }

        HirStatement::VarDecl(name, ty, init) => {
            let resolved = self.subst(ty);
            let mty = self.intern_type(&resolved);
            let id = self.new_local(Some(name.clone()), mty.clone());
            if let Some(e) = init {
                let op = self.lower_expr(e);
                self.emit(Stmt::Assign(id, AssignValue::Use(op)));
            }
            self.bind(name, id);
        }

        HirStatement::Return(opt) => {
            let op = opt.as_ref().map(|e| self.lower_expr(e));
            self.terminate(Terminator::Return(op));
            // Any code after a return goes into a fresh unreachable block.
            let dead = self.new_block();
            self.cur_mut().cur_block = dead;
        }

        HirStatement::While(cond, body) => self.lower_while(cond, body),

        HirStatement::For(_, _, _) => unimplemented!("for-in: out of scope"),

        HirStatement::Break => {
            let to = self.cur().loops.last().expect("break outside loop").break_to;
            self.terminate(Terminator::Goto(to));
            let dead = self.new_block();
            self.cur_mut().cur_block = dead;
        }
        HirStatement::Continue => {
            let to = self.cur().loops.last().expect("continue outside loop").continue_to;
            self.terminate(Terminator::Goto(to));
            let dead = self.new_block();
            self.cur_mut().cur_block = dead;
        }
    }
}
```

### 7.1 `while`

```text
header:        cur_block falls through into header (Goto)
header block:  evaluate cond → CondBr(cond, body, exit)
body block:   lower body; at end Goto(header)
exit block:   becomes the new cur_block
```

```rust
fn lower_while(&mut self, cond: &TypedExpr, body: &HirBlock) {
    let header = self.new_block();
    let body_b = self.new_block();
    let exit   = self.new_block();

    self.terminate(Terminator::Goto(header));

    self.cur_mut().cur_block = header;
    let c = self.lower_expr(cond);
    self.terminate(Terminator::CondBr(c, body_b, exit));

    self.cur_mut().cur_block = body_b;
    self.cur_mut().loops.push(LoopFrame { continue_to: header, break_to: exit });
    self.lower_block_inline(body);
    self.cur_mut().loops.pop();
    self.terminate(Terminator::Goto(header));

    self.cur_mut().cur_block = exit;
}
```

`lower_block_inline` walks `body.statements` and the optional implicit
`returns` (which for a `while`-body is always `None` in practice; if
present, treat it as a final `Expr` statement).

---

## 8. Expression lowering

Every `lower_expr` returns an `Operand`. When the expression has any
non-trivial structure, it materializes a fresh local first and returns
`Operand::Copy(local)`. Constants return `Operand::Const(...)` directly.

Skeleton:

```rust
fn lower_expr(&mut self, e: &TypedExpr) -> Operand {
    match &e.kind {
        ExprKind::Literal(lit) => Operand::Const(lower_literal(lit, &e.ty)),

        ExprKind::Variable(name) => {
            let id = self.lookup(name);
            Operand::Copy(id)
        }

        ExprKind::BinaryOp(l, op, r) => {
            // Short-circuit for And/Or — see §8.1.
            if matches!(op, BinaryOperator::And | BinaryOperator::Or) {
                return self.lower_short_circuit(l, op, r, &e.ty);
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

        ExprKind::If(cond, then_b, else_b) => self.lower_if(cond, then_b, else_b.as_deref(), &e.ty),

        ExprKind::Block(b) => self.lower_block_value(b, &e.ty),

        ExprKind::Call(callee, type_args, args) => self.lower_call(callee, type_args, args, &e.ty),

        ExprKind::StructInit(id, type_args, fields) => self.lower_struct_init(*id, type_args, fields, &e.ty),

        ExprKind::Member(base, name) => self.lower_member(base, name, &e.ty),

        ExprKind::As(x, target) => self.lower_as(x, target, &e.ty),
        ExprKind::Is(x, target) => self.lower_is(x, target, &e.ty),

        ExprKind::FunctionLiteral(tps, params, captures, body) =>
            self.lower_closure(tps, params, captures, body, &e.ty),

        ExprKind::LiteralList(_) | ExprKind::LiteralMap(_) =>
            unimplemented!("list/map literals: out of scope"),
    }
}
```

`lower_literal` maps `HirLiteral` → `MirConst`. The `e.ty` tells you which
primitive variant for `Int`/`Float` (e.g. `Int(42)` with `e.ty == i32` →
`MirConst::Int(42, PrimitiveType::Int32)`). For `HirLiteral::Null`, emit
`MirConst::Null`.

### 8.1 Short-circuit `&&` / `||`

```text
let dst : Bool;
let a = lower(l);
                          // for &&: if a then go to right, else dst=false
                          // for ||: if a then dst=true,    else go to right
CondBr(a, then_blk, else_blk)
then_blk: dst = lower(r);  Goto(join)
else_blk: dst = false/true; Goto(join)
join:    cur_block = join; result = Copy(dst)
```

The result temporary's type is `bool` (the validator guarantees both
sides are bool).

### 8.2 `if`-expression

```text
let dst : T = uninit;
let c = lower(cond);
CondBr(c, then_b, else_b);
then_b: t = lower(then_block); dst = t; Goto(join)
else_b: e = lower(else_block); dst = e; Goto(join)   // or just Goto(join) if no else
join:   cur_block = join; result = Copy(dst)
```

If there's no `else` branch the surface type is `T | null` (the validator
ensures that), so `dst` has a `NullableRef` or `Union` type and the
fall-through arm assigns `null`.

### 8.3 Block as value

`lower_block_value` lowers all statements, then if `returns` is present,
emits `dst = lower_expr(returns)` and yields `Copy(dst)`. If `returns`
is absent the block has no value — only legal where `e.ty` is the unit-y
case (`null` or unused), so emit `Const(Null)` or panic with "validator
should have rejected".

### 8.4 Calls

```rust
fn lower_call(
    &mut self,
    callee: &TypedExpr,
    type_args: &[ResolvedType],
    args: &[TypedExpr],
    ret_ty: &ResolvedType,
) -> Operand {
    // Resolve every type argument under the current subst.
    let resolved_args: Vec<_> = type_args.iter().map(|t| self.subst(t)).collect();

    let arg_ops: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();

    let dst = self.new_temp(ret_ty);

    let callee_node = match &callee.kind {
        // Free function reference, e.g. `foo<T>(...)`.
        // The validator stores the resolved FnId on the Variable kind via
        // some side channel (look up the function by name + module).
        // In practice you'll likely have ExprKind::Variable for the bare
        // function name; treat it as a static call.
        ExprKind::Variable(name) if let Some(fid) = self.resolve_free_fn(name) => {
            let mid = self.monomorphize_fn(fid, &resolved_args, /*self_ty=*/ None);
            Callee::Static(mid)
        }

        // Method call: `recv.method(args)`. The HIR currently models this
        // as Call(Member(recv, "method"), type_args, args).
        ExprKind::Member(recv, method_name) => {
            let recv_op = self.lower_expr(recv);
            let recv_ty = self.subst(&recv.ty);
            return self.lower_method_call(recv_op, recv_ty, method_name, &resolved_args, arg_ops, ret_ty);
        }

        // First-class function value (closure or extern fn ptr).
        _ => {
            let f = self.lower_expr(callee);
            Callee::Indirect(f)
        }
    };

    self.emit(Stmt::Assign(dst, AssignValue::Call(callee_node, arg_ops)));
    Operand::Copy(dst)
}
```

#### Method dispatch (`lower_method_call`)

The receiver type after substitution is one of:

- **Concrete struct** `Struct(sid, sargs)`: pick the right `FnId` from
  `HirStruct::methods` (inline) or `specialised_methods` (extend blocks).
  For extends, find the entry whose `target_args` unify with `sargs` after
  substituting fresh extend-level type vars. Validator guarantees exactly
  one match. Then `monomorphize_fn(fid, type_args, Some(self_mid))` and
  emit `Callee::Static`.
- **Interface** `Interface(iid, iargs)`: look up the slot index for
  `method_name` in the (post-substitution) interface, intern both the
  interface and any concrete struct seen at construction sites that
  implements it (vtables are emitted on demand when a struct is first
  coerced into an interface — see §11). Emit
  `Callee::Virtual(recv_op, interface_mid, slot)`.
- **Union**: not directly callable (validator rejects unless every variant
  has the method via a common interface, in which case the type is
  effectively the interface — handle as Interface).

Slot indices for interface methods are assigned **deterministically**:
sort the method names of the interface (and its inherited methods,
flattened) lexicographically and use the index. The same ordering must
be used in §11 when populating vtables.

### 8.5 `StructInit`

```rust
fn lower_struct_init(
    &mut self,
    id: TypeId, type_args: &[ResolvedType],
    fields: &[(String, TypedExpr)],
    ty: &ResolvedType,
) -> Operand {
    let resolved_args: Vec<_> = type_args.iter().map(|t| self.subst(t)).collect();
    let mid = self.intern_struct(id, &resolved_args);

    // Reorder field exprs to match the struct's field declaration order.
    let def = self.hir.structs[&id].clone();
    let mut ordered = Vec::with_capacity(def.fields.len());
    for f in &def.fields {
        let (_, expr) = fields.iter().find(|(n, _)| n == &f.name)
            .expect("validator ensures all fields present");
        ordered.push(self.lower_expr(expr));
    }

    let dst = self.new_temp(ty);
    self.emit(Stmt::Assign(dst, AssignValue::AllocStruct(mid, ordered)));
    Operand::Copy(dst)
}
```

`AllocStruct` is the canonical allocation site: GC-managed if the
struct's `kind` is `Managed`, header-bearing extern if `Extern` (the
runtime distinguishes by address per `mir.rs:43`).

### 8.6 `Member` (field read)

```rust
fn lower_member(&mut self, base: &TypedExpr, name: &str, ty: &ResolvedType) -> Operand {
    let recv = self.lower_expr(base);
    let base_ty = self.subst(&base.ty);
    let mid = match self.intern_type(&base_ty) {
        MirType::ManagedRef(id) | MirType::Pointer(box MirType::ManagedRef(id)) => id,
        // Pointer to non-managed extern struct: same idea.
        MirType::Pointer(box MirType::Primitive(_)) => unreachable!("primitives have no fields"),
        other => panic!("member on non-aggregate {other:?}"),
    };
    let def = &self.mir.types[&mid];
    let idx = def.field_index(name).expect("validator ensures field exists");
    let dst = self.new_temp(ty);
    self.emit(Stmt::Assign(dst, AssignValue::Field(recv, idx)));
    Operand::Copy(dst)
}
```

`AssignValue::Field` reads field `idx`. Field assignment (e.g.
`obj.x = expr`) doesn't currently exist as an `HirStatement` variant —
if it shows up later it becomes a separate `Stmt::Store(local, idx, op)`
variant in MIR; not needed for this pass.

### 8.7 `as` and `is`

`as T` is "the value's runtime type matches `T`, otherwise trap".
`is T` is the same test producing a bool. Both desugar against the
union/nullable/interface representation:

```text
// is X
match value's MirType:
  Union(_):       UnionTag(value) == tag_of(canonicalized(X))
  NullableRef(p): if X is Null         → value == null
                  if X matches p       → value != null
                  else                 → false  (validator should have rejected)
  ManagedRef(s):  if X == s            → true (always)
                  if X is an interface → check vtable existence at runtime
                                         (compile-time: emit a const true/false
                                          based on whether s implements X — the
                                          validator already knows the answer
                                          for monomorphic types, but generics
                                          require a runtime tag check via
                                          the type-id slot at offset −1).

// as X
1. Lower `is X` into a bool temp `t`.
2. CondBr(t, ok_blk, fail_blk)
3. fail_blk: Terminator::Trap(TrapReason::AsMismatch)
4. ok_blk:   payload extraction depending on representation:
             Union      → UnionPayload(value, mir_id_of(X))
             NullableRef→ Use(value)  (its type narrows to ManagedRef)
             ManagedRef → Use(value)
```

For unions, the tag for `X` is the position of `canonicalized(subst(X))`
in the canonicalized variant list (see §5.3). If `X` itself is a union
that's a subset of the scrutinee's union, lower it as a chain of tag
checks: build a `Switch` whose arms are the tags belonging to `X`'s
canonical members and whose default is the fail block.

For interfaces, you only need the **runtime** type-id check when the
scrutinee's static type is an interface or a union containing both the
interface and a concrete struct. When the static type is a single
concrete struct the answer is constant and the validator's already
rejected the impossible cases — just emit the constant bool.

---

## 9. Closures and function literals

```rust
struct ClosureKey {
    fn_id: MirFnId,                  // monomorphized body
    env:   Vec<MirType>,             // captured field types
}
```

To lower `FunctionLiteral(tps, params, captures, body)`:

1. **Synthesize the body as a normal function.** Its parameters are:
   - an implicit `env: ManagedRef(env_struct_mid)` first parameter,
   - then the user-declared `params`.
   The body reads each capture by `Field(env, i)`.
2. **Synthesize the env struct.** Fields are the `HirCapture`s (each `name` and
   substituted `ty`) in declaration order. Insert as a new
   `MirTypeDef::Struct { kind: Managed }` and remember its `MirTypeId`.
3. **Allocate at the literal site.** Emit `AssignValue::AllocClosure(body_fn_id, env_field_operands)`.
   The env operands are `Copy(local_id)` of each captured local in the
   *outer* scope — which is why captures must be looked up in the
   surrounding scope chain.
4. **Cache** by `ClosureKey` so identical literals reuse the same body
   `MirFnId` and env type. (In practice each literal is at a different
   source location and won't collide; the cache is mostly to dedupe
   when the same generic function is monomorphized at the same args
   twice.)

The body of a closure is an ordinary monomorphization job — push it onto
`fn_queue` and let the worklist handle it. Its `MirFnId` is the one
referenced by `AllocClosure`.

A "function literal" with no captures still goes through this path; the
env struct will have zero fields and the resulting `Closure` is
indistinguishable from a small fn pointer plus an empty env.

---

## 10. FFI calls

A free function whose `HirFunction` was declared `extern` must:

- Be monomorphized just like any other function — but the body is `None`,
  so `lower_function` skips body emission and inserts a `MirFunction`
  with empty `blocks` and `entry = BlockId(0)` (the placeholder). The
  backend treats `Abi::Extern` + empty body as "linker-resolved".
- At call sites it appears as `Callee::Static(MirFnId)` with the
  arguments lowered normally. Pointer arguments come from values already
  typed as `*T` in HIR (`HirParam::is_pointer`).
- Insert a null-deref guard for any non-nullable `*T` argument: if
  there's no `| null` in the source type, emit `CondBr(arg == null, trap, ok)`;
  `trap` is `Terminator::Trap(TrapReason::NullDeref)`. Skip the guard
  when the type is already `NullableRef` — passing null is legal there.

`extern struct` types pass by value (the struct is plain C-ABI). They
intern as `StructKind::Extern`. The validator already rejects extern
structs in generics, in interfaces, and in casts.

---

## 11. Vtables

A vtable is needed for every `(struct_mid, interface_mid)` pair that
flows through one of:

- `As(x, Interface(...))` where `x`'s static type is a struct,
- `StructInit` whose result is consumed at an interface-typed slot
  (parameter, var, return),
- A method call on an interface receiver where the receiver came from a
  struct.

The simplest sound rule: when interning a struct that `implements` an
interface (post-substitution), eagerly emit the vtable. Concretely, in
`intern_struct`, after inserting the type, walk `HirStruct::implements`,
substitute the interface's type args, intern each interface, and for each
emit:

```rust
let slots: Vec<MirFnId> = sorted_iface_methods
    .iter()
    .map(|m| {
        // Find the struct's implementation of `m`. Validator guarantees
        // exactly one match across `methods` and `specialised_methods`.
        let fid = self.resolve_method(struct_id, &struct_args, m);
        self.monomorphize_fn(fid, /*method type args*/ &[], Some(struct_mid))
    })
    .collect();

self.mir.vtables.insert((struct_mid, interface_mid), VTable {
    struct_ty: struct_mid, interface_ty: interface_mid, slots,
});
```

The `sorted_iface_methods` list is the same lexicographic flattening from
§8.4; the *N*-th entry's `MirFnId` lives in `slots[N]`, and that's the
slot index `Callee::Virtual(_, _, N)` uses.

If an interface inherits methods from a parent (`HirInterface::extends`),
include the parent methods in the flattening — recursively, with the
parent's substituted type args.

---

## 12. ID and block helpers

```rust
fn fresh_type_id(&mut self) -> MirTypeId {
    let id = MirTypeId(self.next_type_id); self.next_type_id += 1; id
}
fn fresh_fn_id(&mut self) -> MirFnId {
    let id = MirFnId(self.next_fn_id); self.next_fn_id += 1; id
}

fn new_local(&mut self, name: Option<String>, ty: MirType) -> LocalId {
    let id = LocalId(self.cur().next_local);
    self.cur_mut().next_local += 1;
    self.cur_mut().locals.insert(id, MirLocal { id, name, ty });
    id
}
fn new_temp(&mut self, ty: &ResolvedType) -> LocalId {
    let r = self.subst(ty);
    let mty = self.intern_type(&r);
    self.new_local(None, mty)
}

fn new_block(&mut self) -> BlockId {
    let id = BlockId(self.cur().next_block);
    self.cur_mut().next_block += 1;
    self.cur_mut().blocks.insert(id, MirBlock {
        id, stmts: Vec::new(),
        terminator: Terminator::Unreachable,    // placeholder
    });
    id
}

fn emit(&mut self, s: Stmt) {
    let b = self.cur().cur_block;
    self.cur_mut().blocks.get_mut(&b).unwrap().stmts.push(s);
}
fn terminate(&mut self, t: Terminator) {
    let b = self.cur().cur_block;
    self.cur_mut().blocks.get_mut(&b).unwrap().terminator = t;
}
```

The placeholder `Unreachable` terminator is overwritten by every code
path that finishes a block normally. Anything not overwritten is
genuinely unreachable code (e.g. the dead block after a `Return`), and
leaving it as `Unreachable` is correct.

---

## 13. Monomorphization of functions

```rust
fn monomorphize_fn(
    &mut self,
    fid: FnId,
    type_args: &[ResolvedType],
    self_mid: Option<MirTypeId>,
) -> MirFnId {
    // Substitute type_args under the current subst, then key on the result.
    let resolved: Vec<_> = type_args.iter().map(|t| self.subst(t)).collect();
    let key = (fid, resolved.clone());

    if let Some(m) = self.fn_cache.get(&key) { return *m; }

    let mid = self.fresh_fn_id();
    self.fn_cache.insert(key, mid);
    self.fn_queue.push(MonoJob {
        target_id: mid,
        hir_fn:    fid,
        type_args: resolved,
        self_type: self_mid,
    });
    mid
}
```

**Important**: insert into `fn_cache` *before* pushing the job, so that a
recursive call from within the lowering of this same function reuses the
allocated `MirFnId` instead of allocating a new one.

---

## 14. Driving order and entry point

Inside `Lower::lower`, after pushing `main`:

```rust
while let Some(job) = self.fn_queue.pop() {
    self.lower_function(job);
}
self.mir
```

That's it. Every type the worklist needs becomes interned the first time
it's encountered; every function it needs gets queued.

---

## 15. What the validator already guarantees (do not re-check)

- Every name resolves; every method dispatch has exactly one target.
- Every type argument count matches its parameter count.
- All variable types are inferred and stored in `TypedExpr::ty`.
- Every `as`/`is` target type is reachable from the scrutinee's type.
- Captured variables are accurately listed in `HirCapture`.
- `extern struct` doesn't show up in places it can't (generics,
  interfaces, casts).
- Implicit returns are well-typed, control flow has no fallthroughs into
  non-unit returns.

The lowerer leans on all of these. If a `panic!("validator should have…")`
fires in production, the bug is upstream.

---

## 16. Putting it together — a worked sketch

For a function

```text
fn add<T: Num>(a: T, b: T) -> T {
    a + b
}

fn main() {
    let r = add<i32>(1, 2)
}
```

the lowerer:

1. Pushes `main` (no type args).
2. Lowers `main`: `VarDecl r ← Call(add, [i32], [1, 2])`.
3. Resolving the call: `monomorphize_fn(add_fid, [i32], None)` →
   queues a job for `add<i32>`. Emits
   `Stmt::Assign(r, AssignValue::Call(Static(add_i32_mid), [Const(Int(1, i32)), Const(Int(2, i32))]))`.
4. Terminates main with `Return(None)`.
5. Pops the `add<i32>` job. Subst `T = i32`. Emits a function with
   params `a, b: i32`, single block:
   - `t = Bin(Add, Copy(a), Copy(b))`
   - `Return(Some(Copy(t)))`.
6. Worklist empty → done.

That's the whole loop. Every other case in the language is a more
elaborate version of "intern any type you touch, queue any function you
call, flatten anything that branches".

---

## 17. Testing recipe

A useful smoke test for each example program in `examples/`:

1. Parse → HIR → validate.
2. `Lower::new(hir).lower()`.
3. Walk the resulting `MirProgram` and assert:
   - Every `MirFnId` referenced in any `Callee::Static` exists in
     `mir.functions`.
   - Every `MirTypeId` referenced in any `MirType::ManagedRef` /
     `Pointer` / `Union` / `Closure` / `NullableRef` exists in
     `mir.types`.
   - `mir.entry` is a key in `mir.functions`.
   - For every `Callee::Virtual(_, iface_mid, slot)`, every concrete
     struct that flows there has a vtable entry whose `slots[slot]` is a
     valid `MirFnId`.
   - No block has the placeholder `Terminator::Unreachable` *unless*
     it's reachable only from dead code (i.e. successor of a `Return`
     or `Trap`).

Running this against the existing example set is the cheapest end-to-end
check you'll get before writing a backend.
