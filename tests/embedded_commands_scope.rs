// tests/embedded_commands_scope.rs
//
// Inline embedded commands (`\x01[±]<value><letter>`) take effect **from where
// they appear**, not for the whole utterance — the same as upstream, where
// `DoEmbedded` is called as the phoneme list is walked.  Rate `S`, pitch `P` and
// amplitude `A` are covered here; the values are in the same units as `-s`,
// `-p` and `-a`.

use std::path::Path;
use std::process::Command;

fn samples(markup: &[&str]) -> usize {
    let out = Command::new(env!("CARGO_BIN_EXE_espeak_cli"))
        .args(markup)
        .output()
        .expect("run cli");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    out.stdout.len().saturating_sub(44) / 2
}

fn have_data() -> bool {
    Path::new("espeak-ng-data/en_dict").exists()
}

#[test]
fn embedded_rate_applies_from_its_position() {
    if !have_data() {
        eprintln!("[SKIP] no en data");
        return;
    }
    const TEXT: &str = "one two three four";
    let normal = samples(&["--stdout", TEXT]);
    let from_start = samples(&["--stdout", &format!("\u{1}20S {TEXT}")]);
    let midway = samples(&["--stdout", "one two \u{1}20S three four"]);

    assert!(normal > 0, "no audio");
    assert!(from_start > normal, "a slow rate should lengthen: {from_start} vs {normal}");
    // The words before the command keep the normal rate, so slowing from the
    // middle lengthens *less* than slowing from the start.
    assert!(
        midway < from_start && midway > normal,
        "the command must apply from its position: mid={midway} start={from_start} normal={normal}"
    );

    // A fast rate shortens.
    let fast = samples(&["--stdout", &format!("\u{1}400S {TEXT}")]);
    assert!(fast < normal, "a fast rate should shorten: {fast} vs {normal}");
}

/// The rate value is clamped to the `-s` range, so an extreme command can't
/// stretch the audio without bound.
#[test]
fn embedded_rate_is_clamped() {
    if !have_data() {
        return;
    }
    let at_min = samples(&["--stdout", "\u{1}80S one two three"]);
    let below_min = samples(&["--stdout", "\u{1}1S one two three"]);
    assert_eq!(at_min, below_min, "rates below 80 wpm clamp to 80");
}

/// A `±` command is relative to the value already in force.
#[test]
fn relative_commands_stack() {
    if !have_data() {
        return;
    }
    let absolute = samples(&["--stdout", "\u{1}100S one two three"]);
    // No space between the two commands: a space is a word separator and would
    // add an inter-word gap that the single-command form doesn't have.
    let relative = samples(&["--stdout", "\u{1}175S\u{1}-75S one two three"]);
    assert_eq!(absolute, relative, "175 − 75 should equal an absolute 100");
}

/// Pitch and amplitude commands are accepted and change the audio without
/// changing its length.
#[test]
fn pitch_and_amplitude_commands_apply() {
    if !have_data() {
        return;
    }
    let run = |text: &str| -> Vec<u8> {
        Command::new(env!("CARGO_BIN_EXE_espeak_cli"))
            .args(["--stdout", text])
            .output()
            .expect("run")
            .stdout
    };
    let normal = run("one two three");
    let high = run("one \u{1}90P two three");
    let quiet = run("one \u{1}20A two three");

    let close = |a: usize, b: usize| (a as f64 - b as f64).abs() / (a as f64) < 0.02;
    assert!(close(normal.len(), high.len()),
            "pitch should barely change the duration: {} vs {}", normal.len(), high.len());
    assert_ne!(normal, high, "a pitch command should change the audio");
    assert!(close(normal.len(), quiet.len()),
            "amplitude should not change the duration: {} vs {}", normal.len(), quiet.len());
    assert_ne!(normal, quiet, "an amplitude command should change the audio");
}
