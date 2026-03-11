#!/usr/bin/env bash
# benches/bench.sh
#
# Runs the benchmark suite and writes BENCHMARK.md with tables + SVG charts.
#
# Usage
# -----
#   ./benches/bench.sh                          # auto-detect espeak-ng
#   ./benches/bench.sh --bundled                # build espeak-ng from source
#   ./benches/bench.sh --filter "encoding"      # only matching groups
#   ./benches/bench.sh --no-run                 # parse existing results only
#
# Output
# ------
#   BENCHMARK.md        Human-readable summary with tables and images
#   benches/results/    Raw Criterion JSON + SVG snapshots (commit these)

set -euo pipefail

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

FEATURES=""
BENCH_FILTER=""
RUN_BENCH=true
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RESULTS_DIR="$SCRIPT_DIR/results"
CRITERION_DIR="$REPO_ROOT/target/criterion"
OUTPUT_MD="$REPO_ROOT/BENCHMARK.md"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundled)  FEATURES="bundled-espeak"; shift ;;
    --filter)   BENCH_FILTER="$2"; shift 2 ;;
    --no-run)   RUN_BENCH=false; shift ;;
    -h|--help)
      sed -n '3,20p' "$0" | sed 's/^# //'
      exit 0
      ;;
    *) echo "Unknown argument: $1" >&2; exit 1 ;;
  esac
done

# ---------------------------------------------------------------------------
# 1. Run benchmarks
# ---------------------------------------------------------------------------

if $RUN_BENCH; then
  echo "==> Running benchmarks…"
  CARGO_ARGS=(bench)
  [[ -n "$FEATURES" ]] && CARGO_ARGS+=(--features "$FEATURES")
  [[ -n "$BENCH_FILTER" ]] && CARGO_ARGS+=(-- "$BENCH_FILTER")
  cd "$REPO_ROOT"
  cargo "${CARGO_ARGS[@]}"
  echo ""
fi

# ---------------------------------------------------------------------------
# 2. Snapshot JSON + SVGs into benches/results/
# ---------------------------------------------------------------------------

echo "==> Snapshotting Criterion outputs to $RESULTS_DIR/"
mkdir -p "$RESULTS_DIR"

JSON_COUNT=0
SVG_COUNT=0

# --- JSON: one estimates.json + benchmark.json per individual benchmark ---
while IFS= read -r est_json; do
  bench_json="$(dirname "$est_json")/benchmark.json"
  [[ -f "$bench_json" ]] || continue

  rel="${est_json#"$CRITERION_DIR/"}"
  rel="${rel%/new/estimates.json}"

  dest_dir="$RESULTS_DIR/$rel"
  mkdir -p "$dest_dir"
  cp "$est_json"   "$dest_dir/estimates.json"
  cp "$bench_json" "$dest_dir/benchmark.json"
  JSON_COUNT=$((JSON_COUNT + 1))
done < <(find "$CRITERION_DIR" -path "*/new/estimates.json" 2>/dev/null | sort)

# --- SVGs: top-level group report images only ---
# violin.svg  — always present; shows distribution of all inputs/functions
# lines.svg   — present when group has throughput; shows scaling line chart
while IFS= read -r svg; do
  # Only keep the two-level group summary: <group_dir>/report/{violin,lines}.svg
  # Skip deeper paths like <group>/<function>/report/... or <group>/<input>/...
  rel="${svg#"$CRITERION_DIR/"}"          # e.g. encoding_utf8_decode/report/violin.svg
  depth=$(echo "$rel" | tr -cd '/' | wc -c)
  [[ "$depth" -eq 2 ]] || continue       # exactly group_dir/report/file.svg

  dest="$RESULTS_DIR/$rel"
  mkdir -p "$(dirname "$dest")"
  cp "$svg" "$dest"
  SVG_COUNT=$((SVG_COUNT + 1))
done < <(find "$CRITERION_DIR" -maxdepth 3 \( -name "violin.svg" -o -name "lines.svg" \) 2>/dev/null | sort)

