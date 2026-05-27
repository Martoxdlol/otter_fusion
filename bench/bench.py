#!/usr/bin/env python3
"""Cross-language benchmark driver.

Compares otter_fusion against Rust, Go, Node, and Python on a set of
common programs (Tier A) and language-feature workloads (Tier B).

Pipeline per benchmark:
  1. build   compile native targets (rust/go/otter) with optimizations
  2. verify  run each language once, compare stdout — all must agree
  3. time    run each command N times, record min/median/mean/stddev

otter_fusion note: the AOT binary's `print` never flushes (the C-style
exit bypasses Rust's buffered-stdout flush), so it emits nothing. The
compute still runs in full. We therefore TIME the AOT binary but VERIFY
correctness with the JIT `run` (which flushes). See README.md.

Usage:
  python3 bench.py                 # all benchmarks
  python3 bench.py fib sieve       # a subset
  python3 bench.py --runs 10       # more samples
  python3 bench.py --langs rust,go,otter
  python3 bench.py --skip-verify   # time only
"""

import argparse
import json
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent
PROGRAMS = ROOT / "programs"
BUILD = ROOT / "build"
RESULTS = ROOT / "results"

OTTER = REPO / "target" / "release" / "otter_fusion"
OTTER_RT = REPO / "target" / "release" / "libotter_rt.a"

ALL_LANGS = ["rust", "go", "node", "python", "otter"]

# How each language maps a benchmark dir to a runnable command.
#   ext     source file extension under programs/<bench>/<bench>.<ext>
#   build   (name, srcs) -> (build_cmd | None, error_if_tool_missing)
#   run     (name) -> argv used for timing
#   verify  (name) -> argv used for the correctness check (defaults to run)


def native_bin(name, lang):
    return BUILD / f"{name}_{lang}"


def src(name, ext):
    return PROGRAMS / name / f"{name}.{ext}"


LANGS = {
    "rust": {
        "ext": "rs",
        "tool": "rustc",
        "build": lambda n: ["rustc", "-O", "-o", str(native_bin(n, "rust")), str(src(n, "rs"))],
        "run": lambda n: [str(native_bin(n, "rust"))],
    },
    "go": {
        "ext": "go",
        "tool": "go",
        "build": lambda n: ["go", "build", "-o", str(native_bin(n, "go")), str(src(n, "go"))],
        "run": lambda n: [str(native_bin(n, "go"))],
    },
    "node": {
        "ext": "js",
        "tool": "node",
        "build": None,
        "run": lambda n: ["node", str(src(n, "js"))],
    },
    "python": {
        "ext": "py",
        "tool": "python3",
        "build": None,
        "run": lambda n: ["python3", str(src(n, "py"))],
    },
    "otter": {
        "ext": "of",
        "tool": str(OTTER),
        # build = compile to object then link against libotter_rt.a
        "build": "otter_special",
        "run": lambda n: [str(native_bin(n, "otter"))],
    },
}


def have_tool(lang):
    tool = LANGS[lang]["tool"]
    if "/" in tool:
        return Path(tool).exists()
    return shutil.which(tool) is not None


