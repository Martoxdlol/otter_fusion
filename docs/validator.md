# Otter Fusion Validator - Technical Documentation

The validator transforms an AST (`ast::Module`) into a type-checked HIR (`hir::Hir`). It runs in 6 sequential passes, each building on the results of the previous one. Validation stops at the first pass that produces errors.

## Architecture Overview

```
Source Code
    |
    v
  Lexer  -->  Tokens
    |
    v
  Parser -->  AST (ast::Module)
    |
    v
  Validator (6 passes)  -->  HIR (hir::Hir)
```

### Key Data Structures

**Validator state** (`mod.rs`):
- `hir: Hir` - the output being built incrementally
- `module_scopes` / `visible_names` - name resolution tables
- `type_kinds` - maps each TypeId to Struct, Interface, or Alias
- `type_aliases` - stores alias definitions (name, generics, body)
- `alias_expanding` - cycle detection set for recursive alias expansion
- `resolved_alias_bodies` - cached pre-substitution bodies for recursive aliases
- `type_param_scope` - contextual map of generic param names to TypeParamIds

**ResolvedType** (`hir.rs`) - the core type representation after resolution:
- `Primitive(PrimitiveType)` - i8, i32, f64, str, bool, char
- `Struct(TypeId, Vec<ResolvedType>)` - e.g., `List<i64>`
- `Interface(TypeId, Vec<ResolvedType>)` - e.g., `Drawable<T>`
- `Union(Vec<ResolvedType>)` - e.g., `i64 | str | null`
- `Function(Vec<ResolvedType>, Box<ResolvedType>)` - e.g., `(i64, str) -> bool`
- `TypeParam(TypeParamId)` - unresolved generic, e.g., `T`
- `Alias(TypeId, Vec<ResolvedType>)` - recursive type alias reference
- `Null` - the null type

---

## Pass 0: Module & Import Registration (`pass_register.rs`)

**Purpose**: Allocate IDs for all modules, types, and functions. Build name resolution tables.

**What happens**:
1. Each source file becomes a `ModuleId`
2. For each item in a module:
   - `struct Foo` -> allocate `TypeId`, register as `TypeKind::Struct`
   - `interface Bar` -> allocate `TypeId`, register as `TypeKind::Interface`
   - `type Alias = ...` -> allocate `TypeId`, register as `TypeKind::Alias`
   - `function baz()` -> allocate `FnId`
3. Resolve imports: `import { Foo, bar } from "utils"` copies the referenced IDs into the importing module's `visible_names`
4. Glob imports (`import "utils"`) copy everything

**Output**: `module_scopes`, `visible_names`, `module_name_to_id`, `type_kinds` are populated.

---

## Pass 1: Type Shapes (`pass_register.rs`)

**Purpose**: Register struct/interface members (fields, methods) and `implements` clauses. No type resolution yet - just recording names and shapes.

**What happens**:
1. For each struct: register method names, record `implements` interface names
2. For each interface: register method names, record `extends` parent names
3. For extend blocks: defer to `pending_extends` (processed in Pass 3)

**Why separate from Pass 0**: Methods reference types that may not be registered yet (forward references). Pass 0 registers names; Pass 1 registers shapes.

---

## Pass 2: Type Parameters & Signatures (`pass_register.rs`)

**Purpose**: Allocate TypeParamIds for all generics, resolve field types, parameter types, and return types.

**What happens**:
1. For each generic parameter `<T: SomeInterface>`:
   - Allocate a unique `TypeParamId`
   - Resolve bounds (must be interfaces)
2. Resolve all field types: `name: TypeExpr` -> `name: ResolvedType`
3. Resolve all function signatures: parameter types and return types

**Type resolution** is handled by `resolve_type()` (see "Type Resolution" section below).

---

## Pass 3: Merge Extend Blocks (`pass_extend.rs`)

**Purpose**: Merge methods from `extend` blocks into their target structs.

### Generic vs Specialized Extends

The validator classifies each extend block:

**Generic extend** - methods available for all instantiations:
```
extend<T> Box<T> {
    function get(self): T { ... }
}
```

Classification rule: the target type args are exactly the extend's own generic params in 1:1 order. These methods are merged into `HirStruct.methods`.

**Specialized extend** - methods only available for specific type arguments:
```
extend List<i64> {
    function sum_all(self): i64 { ... }
}
```

