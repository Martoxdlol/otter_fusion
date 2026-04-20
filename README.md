# Otter Fusion Language Specification

This document defines the core syntax and behavior of the language, including types, variables, functions, structs, interfaces, extensions, and more.
## Type System

### Primitive Types
- Signed Integers: `i8`, `i16`, `i32`, `i64`
- Unsigned Integers: `u8`, `u16`, `u32`, `u64`
- Floating Point: `f32`, `f64`
- Boolean: `bool`
- String: `str`
- Character: `char`
- Empty: `null`

Slightly inspired in rust and typescript.

### Discriminated Unions

Very core to the language. It uses pipes (`|`) to indicate that a value can be one or several types. The way to discriminate between them is using the `is` operator, and using `as` to force the type to be a specific one (will fail if it isn't the correct one).

```
type Result = i64 | str | null;
```

Error management example:

```typescript
struct Error {
  message: str,
  code: i32,
}

function divide(a: f64, b: f64): f64 | Error {
  if (b == 0.0) {
    Error {
      message: "Division by zero",
      code: 1,
    }
  } else {
    a / b
  }
}
```

## Variables and Expressions

Variables are declared using the `var` keyword.

```typescript
var name: str = "John Doe";
var age = 30; // Inferred as i64
var price: f32 = 19.99;
```

The `if` block evaluates to the value of its last expression.

```typescript
var status = if (age >= 18) {
  "Adult"
} else {
  "Minor"
};
```

### Block Expressions

A block `{ ... }` evaluates to its last expression. This works anywhere an expression is expected.

```typescript
var value = {
  var x = 10;
  var y = 20;
  x + y
};
```

## Functions

Functions use the `function` keyword. The last expression in a function body is automatically returned. Use the `return` keyword for early exits.

The implicit return allows the use of `if ... else` as an expression. Also it is inspired in rust (AND JUST MAKES SENSE!).

```typescript
function add(a: i64, b: i64): i64 {
  a + b
}

function get_ratio(current: f64, total: f64): f64 {
  if (total == 0.0) {
    return 0.0; // Early return
  }
  current / total
}
```

### Lambdas

Anonymous functions can be used as values. They follow the same implicit return rules.

```typescript
var double = function(x: i32): i32 { x * 2 };
var apply = function<T>(value: T, f: (T) -> T): T { f(value) };
```

## 4. Structs and Interfaces

Interfaces define requirements, and structs provide data and implementation. Methods receive `self` as the first parameter to access the struct's fields.

```typescript
interface Named {
  name: str,
}

struct Person: Named {
  name: str,
  age: i32,

  function greet(self): str {
    "Hello, " + self.name
  }
}
```

Interfaces can extend other interfaces using the same colon syntax:

```typescript
interface Printable: Named {
  function to_string(self): str
}
```

### Struct Instantiation

Structs are instantiated by name with field values.

```typescript
var person = Person {
  name: "Alice",
  age: 30,
};
```

## Extensions

The `extend` keyword adds methods or interface implementations to existing structs. It works like Rust's `impl`.

```typescript
struct Vehicle {
  speed: i32,
}

extend Vehicle {
  function is_fast(self): bool {
    self.speed > 100
  }
}

interface Movable {
  move(self): str,
}

extend Vehicle: Movable {
  function move(self): str {
    "The vehicle moves"
  }
}
```

### Generic Extensions

Extensions can introduce or specialize generic parameters, similar to Rust's `impl<T>`.

```typescript
struct Wrapper<T> {
  value: T,
}

// Generic extension — applies to all Wrapper<T>
extend<T> Wrapper<T> {
  function get(self): T {
    self.value
  }
}

// Specialized extension — only applies to Wrapper<i32>
extend Wrapper<i32> {
  function double(self): i32 {
    self.value * 2
  }
}

// Generic extension with interface implementation
extend<T> Wrapper<T>: Named {
  function name(self): str {
    "Wrapper"
  }
}
```

## Type Logic: `is` and `as`

- `is`: Evaluates to `bool`. Checks if a variable is of a specific type.
- `as`: Performs an explicit cast or type narrowing.

```typescript
function process(input: i64 | str) {
  if (input is i64) {
    var value = input as i64;
    print("Number: " + (value as str));
  } else {
    print("String: " + (input as str));
  }
}
```

## Error Management

Error handling is achieved by returning unions of values and error structs. There is no global exception handling.

```typescript
struct Error {
  message: str,
  code: i32,
}

function divide(a: f64, b: f64): f64 | Error {
  if (b == 0.0) {
    Error {
      message: "Division by zero",
      code: 1,
    }
  } else {
    a / b
  }
}

var result = divide(10.0, 0.0);

var output = if (result is Error) {
  "Failed: " + (result as Error).message
} else {
  "Success: " + (result as f64 as str)
};
```

## Numeric Conversions

Conversions between different numeric sizes or types must be explicit.

If types are not compatible, the `as` operator will crash the program (like rust's panic).

```typescript
var high_precision: f64 = 123.456;
var low_precision: f32 = high_precision as f32;

var large_int: i64 = 1000;
var small_int: i8 = large_int as i8;
```

## Null Handling

The `null` can be used to represent the absence of a value. Use `is null` to check for null values.

```typescript
function find_user(id: i64): str | null {
  if (id == 1) {
    "Admin"
  } else {
    null
  }
}

var user = find_user(5);

if (user is null) {
  print("No user found");
} else {
  print("User: " + (user as str));
}
```

## Generics

Generics allow for defining functions and structs that can operate on any type.

```typescript
function identity<T>(value: T): T {
  value
}

struct Box<T> {
  content: T,

  function get(self): T {
    self.content
  }
}
```

## Lists

Lists are defined using square brackets and can hold any type of value.

```typescript
var numbers: List<i64> = [1, 2, 3, 4, 5];
var mixed: List<i64 | str> = [1, "two", 3, "four"];
```

## Maps
Maps are defined using curly braces and hold key-value pairs.

```typescript
var user_ages: Map<str, i32> = {
  "Alice": 30,
  "Bob": 25,
};
```

Differentiate from block:

- Key value always with quotes: `"key": value,`
- Try to parse as map, if it fails, parse as block.

## Control Flow

### For Loops

For loops iterate over lists or maps.

```typescript
for (num in numbers) {
  print(num);
}
```

### While Loops

```typescript
var i = 0;
while (i < 10) {
  print(i);
  i = i + 1;
}
```

### Break and Continue

Use `break` to exit a loop early and `continue` to skip to the next iteration.

```typescript
for (num in numbers) {
  if (num == 0) {
    continue;
  }
  if (num > 100) {
    break;
  }
  print(num);
}
```

## Iterator

The `Iterator<T>` type provides a way to iterate over collections.

```typescript
// Internal type
interface Iterator<T> {
  next(self): T | null,
}

function print_all<T>(iter: Iterator<T>) {
  for (item in iter) {
    print(item);
  }
}
```

## Foreign Function Interface (`extern`)

The `extern` keyword declares items that follow the C ABI, so they can interact
with foreign code. It can be applied to functions, structs, and function types.

### Extern Functions

The `extern` keyword on a function is the boundary with the C ABI. It has two
forms:

**Import** — no body. The implementation is provided by foreign code.

```typescript
extern function print(value: str);
extern function malloc(size: u64): *Buffer;
```

**Export** — has a body. The compiler emits the function with the C calling
convention so that foreign code can call it. Typically used for callbacks
registered with C libraries.

```typescript
extern function on_tick(d: *MyStruct) {
  d.counter = d.counter + 1
}
```

Exported extern functions are always top-level items. They **cannot capture
variables** from any enclosing scope (there is no enclosing scope), because a
C function pointer is a single machine pointer with nowhere to hold a closure
environment. C callback APIs conventionally pass user state through a
`void*` parameter — in this language that role is filled by a `*T` argument
on the callback itself.

### Extern Structs

Structs marked as `extern` are laid out in memory as C structs, so they can
be passed to or returned from extern functions.

```typescript
extern struct MyCFunctionArgs {
  flags: i32,
  name: *str,
}
```

### Extern Function Types

Function types that follow the C ABI use the `extern (...)  => T` syntax.
Parameters have names (like a declaration) and can be pointers.

```typescript
type Fun = extern (x: MyStruct) => bool;
type Callback = extern (data: *MyStruct, size: u64) => *Buffer | null;
```

### Pointer Syntax

Inside extern functions and extern structs, parameters, fields and return
types can be prefixed with `*` to mark them as pointers.

```typescript
extern function hashmap_set(key: *str, value: *MyStruct);
extern function hashmap_set<T>(key: *str, value: *T);
extern function malloc(size: u64): *Buffer;
extern function malloc(size: u64): *Buffer | null;
```

### Rules

- `*` as a type prefix is only allowed inside extern functions, extern structs
  and extern function types.
- In extern functions and extern structs, a parameter or field marked with `*`
  behaves as a pointer; otherwise, values are passed by value (not by pointer).
- Extern structs are laid out as C structs. Non-extern structs have no
  guaranteed layout.
- Generic type parameters (`T`, `U`, ...) can only be passed as pointers
  (`*T`), since their size is not known at the call site.
- Union types are only supported as pointers over the entire union
  (`*(A | B)`), or as the special case of a nullable pointer (`*T | null`).
- If a return type is declared without the `| null` option, calling the
  function will fail (panic) when the underlying pointer is actually null.

### Memory Model

The runtime has two disjoint memory regions:

- **Managed heap** — allocated, traced and reclaimed by the garbage collector.
  Holds regular (non-extern) structs, lists, maps, strings, closures.
- **Foreign heap** — anything allocated by extern code (`malloc`, `mmap`,
  library allocators, arenas). Opaque to the GC. Ownership is manual.

`extern struct` values live in the foreign heap. Non-extern struct values live
in the managed heap. The `*T` syntax expresses a raw pointer into either
region — the GC decides how to treat it at runtime, based on whether the
pointer refers to memory allocated by the managed allocator.

- Pointer refers to managed memory → the GC knows about the object and applies
  its normal rules (tracing, moving, collection).
- Pointer refers to foreign memory → the GC ignores it entirely. The
  programmer is responsible for its lifetime.

### Pinning

The GC is free to move or reclaim managed objects at any time. Whenever a
`*T` pointing into the managed heap is handed to foreign code, the object
must be **pinned** so the GC neither moves nor collects it. Pinning is
**always manual** — the compiler does not insert `pin` / `unpin` anywhere.

```typescript
function pin<T>(value: T): *T;
function unpin<T>(ptr: *T);
```

- `pin(value)` registers `value` as a pinned GC root and returns its raw
  address as `*T`. While pinned, the object is guaranteed not to move and not
  to be collected.
- `unpin(ptr)` releases the pin. After this call the pointer is no longer
  valid for foreign use — the GC may move or collect the underlying object on
  its next cycle.

Pinning is refcounted per object: nested `pin` / `unpin` pairs on the same
value compose correctly. `pin` on a value that already lives in the foreign
heap returns its address unchanged (no-op).

Passing a managed value to an extern function without first pinning it is a
programmer error. Nothing happens implicitly.

### Example

```typescript
extern struct Buffer {
  data: *u8,
  size: u64,
}

extern function malloc(size: u64): *Buffer | null;
extern function free(buf: *Buffer);

function allocate(size: u64): *Buffer {
  var maybe = malloc(size);
  if (maybe is null) {
    return null; // will fail if caller requires non-null
  }
  maybe as Buffer
}
```

Pinning a managed value across a foreign callback:

```typescript
extern function register_callback(data: *MyStruct, cb: extern (d: *MyStruct) => void);
extern function unregister_callback(data: *MyStruct);

struct MyStruct {
  counter: i64,
}

function install(): *MyStruct {
  var state = MyStruct { counter: 0 };
  var ptr = pin(state);
  register_callback(ptr, on_tick);
  ptr
}

function remove(ptr: *MyStruct) {
  unregister_callback(ptr);
  unpin(ptr);
}
```

## Conclusion

This document outlines the core features of the Otter Fusion, including its type system, variable declarations, functions, structs, interfaces, extensions, and error management. The language is designed to be flexible and powerful while maintaining a clear and concise syntax.