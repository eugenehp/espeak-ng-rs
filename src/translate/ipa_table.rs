//! IPA string lookup for espeak-ng phoneme codes.
//!
//! Maps internal phoneme codes to their Unicode IPA representations.
//! Language-specific overrides (e.g. French ʁ, English schwa) are handled
//! here alongside the general mnemonic-based lookup.
//
// Derived from querying:
//   espeak-ng -v en -q --ipa "[[<mnemonic>]]"
// for every phoneme code in the English phoneme table.
//
// For phoneme codes where the phondata bytecode has an explicit IPA string
// (e.g. oU → əʊ), that string is used.  For others, the C fallback
// `ipa1[char - 0x20]` (Kirshenbaum-to-IPA mapping) produces the correct
// result.
//
// Special / control codes:
//   0              = string terminator (handled by caller)
//   1              = unknown control
//   2  STRESS_U    = unstressed marker  (output: nothing / ˌ before vowel)
//   3  STRESS_D    = diminished
//   4  STRESS_2    = secondary stress   (output: ˌ before next vowel)
//   5  STRESS_3    = secondary2
//   6  STRESS_P    = primary stress     (output: ˈ before next vowel)
//   7  STRESS_P2   = priority primary   (output: ˈ before next vowel)
//   8  STRESS_PREV = stress-prev
//   9  PAUSE       = long pause         (output: space)
//   10 PAUSE_SHORT = short pause
//   11 PAUSE_NOLINK
//   12 LENGTHEN    = length mark (ː)    (appended to previous phoneme)
//   13 SCHWA       = mnem '@' — same as code 13 vowel
//   14 SCHWA_SHORT = mnem '@-'
//   15 END_WORD    = mnem '||' — word boundary (output: space)
//   16..26         = various
//   27 PAUSE_CLAUSE
//   28..33         = vowel type marks

/// The Kirshenbaum-to-IPA array from dictionary.c, indexed by `char - 0x20`.
/// Entries are Unicode codepoints (u32).
pub const IPA1: [u32; 96] = [
    0x20,  0x21,  0x22,  0x2b0, 0x24,  0x25,  0x0e6, 0x2c8, // 0x20-0x27
    0x28,  0x29,  0x27e, 0x2b,  0x2cc, 0x2d,  0x2e,  0x2f,  // 0x28-0x2f
    0x252, 0x31,  0x32,  0x25c, 0x34,  0x35,  0x36,  0x37,  // 0x30-0x37
    0x275, 0x39,  0x2d0, 0x2b2, 0x3c,  0x3d,  0x3e,  0x294, // 0x38-0x3f
    0x259, 0x251, 0x3b2, 0xe7,  0xf0,  0x25b, 0x46,  0x262, // 0x40-0x47
    0x127, 0x26a, 0x25f, 0x4b,  0x26b, 0x271, 0x14b, 0x254, // 0x48-0x4f
    0x3a6, 0x263, 0x280, 0x283, 0x3b8, 0x28a, 0x28c, 0x153, // 0x50-0x57
    0x3c7, 0xf8,  0x292, 0x32a, 0x5c,  0x5d,  0x5e,  0x5f,  // 0x58-0x5f
    0x60,  0x61,  0x62,  0x63,  0x64,  0x65,  0x66,  0x261, // 0x60-0x67
    0x68,  0x69,  0x6a,  0x6b,  0x6c,  0x6d,  0x6e,  0x6f,  // 0x68-0x6f
    0x70,  0x71,  0x72,  0x73,  0x74,  0x75,  0x76,  0x77,  // 0x70-0x77
    0x78,  0x79,  0x7a,  0x7b,  0x7c,  0x7d,  0x303, 0x7f,  // 0x78-0x7f
];

/// Convert an ASCII character (0x20..=0x7f) using the ipa1 table.
/// Returns the Unicode codepoint.
pub fn ipa1_char(c: u8) -> u32 {
    if c >= 0x20 && c < 0x80 {
        IPA1[(c - 0x20) as usize]
    } else {
        c as u32
    }
}

