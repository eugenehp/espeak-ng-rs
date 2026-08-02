#!/usr/bin/env bash
#
# dump_c_frames.sh — instrument eSpeak NG's synthesizer to dump the generated
# formant frame sequence for a word, giving a sample-exact reference for
# verifying the Rust coarticulation port (GAPS §36, task #9).
#
# It patches `DoSpect2` (synthdata's LookupSpect output) to append, for each
# phoneme, its mnemonic, the pass (`which` = 1 vowel-onset / 2 vowel-body /
# 0 other), and per-frame `length`, `rms`, and formant frequencies `ffreq[0..6]`
# to the file named by $ESPEAK_DUMP_FRAMES.  Idempotent; rebuilds in place.
#
# Usage:
#   scripts/dump_c_frames.sh <espeak-ng-src-dir> "<text>" [out.txt]
#
# The source dir must contain a CMake `build/` (e.g. from a prior oracle build).
# Compare the result with the Rust side:
#   ESPEAK_RS_DUMP_FRAMES=rust.txt ./target/release/espeak-ng-rs -q -w /dev/null "<text>"
#   python3 scripts/compare_frames.py out.txt rust.txt
set -euo pipefail

SRC="${1:?usage: dump_c_frames.sh <espeak-ng-src> <text> [out.txt]}"
TEXT="${2:-hello}"
OUT="${3:-/tmp/c_frames.txt}"
F="$SRC/src/libespeak-ng/synthesize.c"
BUILD="$SRC/build"

if ! grep -q ESPEAK_DUMP_FRAMES "$F"; then
  python3 - "$F" <<'PY'
import sys
f = sys.argv[1]; s = open(f).read()
anchor = ('\tframes = LookupSpect(this_ph, which, fmt_params, &n_frames, plist);\n'
          '\tif (frames == NULL)\n\t\treturn 0; // not found\n')
assert anchor in s, "LookupSpect anchor not found (espeak-ng version mismatch?)"
dump = ('\t{ const char *_dfn=getenv("ESPEAK_DUMP_FRAMES"); if(_dfn){ FILE*_df=fopen(_dfn,"a");'
        ' if(_df){ fputs("PH ",_df); for(int _b=0;_b<4;_b++){char _c=(this_ph->mnemonic>>(_b*8))&0xff;'
        ' if(_c)fputc(_c,_df);} fprintf(_df," which=%d nf=%d\\n",which,n_frames);'
        ' for(int _i=0;_i<n_frames;_i++){frame_t*_fr=frames[_i].frame;'
        ' fprintf(_df,"  len=%d rms=%d ffreq=%d,%d,%d,%d,%d,%d,%d\\n",frames[_i].length,_fr->rms,'
        '_fr->ffreq[0],_fr->ffreq[1],_fr->ffreq[2],_fr->ffreq[3],_fr->ffreq[4],_fr->ffreq[5],_fr->ffreq[6]);}'
        ' fclose(_df);} } }\n')
s = s.replace(anchor, anchor + dump, 1)
if "#include <stdlib.h>" not in s:
    s = s.replace("#include <stdio.h>", "#include <stdio.h>\n#include <stdlib.h>", 1)
open(f, "w").write(s)
print("instrumented synthesize.c")
PY
fi

cmake --build "$BUILD" >/tmp/dump_c_build.log 2>&1
BIN="$BUILD/src/espeak-ng"
DATA=""
for d in "$BUILD/espeak-ng-data" "$SRC/espeak-ng-data" "$(dirname "$SRC")/espeak-install/share/espeak-ng-data"; do
  [ -d "$d" ] && DATA="$d" && break
done

rm -f "$OUT"
export ESPEAK_DUMP_FRAMES="$OUT"
[ -n "$DATA" ] && export ESPEAK_DATA_PATH="$DATA"
"$BIN" -q -w /dev/null "$TEXT" 2>/dev/null || true
if [ ! -s "$OUT" ]; then
  echo "ERROR: no frames written (binary=$BIN data=${DATA:-<none>})" >&2
  exit 1
fi
echo "C frames → $OUT ($(wc -l < "$OUT") lines)"
head -12 "$OUT"
