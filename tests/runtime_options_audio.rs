// tests/runtime_options_audio.rs
//
// The runtime text options `--punct`, `-g` (word gap) and `-k` (capitals) must
// reach the WAV / `synth` audio path, not only `-x`/`--ipa`.  Previously the
// engine built its own translator without them.  Skips when data is absent.

use espeak_ng::{EspeakNg, Parameter};

fn engine() -> Option<EspeakNg> {
    let e = EspeakNg::new("en").ok()?;
    // Confirm synthesis actually works with the resolved data.
    if e.synth("x").map(|(s, _)| s.is_empty()).unwrap_or(true) {
        eprintln!("[SKIP] synthesis unavailable");
        return None;
    }
    Some(e)
}

fn samples(e: &EspeakNg, text: &str) -> usize {
    e.synth(text).expect("synth").0.len()
}

#[test]
fn punct_flag_lengthens_audio() {
    let Some(mut e) = engine() else { return };
    let base = samples(&e, "a, b, c.");
    e.set_punctuation_list(Some(Vec::new())); // announce all punctuation
    let punct = samples(&e, "a, b, c.");
    assert!(punct > base, "--punct should lengthen audio: {punct} vs {base}");
}

#[test]
fn word_gap_lengthens_audio() {
    let Some(mut e) = engine() else { return };
    let base = samples(&e, "one two three four");
    e.set_parameter(Parameter::WordGap, 30);
    let gapped = samples(&e, "one two three four");
    assert!(gapped > base, "-g should lengthen audio: {gapped} vs {base}");
}

#[test]
fn set_voice_by_query_resolves_name_and_language() {
    use espeak_ng::voices::VoiceQuery;
    let Some(mut e) = engine() else { return };
    // By human-readable voice name (which plain set_voice can't resolve).
    assert!(e.set_voice_by_query(&VoiceQuery { name: Some("Afrikaans".into()), ..Default::default() }));
    assert_eq!(e.current_voice().language.as_deref(), Some("af"));
    // By language with locale fall-back: "en" → the highest-priority en-gb voice.
    assert!(e.set_voice_by_query(&VoiceQuery { language: Some("en".into()), ..Default::default() }));
    assert_eq!(e.current_voice().language.as_deref(), Some("en-gb"));
    // No match leaves a clean `false`.
    assert!(!e.set_voice_by_query(&VoiceQuery { language: Some("zzq".into()), ..Default::default() }));
}

#[test]
fn say_as_interpret_applies_in_audio() {
    // The interpret modes (ordinal/date/time) were only applied on `-x`/`--ipa`;
    // the audio path now applies them too — so `<say-as ordinal>3</say-as>` reads
    // "third" (== "3rd"), not "three".
    let Some(mut e) = engine() else { return };
    e.set_markup(true);
    let ordinal = e.synth(r#"<say-as interpret-as="ordinal">3</say-as>"#).unwrap().0;
    let date = e.synth(r#"<say-as interpret-as="date">2024-01-15</say-as>"#).unwrap().0;
    e.set_markup(false);
    assert_eq!(ordinal, e.synth("3rd").unwrap().0, "audio ordinal must read '3rd'");
    assert_eq!(date, e.synth("January 15th 2024").unwrap().0, "audio date must read worded form");
}

#[test]
fn markup_tags_are_not_spoken_in_audio() {
    // Regression: the audio markup path only handled <audio>, so other SSML tags
    // (emphasis/prosody/say-as) were spoken literally.  They must be stripped.
    let Some(mut e) = engine() else { return };
    let plain = samples(&e, "hello");
    e.set_markup(true);
    let tagged = samples(&e, "<emphasis>hello</emphasis>");
    assert_eq!(tagged, plain, "SSML tags should be stripped in audio, not spoken");
    // Entities are decoded too.
    let entity = samples(&e, "Tom &amp; Jerry");
    let plain_amp = {
        let mut e2 = EspeakNg::new("en").unwrap();
        e2.set_markup(true);
        samples(&e2, "Tom & Jerry")
    };
    // "&amp;" → "&", so it must sound like the literal symbol.  (That is the
    // dictionary's `& _and` — "and" after a short pause — not the bare word.)
    assert_eq!(entity, plain_amp, "entities should decode in audio");
}

#[test]
fn no_final_pause_shortens_audio() {
    let Some(mut e) = engine() else { return };
    let with = samples(&e, "hello");
    e.set_no_final_pause(true);
    let without = samples(&e, "hello");
    assert!(without < with, "-z should shorten audio: {without} vs {with}");
    // The removed span is the ~0.30 s end-of-utterance trailing silence.
    let removed_secs = (with - without) as f64 / 22_050.0;
    assert!(removed_secs > 0.2, "expected ~0.3s removed, got {removed_secs:.3}s");
}

#[test]
fn capitals_flag_lengthens_audio() {
    let Some(mut e) = engine() else { return };
    let base = samples(&e, "Cat Dog");
    e.set_parameter(Parameter::Capitals, 2); // announce "capital"
    let announced = samples(&e, "Cat Dog");
    assert!(announced > base, "-k 2 should lengthen audio: {announced} vs {base}");
}
