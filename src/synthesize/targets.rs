//! Formant-synthesis target parameters for IPA phonemes.
//!
//! Values are based on:
//! - Peterson & Barney (1952) — vowel formant data (male speaker averages)
//! - Klatt & Klatt (1990)     — formant synthesis parameters
//! - Fant (1960)              — consonant spectral data
//! - Stevens (1998)           — acoustic phonetics reference
//!
//! Each [`FormantTarget`] describes the acoustic goal state for one phoneme.
//! The synthesizer interpolates linearly between consecutive targets.

/// Synthesis parameters for one IPA phoneme.
#[derive(Debug, Clone, Copy)]
pub struct FormantTarget {
    /// First formant frequency (Hz)
    pub f1: f64,
    /// Second formant frequency (Hz)
    pub f2: f64,
    /// Third formant frequency (Hz)
    pub f3: f64,
    /// F1 bandwidth (Hz) – narrower = higher Q = more resonant
    pub bw1: f64,
    /// F2 bandwidth (Hz)
    pub bw2: f64,
    /// F3 bandwidth (Hz)
    pub bw3: f64,
    /// Canonical duration at normal speaking rate (ms)
    pub dur_ms: f64,
    /// Voiced fraction 0.0 – 1.0 (1.0 = fully voiced)
    pub voiced_frac: f64,
    /// Noise fraction 0.0 – 1.0 (1.0 = pure fricative noise)
    pub noise_frac: f64,
    /// Relative amplitude (1.0 = normal vowel level)
    pub amp: f64,
}

impl FormantTarget {
    /// Construct a new `FormantTarget` from its parameters (const-friendly).
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        f1: f64, f2: f64, f3: f64,
        bw1: f64, bw2: f64, bw3: f64,
        dur_ms: f64,
        voiced_frac: f64, noise_frac: f64, amp: f64,
    ) -> Self {
        FormantTarget { f1, f2, f3, bw1, bw2, bw3, dur_ms, voiced_frac, noise_frac, amp }
    }
}

/// Silence target (used for pauses and unknown phonemes).
pub const SILENCE: FormantTarget = FormantTarget::new(
    500.0, 1500.0, 2500.0,
    200.0, 300.0, 400.0,
    80.0, 0.0, 0.0, 0.0,
);

