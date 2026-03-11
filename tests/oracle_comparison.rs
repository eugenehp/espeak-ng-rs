// tests/oracle_comparison.rs
//
// Oracle comparison tests: compare Rust output against the C espeak-ng library.
//
// These tests use the `espeak-ng` binary as the oracle (no c-oracle FFI needed).
// They document expected behaviour and will start FAILING as soon as the Rust
// `translate` module is implemented, because they will receive real output
// instead of NotImplemented errors.
//
// Run all oracle tests (binary must be on PATH):
//   cargo test --test oracle_comparison
//
// Run with verbose output to see what the C oracle produces:
//   cargo test --test oracle_comparison -- --nocapture

mod common;

use espeak_ng::{text_to_ipa, Error};

// ---------------------------------------------------------------------------
// Helper: get C oracle output for a text+language pair.
// Returns None when espeak-ng binary is not on PATH.
// ---------------------------------------------------------------------------

fn oracle(lang: &str, text: &str) -> Option<String> {
    common::try_espeak_ipa(lang, text)
}

// ---------------------------------------------------------------------------
// Macro: assert_matches_oracle
//
// Three outcomes:
//   a) espeak-ng binary not found  → skip with a notice (not a failure)
//   b) Rust returns NotImplemented → print C oracle value as a target
//   c) Rust returns a string       → must exactly match C oracle
//
// This lets us write all comparison tests NOW.  They are skipped in
// environments without espeak-ng, and automatically start enforcing
// correctness as each module is implemented.
// ---------------------------------------------------------------------------

macro_rules! assert_matches_oracle {
    ($lang:expr, $text:expr) => {{
        let Some(c_result) = oracle($lang, $text) else {
            eprintln!(
                "[SKIP] espeak-ng not on PATH – skipping oracle test \
                 lang={:?} text={:?}",
                $lang, $text
            );
            return;
        };
        let rust_result = text_to_ipa($lang, $text);
        match rust_result {
            Ok(rust_ipa) => {
                assert_eq!(
                    rust_ipa, c_result,
                    "lang={:?} text={:?}\n  C:    {:?}\n  Rust: {:?}",
                    $lang, $text, c_result, rust_ipa
                );
            }
            Err(Error::NotImplemented(_)) => {
                eprintln!(
                    "[STUB] lang={:?} text={:?} → C oracle: {:?}",
                    $lang, $text, c_result
                );
            }
            Err(e) => {
                panic!(
                    "Unexpected error for lang={:?} text={:?}: {e}",
                    $lang, $text
                );
            }
        }
    }};
}

// ---------------------------------------------------------------------------
// English – basic words
// ---------------------------------------------------------------------------

#[test]
fn en_hello() {
    assert_matches_oracle!("en", "hello");
}

#[test]
fn en_world() {
    assert_matches_oracle!("en", "world");
}

#[test]
fn en_hello_world() {
    assert_matches_oracle!("en", "hello world");
}

#[test]
fn en_the() {
    assert_matches_oracle!("en", "the");
}

#[test]
fn en_a() {
    assert_matches_oracle!("en", "a");
}

// ---------------------------------------------------------------------------
// English – phonological edge cases
// ---------------------------------------------------------------------------

#[test]
fn en_silent_e() {
    // "make" vs "mac" – silent-e rule
    assert_matches_oracle!("en", "make");
    assert_matches_oracle!("en", "mac");
}

#[test]
fn en_gh_digraph() {
    assert_matches_oracle!("en", "through");
    assert_matches_oracle!("en", "though");
    assert_matches_oracle!("en", "thought");
    assert_matches_oracle!("en", "rough");
}

#[test]
fn en_silent_consonants() {
    assert_matches_oracle!("en", "knight");
    assert_matches_oracle!("en", "write");
    assert_matches_oracle!("en", "pneumonia");
}

#[test]
fn en_suffixes() {
    assert_matches_oracle!("en", "running");
    assert_matches_oracle!("en", "walked");
    assert_matches_oracle!("en", "faster");
    assert_matches_oracle!("en", "happily");
}

// ---------------------------------------------------------------------------
// English – numbers
// ---------------------------------------------------------------------------

#[test]
fn en_numbers_cardinal() {
    for (num, _desc) in [
        ("0", "zero"),
        ("1", "one"),
        ("10", "ten"),
        ("11", "eleven"),
        ("42", "forty-two"),
        ("100", "one hundred"),
        ("1000", "one thousand"),
        ("1900", "nineteen hundred"),
        ("1000000", "one million"),
    ] {
        assert_matches_oracle!("en", num);
    }
}

#[test]
fn en_numbers_with_decimal() {
    assert_matches_oracle!("en", "3.14");
    assert_matches_oracle!("en", "0.5");
}

// ---------------------------------------------------------------------------
// English – punctuation and sentence boundaries
// ---------------------------------------------------------------------------

#[test]
fn en_sentence_period() {
    assert_matches_oracle!("en", "Hello. Goodbye.");
}

#[test]
fn en_sentence_question() {
    assert_matches_oracle!("en", "How are you?");
}

#[test]
fn en_sentence_exclamation() {
    assert_matches_oracle!("en", "Stop!");
}

#[test]
fn en_comma() {
    assert_matches_oracle!("en", "yes, no, maybe");
}

// ---------------------------------------------------------------------------
// French
// ---------------------------------------------------------------------------

#[test]
fn fr_bonjour() {
    assert_matches_oracle!("fr", "bonjour");
}