def build_otter(name):
    obj = BUILD / f"{name}.o"
    binary = native_bin(name, "otter")
    r = subprocess.run(
        [str(OTTER), "compile", str(src(name, "of")), "-o", str(obj)],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        return f"compile failed:\n{r.stdout}\n{r.stderr}"
    r = subprocess.run(
        ["cc", str(obj), str(OTTER_RT), "-o", str(binary)],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        return f"link failed:\n{r.stderr}"
    return None


def build(name, langs):
    BUILD.mkdir(exist_ok=True)
    errors = {}
    for lang in langs:
        if not src(name, LANGS[lang]["ext"]).exists():
            continue
        spec = LANGS[lang]
        if spec["build"] is None:
            continue
        if spec["build"] == "otter_special":
            err = build_otter(name)
        else:
            r = subprocess.run(spec["build"](name), capture_output=True, text=True)
            err = None if r.returncode == 0 else (r.stdout + r.stderr)
        if err:
            errors[lang] = err
    return errors


def run_once(argv):
    """Run argv. Programs print two lines: the answer, then in-process compute
    nanoseconds. Returns (wall_seconds, answer, compute_ns_or_None, returncode)."""
    t0 = time.perf_counter()
    r = subprocess.run(argv, capture_output=True, text=True, cwd=str(PROGRAMS))
    wall = time.perf_counter() - t0
    lines = r.stdout.strip().splitlines()
    answer = lines[0].strip() if lines else ""
    compute_ns = None
    if len(lines) >= 2 and lines[1].strip().lstrip("-").isdigit():
        compute_ns = int(lines[1].strip())
    return wall, answer, compute_ns, r.returncode


def verify(name, langs):
    """Run each language once; compare the answer line. Returns (outputs, agree)."""
    outputs = {}
    for lang in langs:
        if not src(name, LANGS[lang]["ext"]).exists():
            continue
        argv = LANGS[lang]["run"](name)
        _, answer, _, rc = run_once(argv)
        outputs[lang] = answer if rc == 0 else f"<exit {rc}>"
    return outputs, len(set(outputs.values())) <= 1


def _stats(samples):
    return {
        "min": min(samples),
        "median": statistics.median(samples),
        "mean": statistics.fmean(samples),
        "stdev": statistics.stdev(samples) if len(samples) > 1 else 0.0,
    }


def time_bench(name, langs, runs, warmup):
    """Record both end-to-end wall time (includes startup) and in-process
    compute time (the program's own clock around its hot region)."""
    results = {}
    for lang in langs:
        if not src(name, LANGS[lang]["ext"]).exists():
            continue
        argv = LANGS[lang]["run"](name)
        if "/" in argv[0] and not Path(argv[0]).exists():
            continue  # native build missing
        for _ in range(warmup):
            run_once(argv)
        wall, comp = [], []
        for _ in range(runs):
            w, _a, cns, _rc = run_once(argv)
            wall.append(w)
            if cns is not None:
                comp.append(cns / 1e9)  # ns -> seconds
        entry = {"wall": _stats(wall)}
        if comp:
            entry["compute"] = _stats(comp)
        results[lang] = entry
    return results


def fmt_table(name, timings):
    # Rank by in-process compute when available, else wall.
    def key(kv):
        return kv[1].get("compute", kv[1]["wall"])["median"]

    rows = sorted(timings.items(), key=key)
    if not rows:
        return f"### {name}\n\n_(no results)_\n"
    fastest = key(rows[0])
    lines = [f"### {name}\n",
             "| lang | compute | end-to-end | startup≈ | vs fastest |",
             "|------|--------:|-----------:|---------:|-----------:|"]
    for lang, s in rows:
        wall_ms = s["wall"]["median"] * 1000
        if "compute" in s:
            comp_ms = s["compute"]["median"] * 1000
            startup = wall_ms - comp_ms
            ratio = (s["compute"]["median"] / fastest) if fastest else 0
            lines.append(f"| {lang} | {comp_ms:.1f} ms | {wall_ms:.1f} ms "
                         f"| {startup:.1f} ms | {ratio:.2f}× |")
        else:
            lines.append(f"| {lang} | — | {wall_ms:.1f} ms | — | — |")
    return "\n".join(lines) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("benches", nargs="*", help="benchmark names (default: all)")
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--warmup", type=int, default=1)
    ap.add_argument("--langs", default=",".join(ALL_LANGS))
    ap.add_argument("--skip-verify", action="store_true")
    args = ap.parse_args()

    langs = [l for l in args.langs.split(",") if l in LANGS]
    missing = [l for l in langs if not have_tool(l)]
    for l in missing:
        print(f"!! toolchain for '{l}' not found — skipping", file=sys.stderr)
    langs = [l for l in langs if l not in missing]

    available = sorted(p.name for p in PROGRAMS.iterdir() if p.is_dir()) if PROGRAMS.exists() else []
    benches = args.benches or available
    benches = [b for b in benches if b in available]

    RESULTS.mkdir(exist_ok=True)
    report = [f"# Benchmark results\n\nlangs: {', '.join(langs)} | runs: {args.runs} | warmup: {args.warmup}\n"]
    full = {}

    for name in benches:
        print(f"\n=== {name} ===")
        berr = build(name, langs)
        for lang, err in berr.items():
            print(f"  [build:{lang}] FAILED\n{err}")

        if not args.skip_verify:
            outputs, agree = verify(name, langs)
            status = "OK" if agree else "MISMATCH"
            print(f"  verify [{status}]: {outputs}")
            if not agree:
                report.append(f"### {name}\n\n**OUTPUT MISMATCH**: `{outputs}`\n")
                full[name] = {"verify": outputs, "agree": False}
                continue

        timings = time_bench(name, langs, args.runs, args.warmup)

        def _key(kv):
            return kv[1].get("compute", kv[1]["wall"])["median"]

        for lang, s in sorted(timings.items(), key=_key):
            wall_ms = s["wall"]["median"] * 1000
            comp_ms = s["compute"]["median"] * 1000 if "compute" in s else None
            comp_str = f"{comp_ms:8.1f}" if comp_ms is not None else "     n/a"
            print(f"  {lang:8s} compute {comp_str} ms   end-to-end {wall_ms:8.1f} ms")
        report.append(fmt_table(name, timings))
        full[name] = {"timings": timings}

    (RESULTS / "results.json").write_text(json.dumps(full, indent=2))
    (RESULTS / "RESULTS.md").write_text("\n".join(report))
    print(f"\nwrote {RESULTS/'RESULTS.md'} and {RESULTS/'results.json'}")


if __name__ == "__main__":
    main()