/// IPA pattern → FormantTarget table.
///
/// The slice is searched with **longest-prefix-first** matching so that
/// multi-character phonemes (digraphs, diphthongs, long vowels) win over their
/// single-character components.  Entries are ordered longest-to-shortest within
/// each phoneme family.
pub static IPA_TARGETS: &[(&str, FormantTarget)] = &[
    // ═══════════════════════════════════════════════════════════════════════
    // Affricates (must precede their component fricatives)
    // ═══════════════════════════════════════════════════════════════════════
    ("tʃ", FormantTarget::new(1800.0, 2000.0, 2800.0,  250.0, 350.0, 500.0,  100.0, 0.0,  0.8,  0.5)),
    ("dʒ", FormantTarget::new( 250.0,  900.0, 2200.0,  100.0, 200.0, 350.0,  100.0, 0.6,  0.6,  0.6)),

    // ═══════════════════════════════════════════════════════════════════════
    // Long vowels (must precede their short counterparts)
    // ═══════════════════════════════════════════════════════════════════════
    ("iː", FormantTarget::new( 270.0, 2290.0, 3010.0,   60.0,  90.0, 150.0,  130.0, 1.0,  0.0,  1.0)),
    ("ɑː", FormantTarget::new( 730.0, 1090.0, 2440.0,   80.0, 100.0, 280.0,  160.0, 1.0,  0.0,  1.0)),
    ("ɔː", FormantTarget::new( 570.0,  840.0, 2410.0,   80.0, 100.0, 280.0,  160.0, 1.0,  0.0,  1.0)),
    ("uː", FormantTarget::new( 300.0,  870.0, 2240.0,   80.0, 100.0, 280.0,  130.0, 1.0,  0.0,  1.0)),
    ("ɜː", FormantTarget::new( 490.0, 1350.0, 1700.0,   80.0, 100.0, 280.0,  160.0, 1.0,  0.0,  1.0)),

    // ═══════════════════════════════════════════════════════════════════════
    // Diphthongs
    // ═══════════════════════════════════════════════════════════════════════
    ("eɪ", FormantTarget::new( 530.0, 1840.0, 2480.0,   80.0, 100.0, 280.0,  140.0, 1.0,  0.0,  1.0)),
    ("aɪ", FormantTarget::new( 730.0, 1090.0, 2440.0,   80.0, 100.0, 280.0,  140.0, 1.0,  0.0,  1.0)),
    ("ɔɪ", FormantTarget::new( 570.0,  840.0, 2410.0,   80.0, 100.0, 280.0,  140.0, 1.0,  0.0,  1.0)),
    ("aʊ", FormantTarget::new( 730.0, 1090.0, 2440.0,   80.0, 100.0, 280.0,  140.0, 1.0,  0.0,  1.0)),
    ("əʊ", FormantTarget::new( 500.0, 1500.0, 2500.0,   80.0, 100.0, 280.0,  140.0, 1.0,  0.0,  1.0)),
    ("ɪə", FormantTarget::new( 390.0, 1990.0, 2550.0,   80.0, 100.0, 280.0,  140.0, 1.0,  0.0,  1.0)),
    ("eə", FormantTarget::new( 530.0, 1840.0, 2480.0,   80.0, 100.0, 280.0,  140.0, 1.0,  0.0,  1.0)),
    ("ʊə", FormantTarget::new( 440.0, 1020.0, 2240.0,   80.0, 100.0, 280.0,  140.0, 1.0,  0.0,  1.0)),

    // ═══════════════════════════════════════════════════════════════════════
    // Short / monophthong vowels
    // ═══════════════════════════════════════════════════════════════════════
    ("ɪ",  FormantTarget::new( 390.0, 1990.0, 2550.0,   80.0, 100.0, 280.0,   80.0, 1.0,  0.0,  1.0)),
    ("e",  FormantTarget::new( 530.0, 1840.0, 2480.0,   80.0, 100.0, 280.0,   80.0, 1.0,  0.0,  1.0)),
    ("æ",  FormantTarget::new( 660.0, 1720.0, 2410.0,   80.0, 100.0, 280.0,  120.0, 1.0,  0.0,  1.0)),
    ("ɒ",  FormantTarget::new( 570.0,  840.0, 2410.0,   80.0, 100.0, 280.0,   90.0, 1.0,  0.0,  1.0)),
    ("ʊ",  FormantTarget::new( 440.0, 1020.0, 2240.0,   80.0, 100.0, 280.0,   80.0, 1.0,  0.0,  1.0)),
    ("ʌ",  FormantTarget::new( 640.0, 1190.0, 2390.0,   80.0, 100.0, 280.0,   90.0, 1.0,  0.0,  1.0)),
    ("ə",  FormantTarget::new( 500.0, 1500.0, 2500.0,   80.0, 100.0, 280.0,   60.0, 1.0,  0.0,  0.7)),
    ("ɜ",  FormantTarget::new( 490.0, 1350.0, 1700.0,   80.0, 100.0, 280.0,  100.0, 1.0,  0.0,  1.0)),
    ("ɐ",  FormantTarget::new( 700.0, 1220.0, 2600.0,   80.0, 100.0, 280.0,   80.0, 1.0,  0.0,  0.9)),
    // plain ASCII vowels (fallback / non-IPA usage)
    ("i",  FormantTarget::new( 270.0, 2290.0, 3010.0,   60.0,  90.0, 150.0,   80.0, 1.0,  0.0,  1.0)),
    ("a",  FormantTarget::new( 700.0, 1220.0, 2600.0,   80.0, 100.0, 280.0,  100.0, 1.0,  0.0,  1.0)),
    ("o",  FormantTarget::new( 450.0,  750.0, 2400.0,   80.0, 100.0, 280.0,  100.0, 1.0,  0.0,  1.0)),
    ("u",  FormantTarget::new( 300.0,  870.0, 2240.0,   80.0, 100.0, 280.0,   80.0, 1.0,  0.0,  1.0)),

    // ═══════════════════════════════════════════════════════════════════════
    // Fricatives
    // ═══════════════════════════════════════════════════════════════════════
    ("f",  FormantTarget::new( 900.0, 1400.0, 2200.0,  300.0, 400.0, 600.0,  100.0, 0.0,  1.0,  0.4)),
    ("v",  FormantTarget::new( 900.0, 1400.0, 2200.0,  300.0, 400.0, 600.0,   80.0, 0.6,  0.7,  0.5)),
    ("θ",  FormantTarget::new(1400.0, 1800.0, 2400.0,  300.0, 400.0, 500.0,  100.0, 0.0,  1.0,  0.3)),
    ("ð",  FormantTarget::new( 800.0, 1500.0, 2300.0,  200.0, 300.0, 400.0,   80.0, 0.6,  0.6,  0.4)),
    ("s",  FormantTarget::new(1500.0, 2000.0, 3900.0,  300.0, 400.0, 500.0,  100.0, 0.0,  1.0,  0.5)),
    ("z",  FormantTarget::new(1500.0, 2000.0, 3900.0,  300.0, 400.0, 500.0,   80.0, 0.5,  0.8,  0.5)),
    ("ʃ",  FormantTarget::new(1200.0, 1800.0, 2800.0,  300.0, 400.0, 500.0,  100.0, 0.0,  1.0,  0.5)),
    ("ʒ",  FormantTarget::new(1200.0, 1800.0, 2800.0,  300.0, 400.0, 500.0,   80.0, 0.5,  0.8,  0.5)),
    ("h",  FormantTarget::new( 500.0, 1500.0, 2500.0,  150.0, 200.0, 300.0,   60.0, 0.0,  0.4,  0.3)),
    ("x",  FormantTarget::new( 600.0, 1200.0, 2000.0,  300.0, 400.0, 500.0,   80.0, 0.0,  0.9,  0.4)),
    ("ç",  FormantTarget::new(1000.0, 2000.0, 3000.0,  300.0, 400.0, 500.0,   80.0, 0.0,  0.9,  0.4)),
    ("ɣ",  FormantTarget::new( 400.0, 1000.0, 1800.0,  200.0, 300.0, 400.0,   80.0, 0.6,  0.7,  0.5)),

    // ═══════════════════════════════════════════════════════════════════════
    // Stops (plosives)
    // ═══════════════════════════════════════════════════════════════════════
    ("p",  FormantTarget::new( 200.0,  800.0, 2200.0,  200.0, 300.0, 500.0,   80.0, 0.0,  0.3,  0.2)),
    ("b",  FormantTarget::new( 200.0,  800.0, 2200.0,  200.0, 300.0, 500.0,   80.0, 0.8,  0.1,  0.3)),
    ("t",  FormantTarget::new( 500.0, 1700.0, 2600.0,  200.0, 300.0, 500.0,   80.0, 0.0,  0.3,  0.2)),
    ("d",  FormantTarget::new( 250.0, 1700.0, 2600.0,  100.0, 200.0, 400.0,   80.0, 0.8,  0.1,  0.3)),
    ("k",  FormantTarget::new( 500.0, 1500.0, 2500.0,  200.0, 300.0, 500.0,   80.0, 0.0,  0.3,  0.2)),
    ("ɡ",  FormantTarget::new( 250.0, 1500.0, 2500.0,  100.0, 200.0, 400.0,   80.0, 0.8,  0.1,  0.3)),
    ("g",  FormantTarget::new( 250.0, 1500.0, 2500.0,  100.0, 200.0, 400.0,   80.0, 0.8,  0.1,  0.3)),
    ("ʔ",  FormantTarget::new( 500.0, 1500.0, 2500.0,  200.0, 300.0, 400.0,   40.0, 0.0,  0.0,  0.0)),

    // ═══════════════════════════════════════════════════════════════════════
    // Nasals
    // ═══════════════════════════════════════════════════════════════════════
    ("m",  FormantTarget::new( 250.0, 1200.0, 2200.0,  100.0, 200.0, 400.0,   80.0, 1.0,  0.0,  0.3)),
    ("n",  FormantTarget::new( 280.0, 1600.0, 2200.0,  100.0, 200.0, 400.0,   80.0, 1.0,  0.0,  0.3)),
    ("ŋ",  FormantTarget::new( 300.0,  900.0, 2200.0,  100.0, 200.0, 400.0,   80.0, 1.0,  0.0,  0.3)),

    // ═══════════════════════════════════════════════════════════════════════
    // Liquids
    // ═══════════════════════════════════════════════════════════════════════
    ("l",  FormantTarget::new( 360.0, 1200.0, 2400.0,   80.0, 200.0, 300.0,   70.0, 1.0,  0.0,  0.6)),
    ("ɫ",  FormantTarget::new( 400.0,  900.0, 2400.0,   80.0, 200.0, 300.0,   70.0, 1.0,  0.0,  0.6)),
    ("r",  FormantTarget::new( 400.0, 1350.0, 1700.0,   80.0, 150.0, 200.0,   70.0, 1.0,  0.0,  0.6)),
    ("ɹ",  FormantTarget::new( 400.0, 1350.0, 1700.0,   80.0, 150.0, 200.0,   70.0, 1.0,  0.0,  0.6)),
    ("ɾ",  FormantTarget::new( 400.0, 1500.0, 2100.0,   80.0, 150.0, 250.0,   35.0, 1.0,  0.0,  0.5)),

    // ═══════════════════════════════════════════════════════════════════════
    // Approximants / glides
    // ═══════════════════════════════════════════════════════════════════════
    ("j",  FormantTarget::new( 300.0, 2100.0, 3000.0,   80.0, 100.0, 150.0,   60.0, 1.0,  0.0,  0.5)),
    ("w",  FormantTarget::new( 300.0,  610.0, 2200.0,   80.0, 100.0, 200.0,   60.0, 1.0,  0.0,  0.5)),

    // ═══════════════════════════════════════════════════════════════════════
    // Space / word boundary → very short silence
    // ═══════════════════════════════════════════════════════════════════════
    (" ",  FormantTarget::new( 500.0, 1500.0, 2500.0,  150.0, 200.0, 300.0,   60.0, 0.0,  0.0,  0.0)),
];