#[test]
fn fr_nasal_vowels() {
    assert_matches_oracle!("fr", "bon");
    assert_matches_oracle!("fr", "blanc");
    assert_matches_oracle!("fr", "vin");
}

#[test]
fn fr_liaison() {
    // French liaison: "les amis" – the 's' should link to "amis"
    assert_matches_oracle!("fr", "les amis");
}

// ---------------------------------------------------------------------------
// German
// ---------------------------------------------------------------------------

#[test]
fn de_guten_tag() {
    assert_matches_oracle!("de", "guten Tag");
}

#[test]
fn de_umlauts() {
    assert_matches_oracle!("de", "über");
    assert_matches_oracle!("de", "schön");
    assert_matches_oracle!("de", "müde");
}

#[test]
fn de_ch_digraph() {
    assert_matches_oracle!("de", "Bach");
    assert_matches_oracle!("de", "ich");
}

// ---------------------------------------------------------------------------
// Spanish
// ---------------------------------------------------------------------------

#[test]
fn es_hola() {
    assert_matches_oracle!("es", "hola");
}

#[test]
fn es_ll_digraph() {
    assert_matches_oracle!("es", "llamar");
}

// ---------------------------------------------------------------------------
// Ukrainian
// ---------------------------------------------------------------------------

/// Basic Ukrainian words – full match against C espeak-ng.
#[test]
fn uk_basic_words() {
    assert_matches_oracle!("uk", "він");    // he → βˈiːn
    assert_matches_oracle!("uk", "або");   // or → abˈo
    assert_matches_oracle!("uk", "день");  // day → dˈɛɲ
    assert_matches_oracle!("uk", "мир");   // peace → mˈɪr
}

/// Ukrainian words with Ukrainian-specific letters (і ї є ґ).
#[test]
fn uk_specific_letters() {
    assert_matches_oracle!("uk", "привіт");   // hello
    assert_matches_oracle!("uk", "людина");   // person/human
    assert_matches_oracle!("uk", "ніщо");     // nothing
}

/// Ukrainian stress placement.
#[test]
fn uk_stress() {
    assert_matches_oracle!("uk", "вона");    // she → vowel stress
    assert_matches_oracle!("uk", "Слава");  // glory
}

// ---------------------------------------------------------------------------
// Multi-language stress test: all IPA outputs should be non-empty strings
// ---------------------------------------------------------------------------

#[test]
fn all_languages_produce_nonempty_oracle_output() {
    if !common::espeak_available() {
        eprintln!("[SKIP] espeak-ng not on PATH");
        return;
    }
    let cases = [
        ("en", "hello"),
        ("fr", "bonjour"),
        ("de", "hallo"),
        ("es", "hola"),
        ("it", "ciao"),
        ("pt", "olá"),
        ("ru", "привет"),
        ("zh", "你好"),
        ("ja", "こんにちは"),
        ("ar", "مرحبا"),
    ];
    for (lang, text) in cases {
        let ipa = oracle(lang, text)
            .unwrap_or_else(|| panic!("oracle failed for lang={lang:?}"));
        assert!(
            !ipa.is_empty(),
            "Oracle returned empty string for lang={lang:?} text={text:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Oracle CLI self-check: verify the oracle is consistent across calls
// ---------------------------------------------------------------------------

#[test]
fn oracle_is_deterministic() {
    if !common::espeak_available() {
        eprintln!("[SKIP] espeak-ng not on PATH");
        return;
    }
    let a = oracle("en", "the quick brown fox").unwrap();
    let b = oracle("en", "the quick brown fox").unwrap();
    assert_eq!(a, b, "oracle is not deterministic");
}

#[test]
fn oracle_different_languages_differ() {
    if !common::espeak_available() {
        eprintln!("[SKIP] espeak-ng not on PATH");
        return;
    }
    let en = oracle("en", "hello").unwrap();
    let fr = oracle("fr", "hello").unwrap();
    assert_ne!(en, fr,
        "same word in different languages should differ: en={en:?} fr={fr:?}");
}

// ---------------------------------------------------------------------------
// Synthesis comparison (stubs; will pass once synthesize module is done)
// ---------------------------------------------------------------------------

/// Assert that the Rust synthesizer produces a WAV that is "close enough"
/// to the C output, measured by signal-to-noise ratio.
///
/// Threshold: ≥ 40 dB SNR (imperceptible difference to human hearing).
///
/// This test is currently a stub – it only runs the C oracle side.
#[test]
fn en_hello_audio_oracle_baseline() {
    let Some(wav_bytes) = common::try_espeak_wav("en", "hello") else {
        eprintln!("[SKIP] espeak-ng not on PATH");
        return;
    };

    // Basic WAV sanity checks
    assert!(wav_bytes.len() > 44, "WAV must have more than just a header");
    assert_eq!(&wav_bytes[0..4], b"RIFF", "must start with RIFF magic");
    assert_eq!(&wav_bytes[8..12], b"WAVE", "must have WAVE format marker");

    let samples = common::wav_to_pcm(&wav_bytes);
    let rms_val = common::rms(&samples);

    // The audio must have some non-zero signal
    assert!(rms_val > 10.0, "RMS should be above background noise: {rms_val}");

    eprintln!(
        "[ORACLE] 'hello' in English: {} samples @ 22050 Hz, RMS = {:.1}",
        samples.len(),
        rms_val
    );
}