/// Encode a Unicode codepoint as UTF-8 bytes (up to 4 bytes).
pub fn encode_utf8(cp: u32, buf: &mut Vec<u8>) {
    if cp == 0 { return; }
    if cp < 0x80 {
        buf.push(cp as u8);
    } else if cp < 0x800 {
        buf.push(0xc0 | (cp >> 6) as u8);
        buf.push(0x80 | (cp & 0x3f) as u8);
    } else if cp < 0x10000 {
        buf.push(0xe0 | (cp >> 12) as u8);
        buf.push(0x80 | ((cp >> 6) & 0x3f) as u8);
        buf.push(0x80 | (cp & 0x3f) as u8);
    } else {
        buf.push(0xf0 | (cp >> 18) as u8);
        buf.push(0x80 | ((cp >> 12) & 0x3f) as u8);
        buf.push(0x80 | ((cp >> 6) & 0x3f) as u8);
        buf.push(0x80 | (cp & 0x3f) as u8);
    }
}

/// Apply the IPA1 (Kirshenbaum-to-IPA) mapping to a phoneme mnemonic string.
///
/// Mirrors the `use_ipa` fallback path in `WritePhMnemonic`:
/// - iterates mnemonic bytes
/// - skips `/` and everything after (variant indicator)
/// - skips `#` for vowels (subscript-h only for consonants)
/// - skips digit characters after the first character
/// - maps each remaining character through ipa1[]
pub fn mnemonic_to_ipa(mnemonic: u32, is_vowel: bool) -> String {
    // Mnemonic is stored as a packed little-endian u32: bytes [b0, b1, b2, b3]
    // where b0 is the first character, 0x00 terminates.
    let mut out = Vec::new();
    let mut first = true;
    let mut mnem = mnemonic;
    loop {
        let c = (mnem & 0xff) as u8;
        mnem >>= 8;
        if c == 0 { break; }
        if c == b'/' { break; } // variant indicator
        if c == b'#' && is_vowel { break; } // subscript-h for consonants only
        if !first && c.is_ascii_digit() {
            // ignore digits after first char (they encode variants, not IPA)
            continue;
        }
        encode_utf8(ipa1_char(c), &mut out);
        first = false;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// IPA string overrides for English phoneme codes where the phondata
/// `i_IPA_NAME` bytecode gives a different result from the ipa1 fallback.
///
/// Each entry: (code, ipa_string).  All other codes fall back to ipa1.
///
/// Derived by querying:
///   `espeak-ng -v en -q --ipa "[[<mnemonic>]]"`
/// for all 165 active English phoneme codes.
pub static EN_IPA_OVERRIDES: &[(u8, &str)] = &[
    // Syllabic consonants
    (41, "m\u{0329}"),   // m- → m̩
    (42, "n\u{0329}"),   // n- → n̩
    (43, "\u{014b}\u{0329}"), // N- → ŋ̩
    (45, "l\u{0329}"),   // l- → l̩
    // Diphthongs and vowels needing override (ipa1 gives wrong result)
    (111, "\u{0259}"),   // 3  → ə  (ipa1['3'] = ɜ, but '3' alone = ə in EN)
    (118, "\u{0259}l"),  // @L → əl (ipa1 gives əɫ)
    (129, "\u{0252}"),   // 0  → ɒ  (ipa1['0'] = ɒ ✓, but digit-first case)
    (130, "\u{0252}"),   // 0# → ɒ
    (131, "\u{0252}"),   // 02 → ɒ
    (132, "\u{0252}"),   // O2 → ɒ  (ipa1['O'] = ɔ, wrong)
    (144, "\u{0259}\u{028a}"), // oU → əʊ (ipa1 gives oʊ)
    (145, "\u{0259}\u{028a}"), // oU# → əʊ
    // Rhotic vowels
    (156, "\u{0259}\u{0279}"), // IR  → əɹ
    (157, "\u{028c}\u{0279}"), // VR  → ʌɹ
];

/// Look up the IPA string for a phoneme code.
///
/// `mnemonic` is the packed u32 mnemonic from PhonemeTab.
/// `is_vowel` is true for type == phVOWEL (2).
///
/// First checks `EN_IPA_OVERRIDES`, then falls back to `mnemonic_to_ipa`.
pub fn phoneme_ipa(code: u8, mnemonic: u32, is_vowel: bool) -> String {
    phoneme_ipa_lang(code, mnemonic, is_vowel, true)
}

/// IPA lookup with optional EN_IPA_OVERRIDES.
/// `use_en_overrides` should be true only for English.
pub fn phoneme_ipa_lang(code: u8, mnemonic: u32, is_vowel: bool, use_en_overrides: bool) -> String {
    if use_en_overrides {
        for &(oc, ipa) in EN_IPA_OVERRIDES {
            if oc == code {
                return ipa.to_string();
            }
        }
    }
    mnemonic_to_ipa(mnemonic, is_vowel)
}

// ─────────────────────────────────────────────────────────────────────────────
// IPA rendering from raw phoneme byte streams
// ─────────────────────────────────────────────────────────────────────────────

/// The primary-stress IPA marker: ˈ (U+02C8)
pub const IPA_STRESS_PRIMARY:   &str = "\u{02c8}";
/// The secondary-stress IPA marker: ˌ (U+02CC)
pub const IPA_STRESS_SECONDARY: &str = "\u{02cc}";
/// The length mark: ː (U+02D0)
pub const IPA_LENGTH_MARK: &str = "\u{02d0}";

/// Stress marker accumulated while walking a phoneme byte stream.
///
/// Set when a stress-code phoneme (e.g. `PHON_STRESS_P`) is encountered,
/// then emitted before the next vowel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingStress {
    /// No pending stress marker.
    None,
    /// Primary stress `ˈ` (U+02C8) pending.
    Primary,
    /// Secondary stress `ˌ` (U+02CC) pending.
    Secondary,
}

// ── Phoneme-type and control-code constants ──────────────────────────────────
// These re-export a subset of the constants from `phoneme::mod` for use within
// the IPA rendering path.  They mirror the C defines in `phoneme.h`.
#[allow(missing_docs)]
/// Phoneme type: stress marker (`phSTRESS`).
pub const PH_STRESS: u8 = 1;
#[allow(missing_docs)]
/// Phoneme type: vowel (`phVOWEL`).
pub const PH_VOWEL:  u8 = 2;
#[allow(missing_docs)]
/// Phoneme type: pause (`phPAUSE`).
pub const PH_PAUSE:  u8 = 0;

/// Unstressed marker code (`%`).
pub const PHON_STRESS_U:     u8 = 2;
/// Stress-down marker code (`%%`).
pub const PHON_STRESS_D:     u8 = 3;
/// Secondary stress code (`,`).
pub const PHON_STRESS_2:     u8 = 4;
/// Tertiary stress code.
pub const PHON_STRESS_3:     u8 = 5;
/// Primary stress code (`ˈ`).
pub const PHON_STRESS_P:     u8 = 6;
/// Priority-primary stress code (`ˈˈ`).
pub const PHON_STRESS_P2:    u8 = 7;
/// Revert-to-previous-stress code.
pub const PHON_STRESS_PREV:  u8 = 8;
/// Pause code.
pub const PHON_PAUSE:        u8 = 9;
/// Short pause code.
pub const PHON_PAUSE_SHORT:  u8 = 10;
/// No-link pause code.
pub const PHON_PAUSE_NOLINK: u8 = 11;
/// Length mark code (`ː`).
pub const PHON_LENGTHEN:     u8 = 12;
/// Schwa code (`ə`).
pub const PHON_SCHWA:        u8 = 13;
/// Short schwa code.
pub const PHON_SCHWA_SHORT:  u8 = 14;
/// End-of-word boundary code (`||`).
pub const PHON_END_WORD:     u8 = 15;
/// Tonic stress code.
pub const PHON_STRESS_TONIC: u8 = 26;
/// Clause-boundary pause code.
pub const PHON_PAUSE_CLAUSE: u8 = 27;

/// Is this phoneme code a stress-level marker?
#[inline]
pub fn is_stress_code(code: u8) -> bool {
    matches!(code, PHON_STRESS_U | PHON_STRESS_D | PHON_STRESS_2 | PHON_STRESS_3
                 | PHON_STRESS_P | PHON_STRESS_P2 | PHON_STRESS_PREV | PHON_STRESS_TONIC)
}

/// Is this a pause/boundary code?
#[inline]
pub fn is_pause_code(code: u8) -> bool {
    // PHON_PAUSE=9, PHON_PAUSE_SHORT=10, PHON_PAUSE_NOLINK=11,
    // PHON_END_WORD=15, plus miscellaneous control codes 17,21-24,27
    matches!(code, 9 | 10 | 11 | 15 | 17 | 21 | 22 | 23 | 24 | 27)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipa1_spot_checks() {
        // '@' → ə (0x259)
        assert_eq!(ipa1_char(b'@'), 0x259);
        // ':' → ː (0x2d0)
        assert_eq!(ipa1_char(b':'), 0x2d0);
        // 'D' → ð (0xf0)
        assert_eq!(ipa1_char(b'D'), 0xf0);
        // 'T' → θ (0x3b8)
        assert_eq!(ipa1_char(b'T'), 0x3b8);
        // 'N' → ŋ (0x14b)
        assert_eq!(ipa1_char(b'N'), 0x14b);
        // 'S' → ʃ (0x283)
        assert_eq!(ipa1_char(b'S'), 0x283);
        // 'Z' → ʒ (0x292)
        assert_eq!(ipa1_char(b'Z'), 0x292);
        // 'g' → ɡ (0x261)
        assert_eq!(ipa1_char(b'g'), 0x261);
        // 'V' → ʌ (0x28c)
        assert_eq!(ipa1_char(b'V'), 0x28c);
        // 'I' → ɪ (0x26a)
        assert_eq!(ipa1_char(b'I'), 0x26a);
        // 'U' → ʊ (0x28a)
        assert_eq!(ipa1_char(b'U'), 0x28a);
        // '3' → ɜ (0x25c)
        assert_eq!(ipa1_char(b'3'), 0x25c);
    }

    #[test]
    fn mnemonic_ipa_simple() {
        // '@' packed as u32 (LE) = 0x40 in low byte
        let at_mnem: u32 = b'@' as u32;
        assert_eq!(mnemonic_to_ipa(at_mnem, true), "ə");
    }

    #[test]
    fn mnemonic_ipa_colon() {
        // '3:' packed: 0x33 | (0x3a << 8)
        let mnem: u32 = b'3' as u32 | ((b':' as u32) << 8);
        assert_eq!(mnemonic_to_ipa(mnem, true), "ɜː");
    }

    #[test]
    fn mnemonic_ipa_digit_skipped() {
        // 'I2' → 'I' only (digit after first char is skipped)
        let mnem: u32 = b'I' as u32 | ((b'2' as u32) << 8);
        assert_eq!(mnemonic_to_ipa(mnem, true), "ɪ");
    }

    #[test]
    fn mnemonic_ipa_hash_vowel() {
        // '0#' for a vowel → '0' only (# stops iteration for vowels)
        let mnem: u32 = b'0' as u32 | ((b'#' as u32) << 8);
        assert_eq!(mnemonic_to_ipa(mnem, true), "ɒ");
    }

    #[test]
    #[allow(non_snake_case)]
    fn override_oU() {
        // code 144 (oU) should give əʊ, not oʊ
        let mnem: u32 = b'o' as u32 | ((b'U' as u32) << 8);
        let ipa = phoneme_ipa(144, mnem, true);
        assert_eq!(ipa, "əʊ");
    }

    #[test]
    #[allow(non_snake_case)]
    fn override_O2() {
        // code 132 (O2) should give ɒ, not ɔ
        let mnem: u32 = b'O' as u32 | ((b'2' as u32) << 8);
        let ipa = phoneme_ipa(132, mnem, true);
        assert_eq!(ipa, "ɒ");
    }

    #[test]
    fn encode_utf8_ascii() {
        let mut v = Vec::new();
        encode_utf8(b'h' as u32, &mut v);
        assert_eq!(v, b"h");
    }

    #[test]
    fn encode_utf8_2byte() {
        let mut v = Vec::new();
        encode_utf8(0x259, &mut v); // ə
        assert_eq!(String::from_utf8(v).unwrap(), "ə");
    }
}
