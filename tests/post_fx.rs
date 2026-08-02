// tests/post_fx.rs
//
// Post-synthesis libsonic tempo/pitch scaling on the engine
// (`EspeakNg::with_post_tempo` / `with_post_pitch`).  The default (factors 1.0)
// must be byte-identical to no post-processing.  Skips without local data.

use std::path::Path;

use espeak_ng::EspeakNg;

fn engine() -> Option<EspeakNg> {
    let dir = Path::new("espeak-ng-data");
    if !dir.join("phontab").exists() {
        eprintln!("[SKIP] no local espeak-ng-data");
        return None;
    }
    EspeakNg::builder().voice("en").data_dir(dir).build().ok()
}

#[test]
fn default_post_fx_is_byte_identical() {
    let Some(e) = engine() else { return };
    let base = e.synth("hello there world").expect("synth").0;
    // Explicit 1.0 factors must equal the default (no post-processing).
    let same = engine().unwrap().with_post_tempo(1.0).with_post_pitch(1.0)
        .synth("hello there world").expect("synth").0;
    assert_eq!(base, same, "1.0 tempo/pitch must be byte-identical to default");
}

#[test]
fn post_tempo_shortens_audio() {
    let Some(e) = engine() else { return };
    let base = e.synth("hello there world").expect("synth").0;
    let fast = engine().unwrap().with_post_tempo(1.6)
        .synth("hello there world").expect("synth").0;
    assert!(fast.len() < base.len(), "faster tempo → shorter: {} vs {}", fast.len(), base.len());
    let ratio = fast.len() as f64 / base.len() as f64;
    assert!((0.40..0.85).contains(&ratio), "unexpected length ratio {ratio:.2}");
}

#[test]
fn post_pitch_preserves_length_and_alters_audio() {
    let Some(e) = engine() else { return };
    let base = e.synth("hello there world").expect("synth").0;
    let shifted = engine().unwrap().with_post_pitch(1.4)
        .synth("hello there world").expect("synth").0;
    assert_eq!(shifted.len(), base.len(), "pitch shift preserves duration");
    assert_ne!(shifted, base, "pitch shift must change the audio");
}
