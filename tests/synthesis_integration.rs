// tests/synthesis_integration.rs
//
// Integration tests for the full text → PCM synthesis pipeline.
//
// These tests verify end-to-end correctness using the real espeak-ng data
// files (phondata, phonindex, phontab, *_dict).  They gracefully skip if
// the data files are not present at the default path.

use espeak_ng::{text_to_pcm, text_to_ipa, Synthesizer, VoiceParams};

mod common;
use common::{data_available, ESPEAK_DATA_DIR};

// ---------------------------------------------------------------------------
// Helper — check that a PCM buffer is non-trivially non-silent
// ---------------------------------------------------------------------------

fn has_audio(pcm: &[i16], min_peak: i16) -> bool {
    pcm.iter().any(|&s| s.unsigned_abs() > min_peak as u16)
}



// ---------------------------------------------------------------------------
// text_to_pcm — basic smoke tests
// ---------------------------------------------------------------------------

#[test]
fn text_to_pcm_en_hello() {
    if !data_available() {
        println!("[SKIP] espeak-ng data not found at {ESPEAK_DATA_DIR}");
        return;
    }
    let (pcm, rate) = text_to_pcm("en", "hello").expect("synthesis should succeed");
    assert_eq!(rate, 22050);
    assert!(!pcm.is_empty(), "PCM buffer should be non-empty");
    assert!(has_audio(&pcm, 100), "Expected audible audio for 'hello'");
    // At least 100ms of audio (2205 samples)
    assert!(pcm.len() > 2200,
        "Expected at least 100ms of audio, got {} samples", pcm.len());
}

#[test]
fn text_to_pcm_en_hello_world() {
    if !data_available() {
        println!("[SKIP] espeak-ng data not found at {ESPEAK_DATA_DIR}");
        return;
    }
    let (pcm, rate) = text_to_pcm("en", "hello world").expect("synthesis ok");
    assert_eq!(rate, 22050);
    assert!(pcm.len() > 4000, "Two-word utterance should be longer");
    assert!(has_audio(&pcm, 100));
}

#[test]
fn text_to_pcm_empty_string() {
    if !data_available() {
        println!("[SKIP] espeak-ng data not found at {ESPEAK_DATA_DIR}");
        return;
    }
    // Empty input should return empty or very short PCM
    let (pcm, _) = text_to_pcm("en", "").expect("empty input should not error");
    assert!(pcm.len() < 100, "Empty string → very short PCM");
}

#[test]
fn text_to_pcm_longer_phrase_is_longer() {
    if !data_available() {
        println!("[SKIP] espeak-ng data not found at {ESPEAK_DATA_DIR}");
        return;
    }
    let (short_pcm, _) = text_to_pcm("en", "hi").expect("short ok");
    let (long_pcm, _) = text_to_pcm("en", "hello beautiful world").expect("long ok");
    assert!(long_pcm.len() > short_pcm.len(),
        "Longer text should produce more samples: short={}, long={}",
        short_pcm.len(), long_pcm.len());
}

#[test]
fn text_to_pcm_de_guten_tag() {
    if !data_available() {
        println!("[SKIP] espeak-ng data not found at {ESPEAK_DATA_DIR}");
        return;
    }
    let (pcm, _) = text_to_pcm("de", "guten Tag").expect("German ok");
    assert!(!pcm.is_empty());
    assert!(has_audio(&pcm, 100), "German synthesis should produce audio");
}

// ---------------------------------------------------------------------------
// Synthesizer::synthesize_codes — unit path test
// ---------------------------------------------------------------------------

