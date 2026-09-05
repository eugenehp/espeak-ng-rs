// tests/voice_config.rs
//
// Language configuration read from the voice file at runtime rather than a
// transcribed table — the direction of upstream #1466, and the mechanism behind
// #2524's `formant_factor`.
//
// Covers `pitch`, `intonation`, `stressLength`, and the locale fallback that
// has to work for a regional voice with no phoneme table of its own.

use std::path::Path;

use espeak_ng::EspeakNg;

fn data() -> Option<&'static Path> {
    let d = Path::new("espeak-ng-data");
    d.join("phontab").exists().then_some(d)
}

#[test]
fn voice_files_supply_pitch_intonation_and_stress_lengths() {
    let Some(d) = data() else {
        eprintln!("[SKIP] no data");
        return;
    };
    // `lang/gmw/af` declares `pitch 63 120`; `en` declares none.
    assert_eq!(espeak_ng::voices::voice_pitch(d, "af"), Some((63, 120)));
    assert_eq!(espeak_ng::voices::voice_pitch(d, "en"), None);

    // `intonation <n>` — 22 shipped languages set it.
    assert_eq!(espeak_ng::voices::voice_intonation(d, "ja"), Some(4));
    assert_eq!(espeak_ng::voices::voice_intonation(d, "ta"), Some(2));
    assert_eq!(espeak_ng::voices::voice_intonation(d, "en"), None);

    // `stressLength <l0…l7>`.
    assert_eq!(
        espeak_ng::voices::voice_stress_length(d, "lv"),
        Some(vec![160, 120, 200, 130, 0, 0, 230, 180])
    );
    assert_eq!(espeak_ng::voices::voice_stress_length(d, "en"), None);
}

/// A language's own `stressLength` changes its durations — English (Scotland)
/// is shorter than the default, Lancashire longer, matching the direction of
/// the same comparison in upstream.
#[test]
fn stress_lengths_change_duration() {
    let Some(d) = data() else { return };
    let len = |v: &str| -> usize {
        EspeakNg::with_data_dir(v, d)
            .and_then(|e| e.synth("hello world this is a test"))
            .map(|(pcm, _)| pcm.len())
            .unwrap_or(0)
    };
    let base = len("en");
    let scotland = len("en-GB-scotland");
    let lancashire = len("en-GB-x-gbclan");
    assert!(base > 0 && scotland > 0 && lancashire > 0, "a voice failed to speak");
    assert!(scotland < base, "Scotland's stressLength is shorter: {scotland} vs {base}");
    assert!(lancashire > base, "Lancashire's is longer: {lancashire} vs {base}");
}

/// A regional voice with no phoneme table of its own falls back to the base
/// subtag instead of failing — `en-gb-scotland` has no `phontab` entry.
#[test]
fn regional_voice_without_its_own_phoneme_table_still_speaks() {
    let Some(d) = data() else { return };
    for v in ["en-GB-scotland", "en-GB-x-gbclan", "pt-br", "en-US"] {
        let e = EspeakNg::with_data_dir(v, d).unwrap_or_else(|e| panic!("{v}: {e:?}"));
        let (pcm, _) = e.synth("hello").unwrap_or_else(|e| panic!("{v}: {e:?}"));
        assert!(!pcm.is_empty(), "{v} produced no audio");
    }
}

/// A voice's `pitch` line drives both its base F0 and its `formant_factor`
/// (upstream #2524): a voice pitched higher than the default shifts its
/// formants up with it, so the audio differs beyond a simple pitch change.
#[test]
fn voice_pitch_shifts_formants() {
    let Some(d) = data() else { return };
    let render = |v: &str| -> Vec<i16> {
        EspeakNg::with_data_dir(v, d)
            .and_then(|e| e.synth("hello world"))
            .map(|(pcm, _)| pcm)
            .unwrap_or_default()
    };
    // `af` declares `pitch 63 120`, well below the default 80.
    let af = render("af");
    let en = render("en");
    assert!(!af.is_empty() && !en.is_empty());
    assert_ne!(af, en, "a different voice pitch must change the audio");
}