echo "    saved $JSON_COUNT JSON entries, $SVG_COUNT SVG charts"

# ---------------------------------------------------------------------------
# 3. Parse all JSON snapshots into a TSV
#    columns: group_id | function | input | mean_str | err_str | throughput | dir_name
# ---------------------------------------------------------------------------

TSV_FILE="$(mktemp)"
trap 'rm -f "$TSV_FILE"' EXIT

python3 - "$RESULTS_DIR" "$TSV_FILE" <<'PYEOF'
import json, os, sys

results_dir = sys.argv[1]
tsv_path    = sys.argv[2]

def fmt_time(ns):
    if   ns >= 1e9:  return f"{ns/1e9:.3f} s"
    elif ns >= 1e6:  return f"{ns/1e6:.3f} ms"
    elif ns >= 1e3:  return f"{ns/1e3:.3f} µs"
    else:            return f"{ns:.3f} ns"

def fmt_throughput(mean_ns, throughput):
    if not throughput:
        return "-"
    if "Bytes" in throughput:
        bps = throughput["Bytes"] / (mean_ns / 1e9)
        if   bps >= 1e9: return f"{bps/1e9:.2f} GB/s"
        elif bps >= 1e6: return f"{bps/1e6:.2f} MB/s"
        else:            return f"{bps/1e3:.2f} KB/s"
    if "Elements" in throughput:
        return f"{throughput['Elements'] / (mean_ns/1e9):.0f} elem/s"
    return "-"

rows = []
for root, dirs, files in os.walk(results_dir):
    dirs.sort()
    if "estimates.json" not in files or "benchmark.json" not in files:
        continue
    with open(os.path.join(root, "estimates.json")) as f:
        est = json.load(f)
    with open(os.path.join(root, "benchmark.json")) as f:
        bench = json.load(f)

    mean   = est["mean"]["point_estimate"]
    stddev = est["std_dev"]["point_estimate"]

    row = (
        bench["group_id"],                       # 0
        bench.get("function_id") or "",          # 1
        bench.get("value_str")   or "",          # 2
        fmt_time(mean),                          # 3
        f"±{fmt_time(stddev)}",                  # 4
        fmt_throughput(mean, bench.get("throughput")),  # 5
        bench.get("directory_name", "").split("/")[0],  # 6  criterion dir prefix
    )
    rows.append(row)

rows.sort()
with open(tsv_path, "w") as f:
    for row in rows:
        f.write("\t".join(row) + "\n")
PYEOF

echo "    parsed $(wc -l < "$TSV_FILE" | tr -d ' ') benchmark entries"

# ---------------------------------------------------------------------------
# 4. Write BENCHMARK.md
# ---------------------------------------------------------------------------

echo "==> Generating $OUTPUT_MD"

TIMESTAMP=$(date -u '+%Y-%m-%d %H:%M UTC')
RUSTC_VER=$(rustc --version 2>/dev/null || echo "unknown")
ESPEAK_VER=$(espeak-ng --version 2>/dev/null | head -1 || echo "not installed")
OS_INFO=$(uname -srm)

