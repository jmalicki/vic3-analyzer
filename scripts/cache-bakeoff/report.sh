#!/usr/bin/env bash
# Aggregate bake-off JSON results into a markdown report.
set -euo pipefail
RESULTS_DIR="${1:?results dir}"
OUT_MD="${2:?out.md}"

python3 - "$RESULTS_DIR" "$OUT_MD" <<'PY'
import json, sys
from pathlib import Path

results_dir = Path(sys.argv[1])
out_md = Path(sys.argv[2])
rows = []
for p in sorted(results_dir.glob("*.json")):
    rows.append(json.loads(p.read_text()))
if not rows:
    raise SystemExit("no results")

def human(n):
    n = float(n)
    for u in ["B", "KB", "MB", "GB", "TB"]:
        if n < 1024:
            return f"{n:.1f} {u}"
        n /= 1024
    return f"{n:.1f} PB"

baseline = next(r for r in rows if r["name"] == "zstd-3-long30-actions")
for r in rows:
    r["ratio_vs_baseline"] = round(r["size_bytes"] / baseline["size_bytes"], 4)

ranked = sorted(rows, key=lambda r: r["size_bytes"])
winner = ranked[0]
compile_s = baseline["compile_s"]
fetch_s = baseline["fetch_s"]
fetch_plus = baseline["fetch_plus_compile_s"]
tar_bytes = baseline["tar_bytes"]

lines = [
    "# Rust cache compression bake-off (Tauri e2e ubuntu-24.04 corpus)",
    "",
    f"- Corpus: `cargo-home` + `target` after `cargo build -p vic3-analyzer --features webdriver` (debug)",
    f"- Uncompressed tar: **{human(tar_bytes)}**",
    f"- Cold fetch: **{fetch_s}s**; cold compile: **{compile_s}s**; fetch+compile: **{fetch_plus}s**",
    f"- Actions baseline: `{baseline['name']}` = **{human(baseline['size_bytes'])}**",
    "",
    "| name | size | compress | decompress | vs baseline | vs compile (c/d) | vs fetch+compile (c/d) |",
    "|------|------|----------|------------|-------------|------------------|------------------------|",
]
for r in ranked:
    lines.append(
        f"| `{r['name']}` | {human(r['size_bytes'])} | {r['compress_s']}s | {r['decompress_s']}s | "
        f"{r['ratio_vs_baseline']} | {r['compress_vs_compile']}/{r['decompress_vs_compile']} | "
        f"{r['compress_vs_fetch_plus_compile']}/{r['decompress_vs_fetch_plus_compile']} |"
    )
lines += [
    "",
    f"**Winner (smallest among reasonable restores): `{winner['name']}`** at {human(winner['size_bytes'])} "
    f"({winner['ratio_vs_baseline']}× vs Actions baseline).",
    "",
    "Criterion: **space-first within reason** — minimize size; reject slow decompress "
    "(paid on every restore). Slow compress is more tolerable (paid mainly on cache save).",
]
text = "\n".join(lines) + "\n"
out_md.write_text(text)
print(text)
winner_path = out_md.with_name("winner.txt")
winner_path.write_text(winner["name"] + "\n")
PY
