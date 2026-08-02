// tests/voices_cli.rs
//
// CLI smoke tests for `--voices` / `--voices=<lang>`.  Skips automatically when
// the local `espeak-ng-data/lang/` tree is absent.

use std::path::Path;
use std::process::Command;

fn have_data() -> bool {
    Path::new("espeak-ng-data/lang").exists()
}

#[test]
fn voices_flag_lists_voices() {
    if !have_data() {
        eprintln!("[SKIP] no local espeak-ng-data");
        return;
    }
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let out = Command::new(bin).arg("--voices").output().expect("run --voices");
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.starts_with("Pty Language"), "missing header:\n{s}");
    assert!(s.contains("Afrikaans"), "missing Afrikaans:\n{s}");
    assert!(s.contains("gmw/af"), "missing file path:\n{s}");
    assert!(s.lines().count() > 50, "too few voices: {}", s.lines().count());
}

#[test]
fn voice_name_resolves_to_language() {
    if !have_data() {
        return;
    }
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let ipa = |v: &str, text: &str| -> String {
        let out = Command::new(bin)
            .args(["-v", v, "--ipa", "-q", text])
            .output()
            .expect("run");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    // A voice *name* resolves to the same phonemes as its language tag.
    assert!(!ipa("Afrikaans", "hello").is_empty(), "Afrikaans produced nothing");
    assert_eq!(ipa("Afrikaans", "hello"), ipa("af", "hello"));
    assert_eq!(ipa("French (France)", "bonjour"), ipa("fr", "bonjour"));
    // Plain language tags are unaffected.
    assert!(!ipa("en", "hello").is_empty());
}

#[test]
fn missing_variant_warns_but_still_speaks_base() {
    if !have_data() {
        return;
    }
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let out = Command::new(bin)
        .args(["-v", "en+nope", "--ipa", "hi"])
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("variant 'nope' not found"), "no warning:\n{stderr}");
    assert!(stdout.trim() == "hˈaɪ", "base voice should still speak: {stdout:?}");

    // A valid variant must NOT warn.
    let ok = Command::new(bin)
        .args(["-v", "en+f3", "--ipa", "hi"])
        .output()
        .expect("run");
    assert!(
        !String::from_utf8_lossy(&ok.stderr).contains("not found"),
        "valid variant should not warn"
    );
}

#[test]
fn voices_filter_restricts_output() {
    if !have_data() {
        return;
    }
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let out = Command::new(bin).arg("--voices=fr").output().expect("run --voices=fr");
    let s = String::from_utf8_lossy(&out.stdout);
    // French voices present; unrelated ones filtered out.
    assert!(s.to_lowercase().contains("french"), "no French voice:\n{s}");
    assert!(!s.contains("Afrikaans"), "filter leaked Afrikaans:\n{s}");
}
