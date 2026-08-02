#!/usr/bin/env python3
"""ASR-based intelligibility validation for espeak-ng-rs.

Synthesizes text with the Rust engine (and optionally the C oracle), transcribes
each clip with OpenAI Whisper, and reports word accuracy (1 - WER) against the
input. A low Rust score with a high oracle score means the synthesizer's audio
is not intelligible, independent of pitch/energy metrics.

Usage:
  python3 scripts/asr_validate.py --rust ./target/release/espeak-ng-rs \
      [--oracle /path/espeak-ng --data /path/espeak-ng-data] [--model base]
"""
import argparse, subprocess, tempfile, os, re, sys

def norm(s):
    return re.sub(r"[^a-z0-9 ]", " ", s.lower()).split()

def wer(ref, hyp):
    r, h = norm(ref), norm(hyp)
    d = [[0]*(len(h)+1) for _ in range(len(r)+1)]
    for i in range(len(r)+1): d[i][0] = i
    for j in range(len(h)+1): d[0][j] = j
    for i in range(1, len(r)+1):
        for j in range(1, len(h)+1):
            c = 0 if r[i-1]==h[j-1] else 1
            d[i][j] = min(d[i-1][j]+1, d[i][j-1]+1, d[i-1][j-1]+c)
    return d[len(r)][len(h)] / max(1, len(r))

def synth(cmd_prefix, text, wav, env=None):
    subprocess.run(cmd_prefix + ["-v","en","-w",wav,text], check=False,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env)
    return os.path.exists(wav) and os.path.getsize(wav) > 44

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rust", required=True)
    ap.add_argument("--oracle"); ap.add_argument("--data")
    ap.add_argument("--model", default="base",
                    help="whisper model: base (fast, default) … large-v3-turbo")
    ap.add_argument("--device", default="cpu",
                    help="cpu (default) or mps/cuda — use a GPU for the large model")
    args = ap.parse_args()

    import whisper
    model = whisper.load_model(args.model, device=args.device)
    fp16 = args.device != "cpu"
    def tr(wav):
        return model.transcribe(wav, language="en", fp16=fp16)["text"].strip()

    sentences = [
        "the quick brown fox jumps over the lazy dog",
        "hello world how are you today",
        "she sells sea shells by the sea shore",
        "the rain in spain falls mainly on the plain",
        "please call stella and ask her to bring these things",
    ]
    oenv = dict(os.environ, ESPEAK_DATA_PATH=args.data) if args.data else None
    td = tempfile.mkdtemp()
    rs, os_ = [], []
    print(f"{'text':45s} | rust acc | oracle acc")
    print("-"*72)
    for i, s in enumerate(sentences):
        rw = f"{td}/r{i}.wav"
        rust_acc = ""
        if synth([args.rust], s, rw):
            rust_acc = 1 - wer(s, tr(rw)); rs.append(rust_acc)
        oracle_acc = ""
        if args.oracle:
            ow = f"{td}/o{i}.wav"
            if synth([args.oracle], s, ow, oenv):
                oracle_acc = 1 - wer(s, tr(ow)); os_.append(oracle_acc)
        ra = f"{rust_acc:.2f}" if rust_acc!="" else " n/a"
        oa = f"{oracle_acc:.2f}" if oracle_acc!="" else " n/a"
        print(f"{s:45s} |   {ra}   |   {oa}")
    print("-"*72)
    if rs: print(f"{'MEAN word accuracy':45s} |   {sum(rs)/len(rs):.2f}   |   " +
                 (f"{sum(os_)/len(os_):.2f}" if os_ else " n/a"))
main()
