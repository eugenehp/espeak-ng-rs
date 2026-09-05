// tests/prosody_cli.rs
//
// `<prosody rate>` scales speech rate in the audio path (verifiable by WAV
// length).  The change is **scoped**: it applies to the words inside the span
// only, which is how upstream implements it — the SSML element writes embedded
// `\x01<n>S` commands into the text and the synthesizer picks them up as it
// reaches them.  Skips when en data is absent.

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

    // A span that wraps only part of the text slows just that part: longer than
    // no prosody at all, shorter than the same span scaled over everything.
    let scoped = run(&format!(r#"a <prosody rate="50%">{text}</prosody> b"#));
    let scoped_ref = run(&format!("a {text} b"));
    let all_slow = run(&format!(r#"<prosody rate="50%">a {text} b</prosody>"#));
    assert!(scoped > scoped_ref, "scoped prosody should slow its span: {scoped} vs {scoped_ref}");
    assert!(
        scoped < all_slow,
        "the words outside the span should stay at the normal rate: {scoped} vs {all_slow}"
    );

    // The span closes: text after `</prosody>` is back to normal.
    let trailing = run(&format!(r#"<prosody rate="50%">a</prosody> {text}"#));
    let leading = run(&format!(r#"a <prosody rate="50%">{text}</prosody>"#));
    assert!(trailing < leading, "the change must not leak past the closing tag");
}
