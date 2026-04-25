# MIR Recommendation for Otter Fusion

This document proposes a Mid-level Intermediate Representation (MIR) layer for Otter Fusion, sitting between the existing HIR (`src/hir.rs`) and a future code generator. It is grounded in the current state of the project: HIR is fully resolved and type-safe, generics remain abstract, no backend exists yet, closure captures are collected but unvalidated, and union memory layout is unspecified.

## What the MIR needs to make explicit

Working from `src/hir.rs` and the 21 example programs, these are the things HIR leaves implicit that codegen will need spelled out:

1. **Monomorphization** — generic structs/functions still carry `TypeParamId` (`hir.rs:92`, `hir.rs:132`).
2. **Method dispatch** — `specialised_methods: Vec<(Vec<ResolvedType>, FnId)>` (`hir.rs:101`) needs to resolve to a single `FnId` per call site.
3. **Interface dispatch** — `implements` (`hir.rs:103`) needs vtables for indirect calls.
4. **Union layout** — `ResolvedType::Union` (`hir.rs:63`) is logical only; needs discriminants and payload layout.
5. **`is` / `as` lowering** — tag test, payload reinterpret, trap on mismatch.
6. **Block / if-expression flattening** — implicit-return blocks (`hir.rs:148`) become sequences plus result temporaries.
7. **`for-in` over `Iterator<T>`** — desugar to a `next()` loop with a null check (example 14).
8. **Closure captures** — `HirCapture` (`hir.rs:169`) is collected but unvalidated; needs a synthesized environment struct.
9. **Allocations** — distinguish GC-managed (`AllocStruct`, `AllocList`, `AllocMap`, `AllocClosure`) from extern/foreign.
10. **FFI boundary** — pointer-vs-value rules, ABI, nullability traps.

## Recommended shape: CFG of basic blocks, mutable locals, no SSA yet

```
MirProgram {
  types:     Map<MirTypeId, MirTypeDef>          // monomorphized, concrete
  functions: Map<MirFnId, MirFunction>           // monomorphized
  vtables:   Map<(MirTypeId, MirTypeId), VTable> // (struct, interface) -> slots
  entry:     MirFnId
}

MirTypeDef =
  | Struct  { fields: [(name, MirType)], layout, kind: Managed | Foreign }
  | Union   { variants: [(tag: u32, MirType)], discriminant: PrimitiveType }
  | Closure { env_fields: [(name, MirType)], fn: MirFnId }

MirType =
  | Primitive(...)
  | ManagedRef(MirTypeId)              // GC ref: structs, lists, maps, strings, closures
  | Pointer(MirType)                   // *T — extern only
  | Union(MirTypeId)
  | FnPtr(args, ret)                   // bare extern function pointer
  | Closure(MirTypeId)                 // managed closure with environment

MirFunction { id, abi: Otter | Extern, params, locals, blocks, entry, return_type }

MirBlock { stmts: [Stmt], terminator: Terminator }

Stmt = Assign(local, Rvalue)

Rvalue =
  | Use(Operand)
  | Bin(op, Operand, Operand)
  | Un(op, Operand)
  | Call(Callee, [Operand])
  | AllocStruct(MirTypeId, [Operand])
  | AllocList(MirType, [Operand])
  | AllocMap(K, V, [(Operand, Operand)])
  | AllocClosure(MirFnId, [Operand])   // env captures
  | Field(Operand, idx)
  | UnionConstruct(MirTypeId, tag, Operand)
  | UnionTag(Operand)
  | UnionPayload(Operand, MirTypeId)   // unchecked — guard with UnionTag first

Operand = Copy(local) | Move(local) | Const(MirConst)

Callee =
  | Static(MirFnId)                    // monomorphized direct call
  | Indirect(Operand)                  // closure / fn ptr
  | Virtual(Operand, MirTypeId, slot)  // interface dispatch

Terminator =
  | Goto(BlockId)
  | CondBr(Operand, BlockId, BlockId)
  | Switch(Operand, [(value, BlockId)], default)   // union / `is` chains
  | Return(Option<Operand>)
  | Trap(reason)                                   // `as` mismatch, null deref
  | Unreachable
```

Why CFG with basic blocks but **not** SSA: you already need a CFG the moment you flatten `if`/`for`/`while`. Mutable locals are quick to emit and trivial to read. SSA is the right shape if/when you add an optimizer; LLVM (if it becomes the backend) will do its own SSA construction anyway, so paying that cost now is premature.

## Lowering pipeline (HIR → MIR)