#[test]
fn synthesize_codes_via_translate_to_codes() {
    if !data_available() {
        println!("[SKIP] espeak-ng data not found at {ESPEAK_DATA_DIR}");
        return;
    }
    use std::path::Path;
    use espeak_ng::phoneme::PhonemeData;
    use espeak_ng::translate::Translator;

    let data_path = Path::new(ESPEAK_DATA_DIR);
    let mut phdata = PhonemeData::load(data_path).expect("load phoneme data");
    phdata.select_table_by_name("en").expect("select en table");

    let translator = Translator::new_default("en").expect("translator");
    let codes = translator.translate_to_codes("hello").expect("translate");
    assert!(!codes.is_empty(), "Should produce phoneme codes");

    // All real phoneme codes should be > 7 (not just stress markers)
    let real_phonemes: Vec<_> = codes.iter().filter(|c| c.code > 7 && !c.is_boundary).collect();
    assert!(!real_phonemes.is_empty(), "Should have real phonemes: {:?}", codes);

    let synth = Synthesizer::new(VoiceParams::default());
    let pcm = synth.synthesize_codes(&codes, &phdata).expect("synthesize");
    assert!(!pcm.is_empty(), "PCM should be non-empty");
    assert!(has_audio(&pcm, 100), "Should produce audible audio");
}

// ---------------------------------------------------------------------------
// Synthesizer — IPA path (existing cascade formant synthesizer)
// ---------------------------------------------------------------------------

#[test]
fn synthesize_ipa_path_still_works() {
    if !data_available() {
        println!("[SKIP] espeak-ng data not found at {ESPEAK_DATA_DIR}");
        return;
    }
    let ipa = text_to_ipa("en", "hello").expect("IPA ok");
    let synth = Synthesizer::new(VoiceParams::default());
    let pcm = synth.synthesize(&ipa).expect("IPA synthesis ok");
    assert!(!pcm.is_empty());
}

// ---------------------------------------------------------------------------
// Sample rate
// ---------------------------------------------------------------------------

#[test]
fn sample_rate_is_22050() {
    if !data_available() {
        println!("[SKIP]");
        return;
    }
    let (_, rate) = text_to_pcm("en", "test").expect("ok");
    assert_eq!(rate, 22050);
}

// ---------------------------------------------------------------------------
// Duration correlates with speech length
// ---------------------------------------------------------------------------

#[test]
fn spoken_duration_scales_with_speed() {
    if !data_available() {
        println!("[SKIP] espeak-ng data not found at {ESPEAK_DATA_DIR}");
        return;
    }
    use std::path::Path;
    use espeak_ng::phoneme::PhonemeData;
    use espeak_ng::translate::Translator;

    let data_path = Path::new(ESPEAK_DATA_DIR);
    let mut phdata = PhonemeData::load(data_path).expect("load");
    phdata.select_table_by_name("en").expect("select");

    let translator = Translator::new_default("en").expect("translator");
    let codes = translator.translate_to_codes("hello").expect("codes");

    let mut normal_voice = VoiceParams::default();
    normal_voice.speed_percent = 100;

    let mut fast_voice = VoiceParams::default();
    fast_voice.speed_percent = 200; // double speed

    let normal = Synthesizer::new(normal_voice).synthesize_codes(&codes, &phdata).unwrap();
    let fast   = Synthesizer::new(fast_voice).synthesize_codes(&codes, &phdata).unwrap();

    assert!(fast.len() < normal.len(),
        "Double speed should halve duration: normal={}, fast={}", normal.len(), fast.len());
}

// ---------------------------------------------------------------------------
// Voiced vs unvoiced phonemes have different energy characteristics
// ---------------------------------------------------------------------------

#[test]
fn vowel_has_higher_rms_than_silence() {
    if !data_available() {
        println!("[SKIP]");
        return;
    }
    use std::path::Path;
    use espeak_ng::phoneme::PhonemeData;
    use espeak_ng::translate::Translator;

    let data_path = Path::new(ESPEAK_DATA_DIR);
    let mut phdata = PhonemeData::load(data_path).expect("load");
    phdata.select_table_by_name("en").expect("select");

    let translator = Translator::new_default("en").expect("translator");
    // "ah" is a pure vowel, should have significant RMS
    let codes = translator.translate_to_codes("ah").expect("codes");

    let synth = Synthesizer::new(VoiceParams::default());
    let pcm = synth.synthesize_codes(&codes, &phdata).unwrap();

    let r = common::rms(&pcm);
    assert!(r > 100.0, "Vowel 'ah' should have RMS > 100, got {r:.1}");
}
