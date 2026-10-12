// tests/cli_audio_options.rs
//
// The audio-side options: `--split`, `--ssml-break`, `--load`, and the SSML
// `<break>` durations they build on.
//
// Pause lengths are checked as audio *deltas* against the same text without the
// tag, which is what upstream's own `--ssml-break` arithmetic controls; the
// values match a locally built upstream `espeak-ng` (`ba90c8e`) to within a few
// milliseconds.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_espeak_cli")
}

/// A temp path unique to this process *and* this call — the tests in this file
/// run in parallel, so a shared name would race.
fn tmp(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("espeak-rs-{}-{n}-{name}", std::process::id()))
}

/// Length of a mono 16-bit WAV, in milliseconds.
fn wav_ms(path: &Path) -> u64 {
    let bytes = std::fs::read(path).expect("read wav");
    assert!(bytes.len() > 44, "{} is not a WAV", path.display());
    let rate = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as u64;
    ((bytes.len() as u64 - 44) / 2) * 1000 / rate
}

/// Synthesize to a WAV and return its length in ms.
fn speak_ms(args: &[&str]) -> u64 {
    let out = tmp("speak.wav");
    let status = Command::new(bin())
        .args(args)
        .arg("-w")
        .arg(&out)
        .status()
        .expect("run");
    assert!(status.success());
    let ms = wav_ms(&out);
    let _ = std::fs::remove_file(&out);
    ms
}

macro_rules! skip_without_en {
    () => {
        if !Path::new("espeak-ng-data/en_dict").exists() {
            eprintln!("[SKIP] no en data");
            return;
        }
    };
}

/// `<break>`: C's six `strength` lengths, and `time` as an explicit duration.
/// Only `medium` and above are clause boundaries; the weaker ones are a
/// prepause on the next word, and `none` is no pause at all.
#[test]
fn break_lengths_follow_the_tag() {
    skip_without_en!();
    let plain = speak_ms(&["one two"]);
    let delta = |markup: &str| speak_ms(&["-m", markup]) as i64 - plain as i64;

    // `time` is wall-clock exact, whatever the speaking rate.
    assert!((delta(r#"one<break time="2s"/>two"#) - 2000).abs() < 60);
    assert!((delta(r#"one<break time="500ms"/>two"#) - 500).abs() < 60);
    assert!((delta(r#"one<break time="1.5s"/>two"#) - 1500).abs() < 60);
    // `strength` is in clause units, which follow the voice's rate, so these
    // are only checked for order and for being in the right neighbourhood.
    let weak = delta(r#"one<break strength="weak"/>two"#);
    let medium = delta(r#"one<break strength="medium"/>two"#);
    let strong = delta(r#"one<break strength="x-strong"/>two"#);
    assert!(weak > 50 && weak < 200, "weak = {weak}");
    assert!(medium > weak, "medium {medium} <= weak {weak}");
    assert!(strong > medium + 300, "x-strong {strong} vs medium {medium}");
    // `none` adds nothing — but still keeps the two words apart.
    assert_eq!(delta(r#"one<break strength="none"/>two"#), 0);
}

/// `--ssml-break=<percentage>` scales `<break time=…>` and nothing else.
#[test]
fn ssml_break_scales_the_time_attribute() {
    skip_without_en!();
    let plain = speak_ms(&["one two"]);
    let delta = |args: &[&str]| speak_ms(args) as i64 - plain as i64;
    let text = r#"one<break time="1s"/>two"#;

    assert!((delta(&["-m", "--ssml-break=100", text]) - 1000).abs() < 60);
    assert!((delta(&["-m", "--ssml-break=50", text]) - 500).abs() < 60);
    assert!((delta(&["-m", "--ssml-break=200", text]) - 2000).abs() < 60);
    // The option takes a required argument, so the space-separated form works
    // and does not swallow the text.
    assert!((delta(&["-m", "--ssml-break", "50", text]) - 500).abs() < 60);
    // `strength` is not scaled.
    let s = r#"one<break strength="x-strong"/>two"#;
    assert_eq!(delta(&["-m", "--ssml-break=50", s]), delta(&["-m", s]));
}

/// `--split=<minutes>`: a new file at the first sentence boundary past the
/// limit, numbered `_01`, `_02`, … with the extension kept.
#[test]
fn split_writes_numbered_parts() {
    skip_without_en!();
    let text = (1..=30)
        .map(|i| format!("Hello world this is sentence number {i}."))
        .collect::<Vec<_>>()
        .join(" ");
    let base = tmp("split.wav");
    let part = |n: u32| base.with_file_name(format!(
        "{}_{n:02}.wav",
        base.file_stem().unwrap().to_string_lossy()
    ));
    for n in 1..=3 {
        let _ = std::fs::remove_file(part(n));
    }
    let status = Command::new(bin())
        .args(["--split=1", "-w"])
        .arg(&base)
        .arg(&text)
        .status()
        .expect("run");
    assert!(status.success());

    assert!(!base.exists(), "the unsplit name must not be written");
    assert!(part(1).exists(), "{} missing", part(1).display());
    assert!(part(2).exists(), "{} missing", part(2).display());
    // The first part runs past the one-minute limit (it is cut at the next
    // sentence boundary), and the whole text is still there.
    let first = wav_ms(&part(1));
    assert!(first > 60_000, "first part is only {first} ms");
    let total: u64 = (1..=2).map(|n| wav_ms(&part(n))).sum();
    let whole = speak_ms(&[&text]);
    assert!(total.abs_diff(whole) < 500, "split {total} ms vs whole {whole} ms");
    for n in 1..=2 {
        let _ = std::fs::remove_file(part(n));
    }
    // Without --split the plain name is used.
    assert!(!part(3).exists());
}

/// `--load`: the `-v` value is a voice *file*.  Its language picks the
/// translation and its `pitch` line reaches the synthesizer.
#[test]
fn load_reads_a_voice_definition_from_a_file() {
    if !Path::new("espeak-ng-data/de_dict").exists() {
        eprintln!("[SKIP] no de data");
        return;
    }
    let file = tmp("voice");
    std::fs::write(&file, "name test\nlanguage de\npitch 60 100\n").unwrap();
    let x = |args: &[&str]| {
        let out = Command::new(bin()).args(args).output().expect("run");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let loaded = x(&["-q", "-x", "--load", "-v", file.to_str().unwrap(), "hallo welt"]);
    assert_eq!(loaded, x(&["-q", "-x", "-v", "de", "hallo welt"]));

    // The file's `pitch` is applied, so the audio is not the stock voice's.
    let a = tmp("load-a.wav");
    let b = tmp("load-b.wav");
    for (args, out) in [
        (vec!["--load", "-v", file.to_str().unwrap()], &a),
        (vec!["-v", "de"], &b),
    ] {
        let status = Command::new(bin())
            .args(&args)
            .arg("-w")
            .arg(out)
            .arg("hallo welt")
            .status()
            .expect("run");
        assert!(status.success());
    }
    assert_ne!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());

    // A path that is not a voice definition is reported, not spoken.
    let missing = x(&["--load", "-v", "/nonexistent/voice", "hallo"]);
    assert!(missing.is_empty(), "spoke anyway: {missing:?}");

    for f in [&file, &a, &b] {
        let _ = std::fs::remove_file(f);
    }
}
