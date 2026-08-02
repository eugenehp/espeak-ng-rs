// tests/prosody_cli.rs
//
// Document-level `<prosody rate>` scales the whole utterance's speed in the
// audio path (verifiable by WAV length).  Scoped/partial prosody is not applied
// globally.  Skips when en data is absent.

use std::path::Path;
use std::process::Command;

fn wav_samples(bytes: &[u8]) -> usize {
    bytes.len().saturating_sub(44) / 2
}

#[test]
fn document_prosody_rate_changes_wav_length() {
    if !Path::new("espeak-ng-data/en_dict").exists() {
        eprintln!("[SKIP] no en data");
        return;
    }
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let run = |markup: &str| -> usize {
        let out = Command::new(bin)
            .args(["-m", "--stdout", markup])
            .output()
            .expect("run");
        wav_samples(&out.stdout)
    };
    let text = "hello world this is a test";
    let normal = run(text);
    let slow = run(&format!(r#"<prosody rate="50%">{text}</prosody>"#));
    let fast = run(&format!(r#"<prosody rate="200%">{text}</prosody>"#));
    assert!(normal > 0, "no audio produced");
    assert!(slow > normal, "rate 50% should lengthen audio: {slow} vs {normal}");
    assert!(fast < normal, "rate 200% should shorten audio: {fast} vs {normal}");

    // A prosody span that does NOT wrap everything must not be applied globally.
    let scoped = run(&format!(r#"a <prosody rate="50%">{text}</prosody> b"#));
    let scoped_ref = run(&format!("a {text} b"));
    assert_eq!(scoped, scoped_ref, "scoped prosody must not change global rate");
}