Classification rule: any target type arg is a concrete type (not a generic param). These methods are stored in `HirStruct.specialized_methods` as a `SpecializedExtend { type_args, methods }`.

### Processing Flow

For specialized extends:
1. Allocate the extend block's own type params (for partial specialization)
2. Resolve the target's type args to concrete `ResolvedType` values
3. Bring struct type params AND extend params into scope
4. Resolve method signatures
5. Substitute struct type params with the concrete specialization args in method signatures
6. Store in `specialized_methods`
7. If the extend has `implements` clauses, store in `specialized_implements`

### Implements Clauses

```
extend List<i64>: Summable { ... }
```

The `Summable` interface obligation applies only to `List<i64>`, not all `List<T>`. Stored as `specialized_implements: Vec<(Vec<ResolvedType>, TypeId)>`.

---

## Pass 4: Interface Validation (`pass_interface.rs`)

**Purpose**: Verify that structs properly implement their declared interfaces.

For each struct:
1. **Generic implements**: check `HirStruct.methods` satisfies each interface in `HirStruct.implements`
2. **Specialized implements**: for each `(type_args, iface_id)` in `specialized_implements`, collect methods from the matching `SpecializedExtend` plus all generic methods, then check they satisfy the interface

Checks performed:
- All required fields exist with compatible types
- All required methods exist (unless the interface provides a default body)
- Method signatures match: param count, param types, return type

---

## Pass 5: Type-Check Function Bodies (`pass_typecheck.rs`)

**Purpose**: Type-check all expressions and statements in function bodies.

### Method Dispatch (Member Access)

When type-checking `obj.method()` where `obj: Struct(id, type_args)`:

1. **Fields**: check struct fields, substitute type params with concrete args
2. **Specialized methods** (checked first - higher priority): iterate `specialized_methods`, find entries where `type_args` match the instance's type args. If a matching method is found, return it (types already concrete, no substitution needed)
3. **Generic methods**: check `HirStruct.methods`, substitute type params with instance's type args

This priority ordering means a specialized method shadows a generic one with the same name.

### Expression Type-Checking

Each expression is type-checked bottom-up, producing a `TypedExpr { kind, ty }`:
- Literals: infer type from value (`42` -> `i64`, `"hello"` -> `str`)
- Variables: look up in local scope, then module-level functions
- Binary ops: check operand types, determine result type
- Function calls: check arg count, check arg types vs param types
- If/else: type-check branches, compute result type (union if branches differ)
- List literals: infer element type union, wrap in `List<T>`
- Map literals: infer key/value type unions, wrap in `Map<K, V>`

### Alias Unfolding in Type-Check

Several expression forms need the structural type, not an alias wrapper:
- **For loops**: the iterable must be `List<T>` - if it's an `Alias`, unfold first
- **Member access**: needs to see `Struct` or `Interface` - unfold aliases
- **Function calls**: callee must be `Function(...)` - unfold aliases

`shallow_unfold_alias()` handles this: if the type is `Alias(id, args)`, expand one level; otherwise return as-is.

---

## Type Resolution (`resolve.rs`)

### `resolve_type(module_id, type_expr) -> ResolvedType`

Converts an AST `TypeExpr` to a resolved `ResolvedType`:

1. **Primitive**: direct mapping (`i64` -> `Primitive(Int64)`)
2. **Named type**: look up in type param scope first (returns `TypeParam`), then in `visible_names`:
   - Alias -> call `expand_type_alias()`
   - Struct -> `Struct(id, resolved_args)`
   - Interface -> `Interface(id, resolved_args)`
3. **Union**: resolve each variant, flatten nested unions, deduplicate
4. **Function type**: resolve param types and return type

### Type Alias Expansion

`expand_type_alias(module_id, alias_id, args)`:

1. **Cycle check**: if this alias is already being expanded (`alias_expanding` set), return `Alias(alias_id, args)` as a recursive fixpoint marker
2. **Arity check**: verify arg count matches generic param count
3. **Scope setup**: map generic param names to their TypeParamIds
4. **Resolve body**: call `resolve_type` on the alias body AST
5. **Cache**: if the body contains recursive references, cache the pre-substitution body in `resolved_alias_bodies`
6. **Substitute**: replace TypeParams with concrete args
7. **Strip bare self-refs**: remove redundant `| Json |` from `type Json = i64 | Json | null`
8. **Validate base case**: if recursive, ensure at least one non-recursive variant exists

