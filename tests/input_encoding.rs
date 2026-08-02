// tests/input_encoding.rs
//
// `-b <n>` selects the input text encoding for `-f`/`--stdin` (1=UTF-8,
// 2=8-bit/Latin-1, 4=16-bit).  Verifies a non-UTF-8 file decodes correctly.
// Skips when the en data is absent.

use std::io::Write;
use std::path::Path;
use std::process::Command;

#[test]
fn dash_b_2_decodes_latin1_file() {
    if !Path::new("espeak-ng-data/en_dict").exists() {
        eprintln!("[SKIP] no en data");
        return;
    }
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let path = std::env::temp_dir().join("espeak_rs_latin1_input.txt");
    // "café" in Latin-1: é = 0xE9, which is NOT valid UTF-8 on its own.
    std::fs::File::create(&path).unwrap().write_all(b"caf\xe9").unwrap();

    // Default (UTF-8) rejects the invalid byte → no phoneme output.
    let utf8 = Command::new(bin)
        .args(["--ipa", "-q", "-f"])
        .arg(&path)
        .output()
        .expect("run utf8");
    assert!(
        String::from_utf8_lossy(&utf8.stdout).trim().is_empty(),
        "UTF-8 read should fail on the invalid byte"
    );

    // `-b 2` (Latin-1) decodes é and pronounces "café".
    let latin1 = Command::new(bin)
        .args(["--ipa", "-q", "-b", "2", "-f"])
        .arg(&path)
        .output()
        .expect("run latin1");
    let out = String::from_utf8_lossy(&latin1.stdout);
    assert!(out.trim_start().starts_with("kaf"), "expected café-like IPA, got {out:?}");

    let _ = std::fs::remove_file(&path);
}