/// Return the longest-prefix-matching `FormantTarget` for the given IPA string.
///
/// Returns `(target, bytes_consumed)`, where `bytes_consumed` is the number of
/// **bytes** in the matched prefix (not codepoints, since IPA uses multi-byte
/// UTF-8).  Returns `None` if no entry matches.
pub fn match_ipa(ipa: &str) -> Option<(&'static FormantTarget, usize)> {
    // We iterate in table order (longest entries first within each family).
    // The table is already ordered longest-first globally so the first match wins.
    for (pattern, target) in IPA_TARGETS {
        if ipa.starts_with(pattern) {
            return Some((target, pattern.len()));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_long_vowel_wins_over_short() {
        // "iː" must match the long vowel entry, not just "i"
        let (tgt, bytes) = match_ipa("iːp").expect("should match");
        assert_eq!(bytes, "iː".len());
        assert!((tgt.dur_ms - 130.0).abs() < 1.0, "long /iː/ dur");
    }

    #[test]
    fn match_short_vowel_alone() {
        let (tgt, bytes) = match_ipa("ɪ").expect("should match ɪ");
        assert_eq!(bytes, "ɪ".len());
        assert!((tgt.f1 - 390.0).abs() < 1.0);
    }

    #[test]
    fn match_affricate() {
        let (_, bytes) = match_ipa("tʃ").expect("should match tʃ");
        assert_eq!(bytes, "tʃ".len());
    }

    #[test]
    fn no_match_returns_none() {
        assert!(match_ipa("☺").is_none());
    }

    #[test]
    fn silence_target_amp_zero() {
        assert_eq!(SILENCE.amp, 0.0);
    }

    #[test]
    fn all_targets_have_positive_duration() {
        for (ipa, tgt) in IPA_TARGETS {
            assert!(tgt.dur_ms > 0.0, "zero duration for {:?}", ipa);
        }
    }
}
