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

use espeak_ng::{text_to_ipa, text_to_pcm, Error};

// ---------------------------------------------------------------------------
// Helper: get C oracle output for a text+language pair.
// Returns None when espeak-ng binary is not on PATH.
// ---------------------------------------------------------------------------

fn oracle(lang: &str, text: &str) -> Option<String> {
    common::try_espeak_ipa(lang, text)
}

fn trim_silence(pcm: &[i16], threshold: i16) -> &[i16] {
    let first = pcm.iter().position(|&s| s.unsigned_abs() >= threshold as u16);
    let last = pcm.iter().rposition(|&s| s.unsigned_abs() >= threshold as u16);
    match (first, last) {
        (Some(start), Some(end)) if start <= end => &pcm[start..=end],
        _ => &[],
    }
}

fn envelope(pcm: &[i16], window: usize) -> Vec<i16> {
    if pcm.is_empty() || window == 0 {
        return Vec::new();
    }

    let mut out = Vec::with_capacity(pcm.len() / window + 1);
    for chunk in pcm.chunks(window) {
        let avg = chunk
            .iter()
            .map(|s| s.unsigned_abs() as u64)
            .sum::<u64>() / chunk.len() as u64;
        out.push(avg.min(i16::MAX as u64) as i16);
    }
    out
}

fn best_aligned_snr_db(reference: &[i16], test: &[i16], max_shift: usize) -> Option<(f64, isize, usize)> {
    if reference.is_empty() || test.is_empty() {
        return None;
    }

    let mut best: Option<(f64, isize, usize)> = None;

    for shift in -(max_shift as isize)..=(max_shift as isize) {
        let (ref_start, test_start) = if shift >= 0 {
            (shift as usize, 0usize)
        } else {
            (0usize, (-shift) as usize)
        };

        if ref_start >= reference.len() || test_start >= test.len() {
            continue;
        }

        let overlap = (reference.len() - ref_start).min(test.len() - test_start);
        if overlap < 16 {
            continue;
        }

        let ref_slice = &reference[ref_start..ref_start + overlap];
        let test_slice = &test[test_start..test_start + overlap];

        let mut dot_rt = 0.0f64;
        let mut dot_tt = 0.0f64;
        let mut signal_power = 0.0f64;
        for (&r, &t) in ref_slice.iter().zip(test_slice.iter()) {
            let rf = r as f64;
            let tf = t as f64;
            dot_rt += rf * tf;
            dot_tt += tf * tf;
            signal_power += rf * rf;
        }
        if dot_tt <= 1e-12 || signal_power <= 1e-12 {
            continue;
        }

        // Best scalar gain for test_slice in least-squares sense.
        let gain = dot_rt / dot_tt;

        let mut noise_power = 0.0f64;
        for (&r, &t) in ref_slice.iter().zip(test_slice.iter()) {
            let e = r as f64 - gain * t as f64;
            noise_power += e * e;
        }

        let snr = if noise_power <= 1e-12 {
            f64::INFINITY
        } else {
            10.0 * (signal_power / noise_power).log10()
        };

        match best {
            Some((best_snr, _, _)) if snr <= best_snr => {}
            _ => best = Some((snr, shift, overlap)),
        }
    }

    best
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

/// Strict audio parity check against C espeak-ng for a representative utterance.
///
/// We trim leading/trailing silence and search a small time-shift window to
/// account for implementation-level frame alignment differences.
///
/// Pass criterion: best aligned envelope-SNR >= 16 dB.
#[test]
fn en_hello_audio_matches_oracle_snr() {
    let text = "hello";
    let Some(wav_bytes) = common::try_espeak_wav("en", text) else {
        eprintln!("[SKIP] espeak-ng not on PATH");
        return;
    };

    let c_pcm = common::wav_to_pcm(&wav_bytes);
    let (rust_pcm, rate) = text_to_pcm("en", text).expect("Rust synthesis should succeed");
    assert_eq!(rate, 22050, "expected 22050 Hz sample rate");

    let c_trim = trim_silence(&c_pcm, 20);
    let rust_trim = trim_silence(&rust_pcm, 20);

    assert!(!c_trim.is_empty(), "C oracle audio should not be silent");
    assert!(!rust_trim.is_empty(), "Rust audio should not be silent");

    let duration_ratio = rust_trim.len() as f64 / c_trim.len() as f64;
    assert!(
        (0.90..=1.10).contains(&duration_ratio),
        "duration mismatch too large: Rust={} C={} ratio={:.3}",
        rust_trim.len(),
        c_trim.len(),
        duration_ratio
    );

    let c_rms = common::rms(c_trim);
    let rust_rms = common::rms(rust_trim);
    let rms_ratio = rust_rms / c_rms.max(1e-9);
    assert!(
        (0.60..=1.67).contains(&rms_ratio),
        "RMS mismatch too large: Rust={:.1} C={:.1} ratio={:.3}",
        rust_rms,
        c_rms,
        rms_ratio
    );

    // Compare amplitude envelopes (10 ms windows), then align within +/- 12 windows.
    // This is robust to phase differences while still penalizing timing/amplitude drift.
    let c_env = envelope(c_trim, 220);
    let rust_env = envelope(rust_trim, 220);

    let (snr, shift, overlap) = best_aligned_snr_db(&c_env, &rust_env, 12)
        .expect("could not compute aligned SNR");

    eprintln!(
        "[PARITY] en/{text} envelope-SNR={:.2} dB shift={} windows overlap={}",
        snr,
        shift,
        overlap
    );

    assert!(
        snr >= 16.0,
        "audio parity below threshold: envelope-SNR={:.2} dB (shift={} overlap={})",
        snr,
        shift,
        overlap
    );
}
