// tests/trace_cli.rs
//
// `-X` (translation-rule trace) and `-D` (deterministic noise seed).
//
// The `-X` format is upstream's: a `Translate '<word>'` header or a `Found:`
// line for a dictionary hit, then `<score>\t<rule> [<phonemes>]` per matching
// rule, a blank line between letter positions, and the resulting phonemes last.
// Values here were taken from a locally built upstream binary.

use std::path::{Path, PathBuf};
use std::process::Command;

fn data_dir() -> PathBuf {
    std::env::var("ESPEAK_DATA_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("espeak-ng-data"))
}

fn run(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_espeak_cli"))
        .args(args)
        .env("ESPEAK_DATA_PATH", data_dir())
        .output()
        .expect("run cli");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A word translated by rules: header, scored rules, then the phonemes.
/// Byte-identical to upstream for this word.
#[test]
fn trace_matches_upstream_for_a_rules_word() {
    let out = run(&["-X", "-q", "-v", "en", "cat"]);
    assert_eq!(
        out,
        "Translate 'cat'\n  \
         1\tc        [k]\n\n  \
         1\ta        [a]\n\n  \
         1\tt        [t]\n\n\
         k'at\n",
        "trace differs:\n{out}"
    );
}

/// A dictionary hit prints `Found:` with the decoded flags and no rule lines.
#[test]
fn trace_reports_a_dictionary_hit() {
    let out = run(&["-X", "-q", "-v", "en", "the"]);
    assert!(out.starts_with("Found: 'the' [D@2]"), "{out}");
    assert!(out.contains("$nounf"), "dictionary flags should be decoded: {out}");
    assert!(out.trim_end().ends_with("D'@"), "phonemes follow the trace: {out}");
}

/// Rule scores and the rendered rule text carry the pre/post context.
#[test]
fn trace_renders_rule_context() {
    let out = run(&["-X", "-q", "-v", "en", "hi"]);
    // `h) i (_` — pre-context `h`, group `i`, post-context word-end.
    assert!(out.contains(" 43\th) i (_  [,aI]"), "{out}");
    assert!(out.contains("  1\th        [h]"), "{out}");
    assert!(out.trim_end().ends_with("h'aI"), "{out}");
}

/// `-D` is accepted and doesn't change a run that uses no unvoiced noise; the
/// port is deterministic either way (it never seeds from the clock).
#[test]
fn deterministic_flag_is_accepted() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("trace_cli");
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b) = (dir.join("a.wav"), dir.join("b.wav"));

    run(&["-D", "-v", "en", "-w", a.to_str().unwrap(), "hello world"]);
    run(&["-v", "en", "-w", b.to_str().unwrap(), "hello world"]);
    assert!(std::fs::metadata(&a).unwrap().len() > 1000, "no audio written");
    assert_eq!(
        std::fs::read(&a).unwrap().len(),
        std::fs::read(&b).unwrap().len(),
        "the seed must not change the utterance length"
    );

    // Repeat runs are byte-identical with or without the flag.
    let c = dir.join("c.wav");
    run(&["-D", "-v", "en", "-w", c.to_str().unwrap(), "hello world"]);
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&c).unwrap(), "not reproducible");
}
