// tests/compile_phonemes_cli.rs
//
// End-to-end test for `--compile-phonemes`: compile a synthetic phoneme-table
// source into a binary `phontab`, then confirm the file parses back to the same
// tables.  Fully self-contained (no shipped data needed).

use std::path::{Path, PathBuf};
use std::process::Command;

use espeak_ng::phoneme::load::parse_phontab;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

const SOURCE: &str = "\
phonemetable base
phoneme _
\tpause
\tlength 20
endphoneme
phoneme p
\tvls blb stp
\tlengthmod 2
endphoneme
phoneme a
\tvwl
\tlength 180
endphoneme
";

#[test]
fn compile_phonemes_then_parse() {
    let dir = scratch("compile_phonemes_cli");
    std::fs::write(dir.join("phonemes"), SOURCE).unwrap();

    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let out = Command::new(bin)
        .args(["--compile-phonemes", "--path"])
        .arg(&dir)
        .output()
        .expect("run --compile-phonemes");
    assert!(out.status.success(), "compile failed: {}", String::from_utf8_lossy(&out.stderr));

    let phontab = dir.join("phontab");
    assert!(phontab.exists(), "phontab was not written");

    // The compiled table parses back to the same phonemes.
    let bytes = std::fs::read(&phontab).unwrap();
    let tables = parse_phontab(&bytes).expect("parse compiled phontab");
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "base");
    let mnemonics: Vec<String> = tables[0].phonemes.iter().map(|p| p.mnemonic_str()).collect();
    assert_eq!(mnemonics, vec!["_", "p", "a"]);
    assert_eq!(tables[0].phonemes[2].std_length, 180, "'a' length preserved");
}

#[test]
fn compile_phonemes_missing_source_errors() {
    let dir = scratch("compile_phonemes_cli_missing");
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let out = Command::new(bin)
        .args(["--compile-phonemes", "--path"])
        .arg(&dir)
        .output()
        .expect("run");
    assert!(!out.status.success(), "missing source should fail");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("not found"), "clear error expected: {err}");
    assert!(!dir.join("phontab").exists(), "no phontab written on failure");
}
