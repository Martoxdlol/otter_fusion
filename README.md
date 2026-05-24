
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
