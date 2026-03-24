# Otter Fusion Language Specification

This document defines the core syntax and behavior of the language, including types, variables, functions, structs, interfaces, extensions, and more.
## Type System

### Primitive Types
- Signed Integers: `i8`, `i16`, `i32`, `i64`
- Unsigned Integers: `u8`, `u16`, `u32`, `u64`
- Floating Point: `f32`, `f64`
- Boolean: `bool`
- String: `str`
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

## 4. Structs and Interfaces

Interfaces define requirements, and structs provide data and implementation.

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

## Extensions

The `extend` keyword adds methods or interface implementations to existing structs.

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

## For loops iterate over lists or maps.

```typescript
for (num in numbers) {
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

## Conclusion

This document outlines the core features of the FIU-LYC-LANG, including its type system, variable declarations, functions, structs, interfaces, extensions, and error management. The language is designed to be flexible and powerful while maintaining a clear and concise syntax.