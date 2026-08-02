#![cfg(feature = "bundled-data-en")]

use espeak_ng::{EspeakNg, TextToPhonemesOptions};

fn with_english_engine<T>(lang: &str, test: impl FnOnce(&EspeakNg) -> T) -> T {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let data_dir =
        std::env::temp_dir().join(format!("espeak-ng-phonemizer-{}_{}", std::process::id(), n));
    std::fs::create_dir_all(&data_dir).expect("create temporary data directory");
    espeak_ng::install_bundled_language(&data_dir, "en").expect("install bundled English data");
    let engine = EspeakNg::with_data_dir(lang, &data_dir).expect("create English engine");
    let result = test(&engine);
    std::fs::remove_dir_all(&data_dir).expect("remove temporary data directory");
    result
}

#[test]
fn phonemizer_keeps_trailing_comma() {
    with_english_engine("en-us", |engine| {
        let ipa = engine.text_to_phonemes_phonemizer("Hello world,").unwrap();
        assert!(ipa.ends_with(','), "expected trailing comma in {ipa:?}");
        assert!(!ipa.contains('\n'), "expected flattened clauses in {ipa:?}");
    });
}

#[test]
fn phonemizer_keeps_exclamation_mark() {
    with_english_engine("en-us", |engine| {
        let ipa = engine.text_to_phonemes_phonemizer("Hello!").unwrap();
        assert!(
            ipa.ends_with('!'),
            "expected trailing exclamation mark in {ipa:?}"
        );
    });
}

#[test]
fn phonemizer_keeps_kitten_phrase_comma() {
    with_english_engine("en-us", |engine| {
        let ipa = engine
            .text_to_phonemes_phonemizer("seen or unseen,")
            .unwrap();
        assert!(ipa.ends_with(','), "expected trailing comma in {ipa:?}");
    });
}

#[test]
fn default_options_match_existing_method() {
    with_english_engine("en", |engine| {
        let existing = engine.text_to_phonemes("Hello, world.").unwrap();
        let configured = engine
            .text_to_phonemes_with_options("Hello, world.", TextToPhonemesOptions::default())
            .unwrap();
        assert_eq!(configured, existing);
        assert!(!configured.contains(','));
        assert!(!configured.contains('.'));
    });
}

#[test]
fn en_us_and_en_remain_distinct_for_rhotic_word() {
    let en = with_english_engine("en", |engine| {
        engine.text_to_phonemes_phonemizer("car").unwrap()
    });
    let en_us = with_english_engine("en-us", |engine| {
        engine.text_to_phonemes_phonemizer("car").unwrap()
    });

    assert_ne!(en_us, en);
    assert!(
        en_us.contains('ɹ'),
        "expected rhotic en-us IPA in {en_us:?}"
    );
}