### Recursive Type Aliases

```
type Json = i64 | str | Map<str, Json> | List<Json> | null;
```

**How it works**:

When `expand_type_alias` resolves the body of `Json`, it encounters `Json` again inside `Map<str, Json>` and `List<Json>`. At that point, `alias_expanding` contains `json_id`, so the recursive reference returns `Alias(json_id, [])` instead of erroring.

The resolved body becomes:
```
Union([
    Primitive(Int64),
    Primitive(String),
    Struct(map_id, [Primitive(String), Alias(json_id, [])]),
    Struct(list_id, [Alias(json_id, [])]),
    Null
])
```

**Bare self-reference stripping**: `type Json = i64 | Json | null` - the bare `| Json |` variant is redundant (it means "or any Json value", which is what the type already is). The `strip_bare_self_refs` function removes `Alias(id, _)` variants at the top level of a union when they reference the alias being defined.

**Base case validation**: After resolution, `has_non_recursive_base` checks that at least one union variant doesn't reference the alias. This rejects degenerate types like `type X = X` or `type X = List<X>`.

### Unfolding for Type Compatibility

When `types_compatible` encounters an `Alias`, it needs to see inside. Two mechanisms:

1. **`unfold_alias(module_id, alias_id, args)`** - uses `&mut self` (can call `resolve_type`). Used during type-checking.
2. **`unfold_alias_readonly(alias_id, args)`** - uses `&self` (works from cached `resolved_alias_bodies`). Used inside `types_compatible` which takes `&self`.

---

## Type Compatibility (`resolve.rs`)

`types_compatible(expected, actual) -> bool`

Checks if a value of type `actual` can be used where `expected` is required.

### Match Rules (in priority order)

1. **Structural equality**: `expected == actual` -> true
2. **Alias vs Alias (same ID)**: compare args pairwise
3. **Alias on either side**: unfold one level, recurse
4. **Union vs Null**: check if union contains a Null variant
5. **Union vs Union**: every actual variant must match some expected variant
6. **Union vs concrete**: check if actual matches any expected variant
7. **Concrete vs Union**: check if all actual variants match expected
8. **Struct vs Struct**: same TypeId + compatible args
9. **Interface vs Interface**: same TypeId + compatible args
10. **Function vs Function**: compatible params + compatible return type

### Depth Guard

A `depth` counter (max 64) prevents infinite recursion in pathological cases like mutual recursive aliases. The `PartialEq` check at the top catches same-alias-same-args before unfolding, which is the key termination guarantee for well-formed recursive types.

---

## Type Parameter Substitution (`resolve.rs`)

`substitute_type_params(ty, param_ids, args)`:

Recursively replaces `TypeParam(id)` with the corresponding concrete type from `args`. Handles all `ResolvedType` variants including `Alias` (substitutes within the alias's type args while keeping the alias reference intact).

Used in:
- Alias expansion (replace generic params with concrete args)
- Method access (replace struct type params with instance's type args)
- Specialized extend processing (bake concrete types into method signatures)

---

## Standard Library (`mod.rs`)

The validator registers a virtual "std" module containing:
- `List<T>` - built-in list type (TypeId tracked in `list_type_id`)
- `Map<K, V>` - built-in map type (TypeId tracked in `map_type_id`)
- `print(value: str): null` - built-in print function

Users must explicitly import from "std": `import { List, Map, print } from "std"`.

List and Map TypeIds are used internally for literal type inference:
- `[1, 2, 3]` infers to `List<i64>`
- `{"a": 1}` infers to `Map<str, i64>`
- For-loop iterable check: must be `List<T>`

---

## Error Handling

All validation errors are `ValidationError { kind, module, context }`. The `kind` enum covers:
- Name resolution: undefined types/functions/variables, duplicates
- Type checking: mismatches, wrong arg counts, non-callable types
- Type aliases: cyclic aliases, recursive aliases without base cases
- Interfaces: missing fields/methods, signature mismatches
- Extends: target not a struct, specialization arity mismatch, duplicate specialized methods
- Literals: invalid int/float parsing
- Control flow: break/continue outside loop

Errors are accumulated per-pass. If any pass produces errors, validation stops and returns all errors from that pass.
