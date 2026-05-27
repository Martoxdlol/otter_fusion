#!/usr/bin/env python3
"""Render results/results.json as a grouped bar chart (log scale) to an SVG.

No third-party deps — emits hand-built SVG so it renders anywhere (incl. the
GitHub README). Usage: python3 plot.py
"""

import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parent
RESULTS = ROOT / "results" / "results.json"
OUT = ROOT / "results" / "benchmarks.svg"

# Fixed display order. Tier A (common programs) then Tier B (features).
BENCHES = ["fib", "sieve", "mandelbrot", "interfaces", "iterators", "generics", "unions"]
LANGS = ["rust", "go", "otter", "node", "python"]
COLORS = {
    "rust":   "#b07219",
    "go":     "#00add8",
    "otter":  "#e8590c",   # the subject — highlighted
    "node":   "#5a9e4b",
    "python": "#4b8bbe",
}

# Canvas
W, H = 940, 520
PAD_L, PAD_R, PAD_T, PAD_B = 60, 16, 64, 64
PLOT_W = W - PAD_L - PAD_R
PLOT_H = H - PAD_T - PAD_B

Y_MIN, Y_MAX = 2.0, 4000.0  # ms, log scale


def median_ms(entry):
    """In-process compute median (startup excluded); fall back to wall time."""
    t = entry.get("compute", entry.get("wall"))
    return t["median"] * 1000.0


def y_of(ms):
    ms = max(ms, Y_MIN)
    f = (math.log10(ms) - math.log10(Y_MIN)) / (math.log10(Y_MAX) - math.log10(Y_MIN))
    return PAD_T + PLOT_H * (1 - f)


def esc(s):
    return str(s).replace("&", "&amp;").replace("<", "&lt;")


def main():
    data = json.loads(RESULTS.read_text())
    benches = [b for b in BENCHES if b in data and "timings" in data[b]]

    s = []
    s.append(f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" '
             f'viewBox="0 0 {W} {H}" font-family="-apple-system,Segoe UI,Roboto,sans-serif">')
    s.append(f'<rect width="{W}" height="{H}" fill="#ffffff"/>')
    s.append(f'<text x="{PAD_L}" y="26" font-size="17" font-weight="700" fill="#1a1a1a">'
             f'otter_fusion vs Rust · Go · Node · Python</text>')
    s.append(f'<text x="{PAD_L}" y="44" font-size="12" fill="#666">'
             f'in-process compute time (startup excluded), lower is better · log scale (ms)</text>')

    # Horizontal gridlines + y labels at 10/100/1000 ms.
    for tick in (10, 100, 1000):
        y = y_of(tick)
        s.append(f'<line x1="{PAD_L}" y1="{y:.1f}" x2="{W-PAD_R}" y2="{y:.1f}" '
                 f'stroke="#eee" stroke-width="1"/>')
        s.append(f'<text x="{PAD_L-8}" y="{y+4:.1f}" font-size="11" fill="#999" '
                 f'text-anchor="end">{tick} ms</text>')

    group_w = PLOT_W / len(benches)
    bar_gap = 3
    bar_w = (group_w * 0.74 - bar_gap * (len(LANGS) - 1)) / len(LANGS)
    base_y = PAD_T + PLOT_H

    for gi, bench in enumerate(benches):
        gx = PAD_L + group_w * gi
        timings = data[bench]["timings"]
        start = gx + group_w * 0.13
        for li, lang in enumerate(LANGS):
            if lang not in timings:
                continue
            ms = median_ms(timings[lang])
            x = start + li * (bar_w + bar_gap)
            y = y_of(ms)
            stroke = ' stroke="#7a2e00" stroke-width="1"' if lang == "otter" else ""
            s.append(f'<rect x="{x:.1f}" y="{y:.1f}" width="{bar_w:.1f}" '
                     f'height="{base_y-y:.1f}" fill="{COLORS[lang]}"{stroke} rx="1.5"/>')
            label = f"{ms:.0f}" if ms >= 10 else f"{ms:.1f}"
            s.append(f'<text x="{x+bar_w/2:.1f}" y="{y-3:.1f}" font-size="8.5" '
                     f'fill="#555" text-anchor="middle">{label}</text>')
        # benchmark name under the group
        s.append(f'<text x="{gx+group_w/2:.1f}" y="{base_y+18:.1f}" font-size="12" '
                 f'fill="#333" text-anchor="middle" font-weight="600">{esc(bench)}</text>')

    # axis baseline
    s.append(f'<line x1="{PAD_L}" y1="{base_y}" x2="{W-PAD_R}" y2="{base_y}" '
             f'stroke="#ccc" stroke-width="1"/>')

    # legend
    lx, ly = PAD_L, H - 22
    for lang in LANGS:
        s.append(f'<rect x="{lx}" y="{ly-9}" width="11" height="11" '
                 f'fill="{COLORS[lang]}" rx="2"/>')
        weight = "700" if lang == "otter" else "400"
        s.append(f'<text x="{lx+16}" y="{ly}" font-size="12" fill="#333" '
                 f'font-weight="{weight}">{lang}</text>')
        lx += 26 + len(lang) * 8 + 26

    s.append('</svg>')
    OUT.write_text("\n".join(s))
    print(f"wrote {OUT}")

    # Also emit a PNG (some renderers handle SVG poorly). Best-effort.
    png = OUT.with_suffix(".png")
    try:
        import cairosvg
        cairosvg.svg2png(url=str(OUT), write_to=str(png), output_width=W * 2)
        print(f"wrote {png}")
    except Exception as e:
        print(f"(skipped PNG: {e}; `pip install cairosvg` to enable)")


if __name__ == "__main__":
    main()