Structure as ordered passes; each one can land independently.

### 1. Monomorphization

Fixed-point worklist. Starting from `extern` exports plus `main`, walk `Call(callee, type_args, args)` and `StructInit(_, type_args, _)` (`hir.rs:179`, `hir.rs:182`), instantiate `(HirFnId, [ResolvedType])` and `(TypeId, [ResolvedType])` into fresh `MirFnId` / `MirTypeId`. Substitute `TypeParam(_)` everywhere. This pass also resolves the `specialised_methods` puzzle as a side effect — at a concrete call site you pick the entry whose target args unify.

Prerequisite: make `ResolvedType` `Hash + Eq` (currently only `PartialEq`, `hir.rs:58`) so it can key the monomorphization cache.

### 2. Interface vtable construction

For each `(struct, interface)` pair in `HirStruct.implements`, emit a vtable mapping method-slot to `MirFnId` (post-monomorphization). Indirect calls through an interface value become `Callee::Virtual`.

### 3. Control-flow flattening

- `If` / `Block` expressions → basic blocks plus a result temporary.
- `While` → head / body / exit blocks.
- `for x in iter` → `let it = iter; loop { let n = it.next(); if n is null { break } else { let x = n as T; … } }`.
- `break` / `continue` → `Goto`.

### 4. Union lowering

Assign tags per variant (stable order based on a canonical sort of the union members). Lower:

- `is T` → `UnionTag(_) == tag(T)`.
- `as T` → `if tag != tag(T) { Trap } else { UnionPayload }`.
- Union construction → `UnionConstruct`.

Special-case `T | null` over `ManagedRef` to a nullable pointer (skip discriminant). This is the highest-payoff layout optimization given how heavily the examples use `| null` for errors and iterators.

### 5. Closure capture analysis

For each `FunctionLiteral`, walk free variables, materialize a closure environment struct, emit `AllocClosure` at the literal site, and rewrite captured reads to environment loads. This closes the one HIR-validation gap (`hir.rs:191`) cleanly inside MIR.

### 6. FFI lowering

Extern calls become `Call(Static, …)` with `abi: Extern`. Verify `*T` only crosses the boundary. If a non-nullable pointer is declared without `| null`, insert a trap-on-null guard at the call site (per the README rule).

### 7. Allocation explicitization

Every `StructInit`, list literal, map literal, and function literal becomes an explicit `Alloc*` rvalue. `pin` / `unpin` stay as ordinary calls — the user-stated rule is that pinning is always manual.

## Phasing suggestion

- **Phase 1 — smallest viable MIR.** Monomorphization, control-flow flattening, explicit allocations. CFG with mutable locals. Skip unions, closures, vtables. Covers examples 01–05 and 07. Gets you to a real backend the fastest.
- **Phase 2.** Vtables, union lowering, `is`/`as`. Unlocks 06, 08, 09, 12, 13, 16–20.
- **Phase 3.** Closure environments and iterator desugar. Unlocks 04 and 14.
- **Phase 4.** FFI lowering hardening and the null-pointer optimization for `T | null`. Unlocks 15.

## Main tradeoffs

- **Monomorphize vs erase.** Monomorphization is the right call for a GC + value-semantics + bounded-generics model. Cost: binary size. Benefit: direct calls and no runtime type info for generics. Erasure would force boxing on managed values you currently treat as flat.
- **Tree IR vs CFG.** A tree IR is faster to write but painful as soon as you implement DCE or inlining. CFG with mutable locals is the cheapest form that scales.
- **Union layout policy.** Uniform tagged unions are simple but waste space; specialized layouts (null-pointer optimization for `T | null`, pointer-tagging for `interface | struct`) are higher-leverage but add cases. Recommend uniform tagged as the default plus the `T | null` specialization up front, since every example with errors or iterators leans on it.
- **Monomorphization cache keys.** `(FnId, [ResolvedType])` requires structural equality and hashing on `ResolvedType` before starting.

## Suggested files

- `src/mir.rs` — type definitions (mirrors the shape above).
- `src/lower/mod.rs` — orchestrator; public entry `lower(hir: &Hir) -> MirProgram`.
- `src/lower/mono.rs` — monomorphization worklist.
- `src/lower/cfg.rs` — control-flow flattening.
- `src/lower/unions.rs` — union, `is`, `as`.
- `src/lower/closures.rs` — capture analysis and environment synthesis.
- `src/lower/ffi.rs` — extern boundary.

This keeps `validator.rs` untouched: HIR stays the canonical typed surface; MIR is the codegen-ready form.
