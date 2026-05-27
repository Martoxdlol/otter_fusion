# Benchmarks

Otter Fusion vs Rust, Go, Node, and Python on seven workloads. The headline
chart and discussion live in the [top-level README](../README.md#benchmarks);
this file is the operator's manual.

## Layout

```
bench/
  bench.py              # driver: build, verify, time, report
  plot.py               # results.json -> benchmarks.svg / .png
  programs/<name>/      # <name>.{rs,go,js,py,of} — one impl per language
  results/
    results.json        # raw samples (wall + in-process compute)
    RESULTS.md          # generated markdown tables
    benchmarks.svg/.png # generated chart
```

## Workloads

Tier A — common programs:

- **fib** — recursive `fib(35)`; function-call overhead.
- **sieve** — count primes below 2,000,000; array + tight loops.
- **mandelbrot** — 800×800 escape-time, checksum of iteration counts; pure f64.

Tier B — language features:

- **interfaces** — 10M virtual `area()` calls over a list of shapes.
- **iterators** — filter/map/sum over 0..10,000,000.
- **generics** — construct + read a generic `Wrapper<T>` 10M times.
- **unions** — build + evaluate a small AST 2M times (discriminated union).

Each program prints two lines: the **answer** (an integer checksum, identical
across all five languages — the harness refuses to time a benchmark whose
outputs disagree) and the **in-process compute time in nanoseconds**.

## Running

```bash
# from the repo root, build the toolchain first:
cargo build --release -p otter_fusion -p otter_rt

python3 bench/bench.py                      # all benchmarks, all languages
python3 bench/bench.py fib sieve            # a subset
python3 bench/bench.py --runs 10            # more samples (default 5)
python3 bench/bench.py --langs rust,otter   # a subset of languages
python3 bench/bench.py --skip-verify        # time without the agreement check
python3 bench/plot.py                       # regenerate the chart from results.json
```

Requires `rustc`, `go`, `node`, `python3` on `PATH`; missing toolchains are
skipped with a warning. `plot.py` uses `cairosvg` for the PNG (optional; the
SVG is always written).

## Two metrics, and why

- **compute** — the program times only its hot region with a monotonic clock
  after a warm-up call, and prints the elapsed nanoseconds. Startup is excluded
  *by construction*. This is the fair language-vs-language compute number.
- **end-to-end** — wall time of the whole process (what `bench.py` measures
  around `subprocess`), startup included.

`startup ≈ end-to-end − compute`. We deliberately do **not** measure a
do-nothing program and subtract it: an empty program's startup is not the same
fixed cost embedded in a real run (Node JIT-compiles the actual functions,
imports differ), so subtraction would invent a number that corresponds to no
real measurement.

## Fairness controls

- Release/optimized builds: `rustc -O`, `go build`, otter **AOT** (`compile` +
  link `libotter_rt.a`) — not the JIT `run`, which would include compile time.
- One in-program warm-up call before the timed region, so Node's JIT is hot
  (each process is fresh, so without it the timed run would be cold).
- Rust wraps inputs/outputs in `std::hint::black_box`, otherwise the optimizer
  constant-folds the literal-input workloads to nothing.
- Go's Mandelbrot wraps the products as `float64(a*b)` to block FMA
  contraction, so its float checksum matches the separately-rounded result the
  other four languages produce.
- Idiomatic, not transliterated: Rust/JS/Python use lazy iterator chains /
  generators; Go and otter use explicit loops / a hand-driven `Iterator`
  because neither has lazy combinators in its standard library.

These are micro-benchmarks. They measure exactly what each program does and
should not be read as a general ranking of the languages.

## otter runtime notes

Two small `otter_rt` additions back this suite (both also fix real gaps):

- `__of_now_ns()` — a monotonic nanosecond clock, exposed as `now_ns()` in
  `of:core`, for the in-program timing.
- `__of_print`/`__of_println` now flush stdout. An AOT executable returns
  through the codegen-emitted C `main`, bypassing Rust's flush-at-exit, so
  before this an AOT program printed nothing at all.
