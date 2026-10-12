#!/usr/bin/env python3
"""Compare this port's text output against a built upstream espeak-ng, in bulk.

The corpus is upstream's own pronunciation test file — `tests/language-pronunciation.test`,
one UDHR-style sentence per shipped language — plus a symbol/emoji/suffix set that
exercises the paths those sentences don't reach.

Build the oracle once:

    git clone --depth 1 https://github.com/espeak-ng/espeak-ng /tmp/espeak-up
    cd /tmp/espeak-up && cmake -B build -DCMAKE_BUILD_TYPE=Release \\
        -DUSE_MBROLA=OFF -DUSE_LIBSONIC=OFF -DUSE_LIBPCAUDIO=OFF && cmake --build build

Then, from the repo root:

    cargo build --bin espeak-ng-rs
    scripts/oracle_sweep.py --upstream /tmp/espeak-up record oracle.txt --oracle
    scripts/oracle_sweep.py --upstream /tmp/espeak-up record port.txt
    scripts/oracle_sweep.py compare port.txt oracle.txt

`compare` prints how many cases are byte-identical and groups the rest by the kind
of difference.  Pass a third file (an earlier `record`) to see what a change moved:

    scripts/oracle_sweep.py compare after.txt oracle.txt --baseline before.txt
"""
import argparse
import os
import re
import subprocess
import sys
from collections import Counter

EXTRA_LANGS = ['en', 'de', 'fr', 'es', 'it', 'nl', 'ru', 'hu', 'pl', 'tr', 'cs', 'sv',
               'ja', 'he', 'uk', 'pt', 'fi', 'el']
EXTRA_TEXTS = ['a − b', '™', '©', '€', '±', '×', '÷', '°', '∑', '→', '§', '#', '%', '&',
               '@', '😀', '👍', '❤️', '5+3', '20°', '50%', '-5', 'a ( b', 'a , b',
               'a / b', 'a : b']
# Paths where one language's behaviour stands in for a whole mechanism.
SPOT_CASES = [
    ('en', "s's"), ('en', "s's's"), ('en', "dog's"), ('en', 'BBC'), ('en', 'USA'),
    ('en', "CD's"), ('en', 'thumbs up'), ('en', 'a half'),
    ('nl', 'vorige'), ('nl', 'nadelige'), ('nl', 'vorig'), ('nl', 'hoge'),
    ('de', 'beachten'), ('de', 'Gesicht'), ('de', 'Copyright'),
    ('fr', "aujourd'hui"), ('it', 'casa'), ('hu', 'legjobb'), ('ku', 'naxwaze'),
]


def corpus(upstream):
    """(lang, text) pairs: upstream's test corpus plus the extra cases."""
    pairs = []
    test_file = os.path.join(upstream, 'tests', 'language-pronunciation.test')
    src = open(test_file, encoding='utf-8').read()
    for m in re.finditer(r'^test_phon\s+(\S+)\s+"((?:[^"\\]|\\.)*)"\s+"((?:[^"\\]|\\.)*)"',
                         src, re.M):
        pairs.append((m.group(1), m.group(3)))
    pairs += [(lang, text) for lang in EXTRA_LANGS for text in EXTRA_TEXTS]
    pairs += SPOT_CASES
    return pairs


def record(args):
    if args.oracle:
        binary = os.path.join(args.upstream, 'build', 'src', 'espeak-ng')
        base = [binary, '-q', '-x']
        env = dict(os.environ, ESPEAK_DATA_PATH=os.path.join(args.upstream, 'build'))
    else:
        base = ['./target/debug/espeak-ng-rs', '-q', '-x', f'--path={args.data}']
        env = None
    pairs = corpus(args.upstream)
    with open(args.out, 'w', encoding='utf-8') as f:
        for lang, text in pairs:
            try:
                r = subprocess.run(base + ['-v', lang, '--', text],
                                   capture_output=True, text=True, timeout=30, env=env)
                phonemes = ' '.join(r.stdout.split())
            except Exception as exc:  # a hang or a crash is a result too
                phonemes = f'<ERROR {exc}>'
            f.write(f'{lang}\t{text}\t{phonemes}\n')
    print(f'{len(pairs)} cases -> {args.out}')


def load(path):
    out = {}
    for line in open(path, encoding='utf-8'):
        lang, text, phonemes = line.rstrip('\n').split('\t')
        out[(lang, text)] = phonemes
    return out


def classify(got, want):
    if got == '':
        return 'port silent'
    if want == '':
        return 'C silent'
    if got.replace("'", '').replace(',', '') == want.replace("'", '').replace(',', ''):
        return 'stress markers only'
    strip = lambda s: re.sub(r"[',_|:\- ]", '', s)
    if strip(got) == strip(want):
        return 'pause/spacing only'
    if len(got.split()) != len(want.split()):
        return 'word count'
    return 'phoneme differences'


def compare(args):
    port, oracle = load(args.port), load(args.oracle)
    keys = [k for k in port if k in oracle]
    same = [k for k in keys if port[k] == oracle[k]]
    print(f'byte-identical to upstream: {len(same)} / {len(keys)}')
    bad = [k for k in keys if port[k] != oracle[k]]
    print('remaining differences:', dict(Counter(classify(port[k], oracle[k]) for k in bad)))
    if args.baseline:
        before = load(args.baseline)
        changed = [k for k in keys if k in before and before[k] != port[k]]
        better = [k for k in changed if port[k] == oracle[k] and before[k] != oracle[k]]
        worse = [k for k in changed if before[k] == oracle[k] and port[k] != oracle[k]]
        print(f'vs baseline: {len(changed)} changed, {len(better)} now identical, '
              f'{len(worse)} no longer identical')
        for k in worse:
            print(f'  {k[0]} {k[1]!r}\n    was   [{before[k]}]\n    now   [{port[k]}]\n'
                  f'    C     [{oracle[k]}]')
    if args.verbose:
        for k in bad:
            print(f'{k[0]:>5} {k[1][:40]!r}\n    port[{port[k]}]\n    C   [{oracle[k]}]')


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--upstream', default='/tmp/espeak-up',
                    help='upstream checkout with build/ (default: /tmp/espeak-up)')
    ap.add_argument('--data', default='espeak-ng-data', help="this port's data directory")
    sub = ap.add_subparsers(dest='cmd', required=True)
    r = sub.add_parser('record', help='run the corpus through one engine')
    r.add_argument('out')
    r.add_argument('--oracle', action='store_true', help='use the upstream binary')
    r.set_defaults(func=record)
    c = sub.add_parser('compare', help='compare two recordings')
    c.add_argument('port')
    c.add_argument('oracle')
    c.add_argument('--baseline', help='an earlier recording of the port')
    c.add_argument('--verbose', action='store_true')
    c.set_defaults(func=compare)
    args = ap.parse_args()
    args.func(args)


if __name__ == '__main__':
    sys.exit(main())
