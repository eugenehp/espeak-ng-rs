#!/usr/bin/env python3
"""compare_frames.py — diff C-reference vs Rust synthesizer frame dumps.

Enables sample-exact verification of the coarticulation port (GAPS §36).

  scripts/dump_c_frames.sh <espeak-ng-src> "hello" c.txt
  ESPEAK_RS_DUMP_FRAMES=r.txt ./target/release/espeak-ng-rs -q -w /dev/null "hello"
  python3 scripts/compare_frames.py c.txt r.txt

The C dump records two passes per vowel (`which=1` onset, `which=2` body); the
Rust dump currently emits one block per phoneme.  The report shows, per phoneme
in order, the frame counts and each frame's length/rms/formants so divergences
(missing onset/body split, wrong formant targets, wrong lengths) are visible.
"""
import sys, re

def parse(path):
    blocks = []
    cur = None
    for line in open(path):
        m = re.match(r"PH (\S*)\s*(?:which=(\d+))?\s*nf=(\d+)", line)
        if m:
            cur = {"ph": m.group(1), "which": m.group(2), "frames": []}
            blocks.append(cur)
        elif cur is not None:
            fm = re.match(r"\s+len=(\d+) rms=(\d+) ffreq=([\d,]+)", line)
            if fm:
                cur["frames"].append((int(fm.group(1)), int(fm.group(2)),
                                      [int(x) for x in fm.group(3).split(",") if x != ""]))
    return blocks

def fmt(b):
    w = f" which={b['which']}" if b["which"] else ""
    out = [f"PH {b['ph']}{w} nf={len(b['frames'])}"]
    for (ln, rms, ff) in b["frames"]:
        out.append(f"    len={ln:<3} rms={rms:<3} F1-3={ff[1]:>4},{ff[2]:>4},{ff[3]:>4}")
    return out

def main():
    if len(sys.argv) != 3:
        print("usage: compare_frames.py <c.txt> <rust.txt>"); sys.exit(2)
    c, r = parse(sys.argv[1]), parse(sys.argv[2])
    print(f"C reference: {len(c)} blocks   Rust: {len(r)} blocks\n")
    print(f"{'C (reference)':45s} | Rust")
    print("-" * 90)
    ci = ri = 0
    while ci < len(c) or ri < len(r):
        cl = fmt(c[ci]) if ci < len(c) else []
        rl = fmt(r[ri]) if ri < len(r) else []
        for k in range(max(len(cl), len(rl))):
            left = cl[k] if k < len(cl) else ""
            right = rl[k] if k < len(rl) else ""
            print(f"{left:45s} | {right}")
        print()
        ci += 1; ri += 1

main()
