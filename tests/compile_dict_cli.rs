// tests/compile_dict_cli.rs
//
// End-to-end test for `--compile[=<lang>]`: compile a synthetic dictsource
// (`<lang>_list` + `<lang>_rules`) into a binary `<lang>_dict`, then prove the
// freshly-compiled dict actually drives lookup via `-x`.
//
// Self-contained: it copies only the small phoneme files `PhonemeData::load`
// needs (phontab/phonindex/intonations) into a scratch dir, so it never touches
// the real `en_dict`.  Skips when the local `espeak-ng-data/` tree is absent.

use std::path::{Path, PathBuf};
use std::process::Command;

fn data_root() -> Option<PathBuf> {
    let d = PathBuf::from("espeak-ng-data");
    if d.join("phontab").exists() && d.join("phonindex").exists() {
        Some(d)
    } else {
        None
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn compile_dict_then_lookup() {
    let Some(data) = data_root() else {
        eprintln!("[SKIP] no local espeak-ng-data");
        return;
    };
    let dir = scratch("compile_dict_cli");

    // Everything `PhonemeData::load` requires for mnemonic resolution.
    for f in ["phontab", "phonindex", "intonations"] {
        std::fs::copy(data.join(f), dir.join(f)).unwrap_or_else(|e| panic!("copy {f}: {e}"));
    }
    // A tiny synthetic dictsource for language "en".
    std::fs::write(dir.join("en_list"), "hello  h@l'oU\nworld  w3:ld\n").unwrap();
    std::fs::write(dir.join("en_rules"), ".group h\n\th  h\n.group w\n\tw  w\n").unwrap();

    let bin = env!("CARGO_BIN_EXE_espeak_cli");

    // Compile.
    let out = Command::new(bin)
        .args(["--compile=en", "--path"])
        .arg(&dir)
        .output()
        .expect("run --compile");
    assert!(out.status.success(), "compile failed: {}", String::from_utf8_lossy(&out.stderr));
    let dict = dir.join("en_dict");
    assert!(dict.exists(), "en_dict was not written");
    assert!(std::fs::metadata(&dict).unwrap().len() > 8, "en_dict is empty/header-only");

    // The compiled dict must actually resolve the words we put in it.
    let ipa = |word: &str| -> String {
        let out = Command::new(bin)
            .args(["-q", "-x", "-v", "en", "--path"])
            .arg(&dir)
            .arg(word)
            .output()
            .expect("run -x");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    assert_eq!(ipa("hello"), "h@l'oU", "compiled dict did not drive 'hello' lookup");
    // 'world' gains a stress mark from stress placement (w'3:ld); assert the stem.
    assert!(ipa("world").contains("3:ld"), "compiled dict did not drive 'world' lookup");
}

#[test]
fn compile_missing_source_errors_cleanly() {
    let Some(data) = data_root() else { return; };
    let dir = scratch("compile_dict_cli_missing");
    for f in ["phontab", "phonindex", "intonations"] {
        std::fs::copy(data.join(f), dir.join(f)).unwrap();
    }
    // No zz_list / zz_rules present.
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let out = Command::new(bin)
        .args(["--compile=zz", "--path"])
        .arg(&dir)
        .output()
        .expect("run --compile");
    // A missing dictsource is a clean failure (non-zero exit, message), not a panic.
    assert!(!out.status.success(), "expected failure for missing source");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("zz"), "error should mention the language: {err}");
    assert!(!dir.join("zz_dict").exists(), "no dict should be written on failure");
}