{
cat <<HEADER
# Benchmark Results

Generated: $TIMESTAMP  
Platform: \`$OS_INFO\`  
Rust: \`$RUSTC_VER\`  
eSpeak NG: \`$ESPEAK_VER\`

> **Reading this file**  
> Times are wall-clock per operation (lower is better).  
> Throughput is input bytes or elements processed per second (higher is better).  
> **Rust** rows show the pure-Rust implementation; rows marked **c\_cli** call
> the \`espeak-ng\` binary as a subprocess (includes process-spawn overhead).  
> During the stub phase, Rust \`text_to_ipa\` rows measure error-path overhead
> only and will be replaced by real numbers as each module is implemented.

---

HEADER

# One section per group
python3 - "$TSV_FILE" "$RESULTS_DIR" <<'PYEOF'
import sys, os
from itertools import groupby

rows = []
with open(sys.argv[1]) as f:
    for line in f:
        parts = line.rstrip("\n").split("\t")
        if len(parts) == 7:
            rows.append(parts)

results_dir = sys.argv[2]

for group_id, group_rows in groupby(rows, key=lambda r: r[0]):
    group_rows = list(group_rows)

    # Criterion dir name for this group (first element of directory_name field)
    dir_name = group_rows[0][6] if group_rows else ""

    has_throughput = any(r[5] != "-" for r in group_rows)
    has_input      = any(r[2] for r in group_rows)

    print(f"## {group_id}\n")

    # --- Table ---
    if has_input and has_throughput:
        headers = ["Function", "Input",    "Mean", "±Std", "Throughput"]
        cols    = [1,          2,          3,      4,      5          ]
    elif has_input:
        headers = ["Function", "Input",    "Mean", "±Std"]
        cols    = [1,          2,          3,      4     ]
    elif has_throughput:
        headers = ["Function", "Mean", "±Std", "Throughput"]
        cols    = [1,          3,      4,      5          ]
    else:
        headers = ["Function", "Mean", "±Std"]
        cols    = [1,          3,      4     ]

    widths = [max(len(h), max((len(r[c]) for r in group_rows), default=0))
              for h, c in zip(headers, cols)]

    print("| " + " | ".join(h.ljust(w) for h, w in zip(headers, widths)) + " |")
    print("| " + " | ".join("-" * w for w in widths) + " |")
    for r in group_rows:
        print("| " + " | ".join(r[c].ljust(w) for c, w in zip(cols, widths)) + " |")
    print()

    # --- Charts ---
    # violin.svg is always the group-level distribution plot
    violin = os.path.join(results_dir, dir_name, "report", "violin.svg")
    lines  = os.path.join(results_dir, dir_name, "report", "lines.svg")

    # Paths relative to BENCHMARK.md (which sits at the repo root)
    rel_violin = os.path.relpath(violin, start=os.path.dirname(os.path.dirname(results_dir)))
    rel_lines  = os.path.relpath(lines,  start=os.path.dirname(os.path.dirname(results_dir)))

    imgs = []
    if os.path.exists(violin):
        imgs.append(f"![{group_id} violin plot]({rel_violin})")
    if os.path.exists(lines):
        imgs.append(f"![{group_id} throughput]({rel_lines})")

    if imgs:
        print("\n".join(imgs))
        print()

PYEOF

cat <<FOOTER
---

## Notes

- Times are [criterion](https://github.com/bheisler/criterion.rs) means over
  100 samples (15 for CLI subprocess groups).
- **c\_cli** benchmarks include subprocess spawn + espeak-ng initialisation +
  data file loading on every call — this is the real-world latency a caller
  would see when shelling out to \`espeak-ng\`.
- The **bundled-espeak** feature (\`cargo bench --features bundled-espeak\`)
  downloads and compiles espeak-ng from source so the C baseline runs even
  without a system installation.
- Once the Rust \`translate\` module is implemented, the **rust** rows in the
  \`text_to_ipa\` groups will reflect actual pipeline performance.
- Charts are Criterion's SVG output copied into \`benches/results/\` so they
  render directly in GitHub without needing \`target/\` to be checked in.

## Re-running

\`\`\`bash
# Using system espeak-ng (must be on PATH)
./benches/bench.sh

# Building espeak-ng from source automatically
./benches/bench.sh --bundled

# Only encoding benchmarks
./benches/bench.sh --filter encoding

# Parse existing results without re-running
./benches/bench.sh --no-run
\`\`\`
FOOTER
} > "$OUTPUT_MD"

echo ""
echo "==> Done."
echo "    $OUTPUT_MD"
echo "    $RESULTS_DIR/ ($(find "$RESULTS_DIR" -name "*.json" | wc -l | tr -d ' ') JSON, $(find "$RESULTS_DIR" -name "*.svg" | wc -l | tr -d ' ') SVG)"
