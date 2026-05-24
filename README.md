
## Build stdlib

As a static archive (recommended — bundles the Rust std bits rt depends on, so the final link only needs `cc`):

```bash
cargo build -p otter_rt --release
# target/release/libotter_rt.a
```

As a single object (no std bundled; only useful if you handle std linkage yourself):

```bash
cargo rustc -p otter_rt --release --lib --crate-type=staticlib \
    -- --emit=obj -C codegen-units=1
# → target/release/deps/otter_rt-<hash>.o
```

## Compile and link a program

```bash
# 1. Emit a program object
cargo run --release -- compile examples/hello.of -o hello.o

# 2. Link with cc (clang on macOS, gcc on Linux)
cc hello.o target/release/libotter_rt.a -o hello

# 3. Run
./hello
```

On Linux, add the system libs Rust std pulls in:

```bash
cc hello.o target/release/libotter_rt.a -o hello -lpthread -ldl -lm
```

If the linker rejects relocations, AOT codegen runs with PIC off — pass `-Wl,-no_pie` (macOS) or `-no-pie` (Linux).

Re-run `cargo build -p otter_rt --release` after editing rt: the workspace's default `cargo build` only builds rt as an rlib for the compiler crate, not as a staticlib.

## External libraries (`of_io` example)

`of_io` is an external library that exposes POSIX file, networking and
multiplexing syscalls. It is **not** part of `of:core` — programs must
import its symbols explicitly and link against `libotter_io.a`.

### Build the library

```bash
cargo build -p otter_io --release
# → target/release/libotter_io.a
```

### Use it from an `of` program

Because the compiler resolves `import { ... } from "of_io"` by looking
for `of_io.of` next to the importing program, copy the bindings file
into your project directory:

```bash
cp io/of_io.of my_project/
```

Then `import` the symbols you need:

```typescript
import { __of_open, __of_read, __of_close, O_RDONLY } from "of_io";
```

### Link

Pass both archives to `cc`. Order matters — program object first,
then archives:

```bash
# macOS
cc my_project/main.o \
   target/release/libotter_rt.a \
   target/release/libotter_io.a \
   -o myprog

# Linux: add the usual system libs Rust std needs.
cc my_project/main.o \
   target/release/libotter_rt.a \
   target/release/libotter_io.a \
   -lpthread -ldl -lm \
   -o myprog
```

### Authoring your own external library

1. Add a new workspace member crate (`Cargo.toml`: `crate-type =
   ["staticlib", "cdylib", "rlib"]`, depend on `libc` if you call
   POSIX).
2. Export each symbol with `#[unsafe(no_mangle)] pub extern "C" fn ...`,
   using the same C-ABI calling convention `of_io` uses (see
   `io/src/lib.rs`).
3. Write a sibling `.of` file with matching `extern function` declarations
   and `const` definitions for any portable constants. Pass `Buffer` for
   raw byte regions.
4. Ship the `.of` file alongside the static archive. Consumers drop the
   `.of` next to their program and link the archive at `cc` time.
