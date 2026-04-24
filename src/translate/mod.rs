//! Text → phoneme code → IPA string translation pipeline.
//!
//! Rust port of key portions of:
//! - `translate.c` (1807 lines)
//! - `translateword.c` (1201 lines)
//! - `readclause.c` (1023 lines)
//! - `numbers.c` (1873 lines)
//! - `tr_languages.c` (1704 lines)
//!
//! # Pipeline
//! ```text
//! &str  (raw text)
//!   │  tokenize()           → Vec<Token>
//!   │  word_to_phonemes()   → per-word phoneme codes (dictionary + rules)
//!   │  set_word_stress()    → stress placement
//!   │  phonemes_to_ipa()    → IPA string
//!   ▼
//! String  (IPA)
//! ```
//!
//! # Main entry point
//! [`Translator::text_to_ipa`] handles the full pipeline.
//! [`Translator::translate_to_codes`] stops before IPA rendering and
//! returns raw [`PhonemeCode`] values for the synthesizer.

pub mod ipa_table;

use std::path::{Path, PathBuf};

/// Return the default espeak-ng data directory.
///
/// Resolution order:
/// 1. `ESPEAK_DATA_PATH` environment variable.
/// 2. A directory named `espeak-ng-data` next to the currently running
///    executable (useful when the crate is used as a standalone binary).
/// 3. `/usr/share/espeak-ng-data` (system installation).
pub fn default_data_dir() -> String {
    // 1. Explicit environment variable overrides everything.
    if let Ok(path) = std::env::var("ESPEAK_DATA_PATH") {
        return path;
    }

    // 2. espeak-ng-data/ relative to the binary.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let local = dir.join("espeak-ng-data");
            if local.join("en_dict").exists() {
                return local.to_string_lossy().into_owned();
            }
        }
    }

    // 3. espeak-ng-data/ relative to the current working directory.
    {
        let cwd_local = std::path::Path::new("espeak-ng-data");
        if cwd_local.join("en_dict").exists() {
            if let Ok(abs) = cwd_local.canonicalize() {
                return abs.to_string_lossy().into_owned();
            }
        }
    }

    // 4. System-wide installation.
    "/usr/share/espeak-ng-data".to_string()
}

use crate::error::{Error, Result};
use crate::phoneme::load::PhonemeData;
use crate::dictionary::file::Dictionary;
use crate::dictionary::lookup::{lookup, LookupCtx};
use crate::dictionary::rules::translate_rules_phdata;
use crate::dictionary::{SUFX_I, FLAG_SUFFIX_REMOVED};
use crate::dictionary::stress::{set_word_stress, promote_strend_stress, change_word_stress,
                               apply_word_final_devoicing, apply_alt_stress_upgrade, StressOpts};

use ipa_table::{
    phoneme_ipa_lang,
    IPA_STRESS_PRIMARY, IPA_STRESS_SECONDARY,
    PendingStress, PHON_STRESS_P, PHON_STRESS_P2, PHON_STRESS_TONIC,
    PHON_STRESS_2, PHON_STRESS_3,
    PHON_STRESS_U, PHON_STRESS_D, PHON_STRESS_PREV,
    is_pause_code,
};

// ---------------------------------------------------------------------------
// Clause type flags
// Mirrors CLAUSE_TYPE_XXX from translate.h
// ---------------------------------------------------------------------------

bitflags::bitflags! {
    /// Encodes punctuation pause length, intonation shape, and clause type
    /// in a single u32 – exactly as the C code packs them.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ClauseFlags: u32 {
        /// Pause duration field (bits 0–11, units of 10ms).
        const PAUSE_MASK           = 0x0000_0FFF;
        /// Intonation type field (bits 12–14).
        const INTONATION_MASK      = 0x0000_7000;
        /// Optional space after punctuation.
        const OPTIONAL_SPACE_AFTER = 0x0000_8000;
        /// Phrase type field (bits 16–19).
        const TYPE_MASK            = 0x000F_0000;
        /// Punctuation character can appear inside a word (Armenian).
        const PUNCT_IN_WORD        = 0x0010_0000;
        /// Speak the name of the punctuation character.
        const SPEAK_PUNCT_NAME     = 0x0020_0000;
        /// Dot after the last word.
        const DOT_AFTER_LAST_WORD  = 0x0040_0000;
        /// Multiply CLAUSE_PAUSE by 320ms instead of 10ms.
        const PAUSE_LONG           = 0x0080_0000;
    }
}

/// Intonation pattern for a clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intonation {
    /// Falling intonation (`.`).
    FullStop,
    /// Rising–falling intonation (`,`).
    Comma,
    /// Rising intonation (`?`).
    Question,
    /// Emphatic intonation (`!`).
    Exclamation,
    /// No intonation marker.
    None,
}

/// Phrase / sentence boundary type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClauseType {
    /// No boundary.
    None,
    /// End of input.
    Eof,
    /// Language/voice switch.
    VoiceChange,
    /// Clause boundary (comma-class punctuation).
    Clause,
    /// Sentence boundary (period-class punctuation).
    Sentence,
}

/// A clause read from the input text.
#[derive(Debug, Clone)]
pub struct Clause {
    /// The raw UTF-8 text of the clause.
    pub text: String,
    /// How the clause ends (intonation pattern).
    pub intonation: Intonation,
    /// What kind of boundary follows.
    pub clause_type: ClauseType,
    /// Pause after the clause in milliseconds.
    pub pause_ms: u32,
}

// ---------------------------------------------------------------------------
// Language options
// ---------------------------------------------------------------------------

/// Language-specific translation options.
#[derive(Debug, Clone)]
pub struct LangOptions {
    /// BCP-47 language tag, e.g. "en", "fr", "de"
    pub lang: String,
    /// Words per minute (default 175)
    pub rate: u32,
    /// Base pitch (0–100, default 50)
    pub pitch: u32,
    /// Word gap in units of 10ms
    pub word_gap: i32,
    /// Stress rule index (STRESSPOSN_XXX from translate.h)
    pub stress_rule: u8,
}

impl Default for LangOptions {
    fn default() -> Self {
        LangOptions {
            lang:        "en".to_string(),
            rate:        175,
            pitch:       50,
            word_gap:    0,
            stress_rule: 2, // STRESSPOSN_2R = penultimate
        }
    }
}

// ---------------------------------------------------------------------------
// CJK character detection
// ---------------------------------------------------------------------------

/// Returns `true` if `c` is a CJK ideographic character.
///
/// These characters should each form an individual word token,
/// matching the C espeak-ng behaviour for languages with `words 1`
/// (e.g. Chinese, Japanese Kanji, Korean Hanja).
fn is_cjk_ideograph(c: char) -> bool {
    let cp = c as u32;
    // CJK Unified Ideographs
    (0x4E00..=0x9FFF).contains(&cp)
    // CJK Unified Ideographs Extension A
    || (0x3400..=0x4DBF).contains(&cp)
    // CJK Unified Ideographs Extension B-H
    || (0x20000..=0x323AF).contains(&cp)
    // CJK Compatibility Ideographs
    || (0xF900..=0xFAFF).contains(&cp)
    // CJK Radicals / Kangxi
    || (0x2F00..=0x2FDF).contains(&cp)
}

// ---------------------------------------------------------------------------
// Token types for the simple tokenizer
// ---------------------------------------------------------------------------

/// One token produced by [`tokenize`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// A word (sequence of letters / digits / apostrophes).
    Word(String),
    /// One or more whitespace characters collapsed into a single separator.
    Space,
    /// Sentence/clause boundary punctuation: `.`, `,`, `!`, `?`, `;`, `:`.
    ClauseBoundary(char),
    /// Any other punctuation character.
    Punctuation(char),
}

/// Tokenize plain text into a sequence of words, spaces and punctuation.
///
/// This is a simplified version of `ReadClause()` from readclause.c.  It
/// handles plain ASCII / UTF-8 text without SSML.
pub fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            // Collapse runs of whitespace into a single Space token.
            while chars.peek().map(|c| c.is_whitespace()).unwrap_or(false) {
                chars.next();
            }
            tokens.push(Token::Space);
        } else if matches!(c, '.' | ',' | '!' | '?' | ';' | ':') {
            // Clause/sentence boundary punctuation
            // Absorb trailing whitespace after punctuation
            while chars.peek().map(|ch| ch.is_whitespace()).unwrap_or(false) {
                chars.next();
            }
            tokens.push(Token::ClauseBoundary(c));
        } else if c.is_ascii_digit() {
            // Accumulate a digit string (possibly with decimal point) as a Word.
            let mut word = String::new();
            word.push(c);
            let mut has_dot = false;
            while let Some(&next) = chars.peek() {
                if next.is_ascii_digit() {
                    word.push(next);
                    chars.next();
                } else if next == '.' && !has_dot {
                    // Peek ahead to see if followed by a digit
                    let mut lookahead = chars.clone();
                    lookahead.next(); // skip '.'
                    if lookahead.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                        has_dot = true;
                        word.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            tokens.push(Token::Word(word));
        } else if is_cjk_ideograph(c) {
            // CJK ideographic characters: each character is a separate word.
            tokens.push(Token::Word(c.to_string()));
            // Consume any additional CJK characters as individual words.
            while let Some(&next) = chars.peek() {
                if is_cjk_ideograph(next) {
                    tokens.push(Token::Word(next.to_string()));
                    chars.next();
                } else {
                    break;
                }
            }
        } else if c.is_alphabetic() || c == '\'' {
            // Accumulate a word (letters, apostrophes, hyphens within words).
            let mut word = String::new();
            word.push(c);
            while let Some(&next) = chars.peek() {
                if is_cjk_ideograph(next) {
                    // Stop word accumulation at CJK boundary.
                    break;
                } else if next.is_alphabetic() || next == '\'' {
                    word.push(next);
                    chars.next();
                } else if next == '-' {
                    // Accept hyphen only if followed by a letter (compound word).
                    // We peek one more character ahead via a clone.
                    let mut lookahead = chars.clone();
                    lookahead.next(); // skip '-'
                    if lookahead.peek().map(|c| c.is_alphabetic()).unwrap_or(false) {
                        word.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            tokens.push(Token::Word(word));
        } else {
            tokens.push(Token::Punctuation(c));
        }
    }

    tokens
}

// ---------------------------------------------------------------------------
// Letter-bits table for English
// ---------------------------------------------------------------------------

/// English letter-bits table (256 bytes, indexed by ASCII/Latin-1 byte).
///
/// Bit layout:
///   bit 0 = vowel (A/E/I/O/U and their variants)
///   bit 2 = consonant
///   bit 7 = vowel2 (stressable vowel)
///
/// This is a simplified version.  The C code builds this from the language
/// definition files (`tr_languages.c`).  We hard-code the basic Latin letters.
/// Build the English letter_bits table matching InitTranslator() in tr_languages.c
/// for Latin-script English (letter_bits_offset = 0).
///
/// Groups (bit positions):
///   0 = A  vowels (aeiou)
///   1 = B  hard consonants, excluding h,r,w  (bcdfgjklmnpqstvxz)
///   2 = C  all consonants                    (bcdfghjklmnpqrstvwxz)
///   3 = H  'soft' consonants                 (hlmnr)
///   4 = F  voiceless consonants              (cfhkpqstx)
///   5 = G  voiced                            (bdgjlmnrvwyz)
///   6 = Y  front vowels                      (eiy)
///   7 = vowels including y                   (aeiouy)
pub fn english_letter_bits() -> [u8; 256] {
    let mut bits = [0u8; 256];

    let set = |bits: &mut [u8; 256], group: u8, letters: &[u8]| {
        for &c in letters {
            bits[c as usize] |= 1 << group;
            // Also uppercase
            if c.is_ascii_lowercase() {
                bits[(c - 32) as usize] |= 1 << group;
            }
        }
    };

    set(&mut bits, 0, b"aeiou");
    set(&mut bits, 1, b"bcdfgjklmnpqstvxz");
    set(&mut bits, 2, b"bcdfghjklmnpqrstvwxz");
    set(&mut bits, 3, b"hlmnr");
    set(&mut bits, 4, b"cfhkpqstx");
    set(&mut bits, 5, b"bdgjlmnrvwyz");
    set(&mut bits, 6, b"eiy");
    set(&mut bits, 7, b"aeiouy");

    bits
}

// ---------------------------------------------------------------------------
// Phoneme-byte → IPA rendering
// ---------------------------------------------------------------------------

/// Render a sequence of raw phoneme codes into an IPA string.
///
/// `phdata` provides the mnemonic and type for each phoneme code.
/// Stress codes set a "pending stress" that is prepended before the next vowel.
///
/// This mirrors the `GetTranslatedPhonemeString()` rendering in dictionary.c,
/// simplified for direct use from the raw phoneme byte stream (no phoneme list).
pub fn phonemes_to_ipa(
    phoneme_bytes: &[u8],
    phdata: &PhonemeData,
    pending_stress_in: PendingStress,
    word_sep: bool,          // prepend a space before the first vowel?
) -> (String, PendingStress) {
    phonemes_to_ipa_lang(phoneme_bytes, phdata, pending_stress_in, word_sep, true)
}

/// Like [`phonemes_to_ipa`] but with an explicit `use_en_overrides` flag.
///
/// Set `use_en_overrides = false` for non-English languages to skip the
/// English-specific schwa / r rendering.
pub fn phonemes_to_ipa_lang(
    phoneme_bytes: &[u8],
    phdata: &PhonemeData,
    pending_stress_in: PendingStress,
    word_sep: bool,
    use_en_overrides: bool,
) -> (String, PendingStress) {
    phonemes_to_ipa_full(phoneme_bytes, phdata, pending_stress_in, word_sep, use_en_overrides, false)
}

/// Full phoneme-to-IPA renderer.
/// `suppress_word_final_liaison`: if true, liaison phonemes (mnemonic ending in
/// '2' or '3') at word-final position are suppressed (not rendered).
pub fn phonemes_to_ipa_full(
    phoneme_bytes: &[u8],
    phdata: &PhonemeData,
    pending_stress_in: PendingStress,
    word_sep: bool,
    use_en_overrides: bool,
    suppress_word_final_liaison: bool,
) -> (String, PendingStress) {
    let mut out = String::new();
    let mut stress = pending_stress_in;
    let mut need_space = word_sep;
    let mut prev_phcode: u8 = 0; // track previous real phoneme for d#/z# logic
    const PH_VOICED_FLAG: u32 = 1 << 4; // phFLAGBIT_VOICED

    for (idx, &code) in phoneme_bytes.iter().enumerate() {
        if code == 0 { break; }

        // ── Stress codes ─────────────────────────────────────────────────
        match code {
            PHON_STRESS_P | PHON_STRESS_P2 | PHON_STRESS_TONIC => {
                stress = PendingStress::Primary;
                continue;
            }
            PHON_STRESS_2 | PHON_STRESS_3 => {
                stress = PendingStress::Secondary;
                continue;
            }
            PHON_STRESS_U | PHON_STRESS_D | PHON_STRESS_PREV => {
                // Clear any pending stress for this word
                stress = PendingStress::None;
                continue;
            }
            _ => {}
        }

        // ── Pause / boundary codes ────────────────────────────────────────
        if is_pause_code(code) {
            // END_WORD (||, code 15) marks a word boundary within a single token.
            // This is used in number phoneme sequences to create spaces between
            // components (e.g., "forty-two" → "fˈɔːti tˈuː").
            // Other pause codes (9, 10, 11) are silently skipped.
            if code == 15 { // PHON_END_WORD
                need_space = true;
                // Reset stress so next word gets fresh stress
                stress = PendingStress::None;
            }
            continue;
        }

        // ── Real phoneme ─────────────────────────────────────────────────
        // Apply synthesis-stage ChangeIf stress resolution.
        // Phonemes like Russian `o` use `ChangeIfNotStressed(V)` which maps the
        // phoneme to a different code based on its stress level.  We resolve
        // this here so the IPA string lookup uses the acoustically correct code.
        let is_primary = stress == PendingStress::Primary;
        let resolved_code = phdata.resolve_stressed_phoneme(code, is_primary);
        let code = resolved_code; // shadow with resolved code

        if let Some(ph) = phdata.get(code) {
            let is_vowel = ph.typ == 2; // phVOWEL
            let is_stress_type = ph.typ == 1; // phSTRESS

            if is_stress_type {
                // Stress-type phoneme in phontab (e.g. code 6 has type=1)
                // This is a stress MARKER phoneme, not an acoustic phoneme.
                // Decode as in DecodePhonemes:
                //   if std_length <= 4 and program==0: use stress_chars[std_length]
                if ph.std_length <= 4 && ph.program == 0 {
                    match ph.std_length {
                        4 => { stress = PendingStress::Primary; }
                        2 | 3 => { stress = PendingStress::Secondary; }
                        _ => {}
                    }
                }
                continue;
            }

            // Suppress liaison phonemes at word-final position when requested.
            // A "liaison phoneme" has a mnemonic ending in '2' or '3' (e.g. n2, z2, t2).
            // They only surface before vowels; at word-end without a following vowel they
            // are silent. We detect "word-final" by checking if all remaining bytes are 0.
            if suppress_word_final_liaison {
                let mnemonic = ph.mnemonic;
                // Check second byte of mnemonic (little-endian u32: bytes [b0,b1,b2,b3])
                // If byte1 is '2' or '3' and byte2 is 0 (2-char mnemonic)
                // AND it's a consonant (not a vowel) — liaison phonemes are consonants
                let b1 = ((mnemonic >> 8) & 0xff) as u8;
                let b2 = ((mnemonic >> 16) & 0xff) as u8;
                let is_liaison = (b1 == b'2' || b1 == b'3') && b2 == 0 && !is_vowel;
                if is_liaison {
                    // Check if word-final (all remaining phonemes are 0 or stress codes)
                    let word_final = phoneme_bytes[idx+1..].iter()
                        .all(|&c| c == 0 || c <= 8 || c == 15);
                    if word_final {
                        continue; // suppress
                    }
                }
            }

            // Output space separator between words if needed
            if need_space {
                out.push(' ');
                need_space = false;
            }

            // Emit pending stress before vowels
            if is_vowel {
                match stress {
                    PendingStress::Primary   => { out.push_str(IPA_STRESS_PRIMARY); }
                    PendingStress::Secondary => { out.push_str(IPA_STRESS_SECONDARY); }
                    PendingStress::None      => {}
                }
                stress = PendingStress::None;
            }

            // Special handling for phonemes that change based on previous phoneme voice:
            // d# → 'd' if prev is voiced, else 't'
            // z# → 'z' if prev is voiced, else 's'
            // These are common in English past tense and plural suffixes.
            let b1 = ((ph.mnemonic >> 8) & 0xff) as u8;
            if b1 == b'#' {
                let b0 = (ph.mnemonic & 0xff) as u8;
                // Check if previous real phoneme is voiced
                let prev_voiced = if let Some(prev_ph) = phdata.get(prev_phcode) {
                    prev_ph.typ == 2 /* phVOWEL */ ||
                    prev_ph.typ == 3 /* phLIQUID */ ||
                    (prev_ph.phflags & PH_VOICED_FLAG) != 0
                } else { false };

                let ipa_char = if b0 == b'd' {
                    if prev_voiced { "d" } else { "t" }
                } else if b0 == b'z' {
                    if prev_voiced { "z" } else { "s" }
                } else {
                    // Other X# phonemes: fall through to normal rendering
                    ""
                };

                if !ipa_char.is_empty() {
                    out.push_str(ipa_char);
                    prev_phcode = code;
                    continue;
                }
            }

            // Look up IPA: try phonindex i_IPA_NAME first, then mnemonic fallback
            let ipa = if let Some(ipa_str) = phdata.phoneme_ipa_string(ph.program) {
                ipa_str
            } else {
                phoneme_ipa_lang(code, ph.mnemonic, is_vowel, use_en_overrides)
            };
            out.push_str(&ipa);
            prev_phcode = code;
        }
        // Unknown code → skip silently
    }

    (out, stress)
}

// ---------------------------------------------------------------------------
// Word-level translation
// ---------------------------------------------------------------------------

/// Result of translating a single word.
pub struct WordResult {
    /// Phoneme codes with stress markers.
    pub phonemes: Vec<u8>,
    /// Raw dictionary flags (0 if not found in dictionary).
    pub dict_flags: u32,
}

// ---------------------------------------------------------------------------
// Number-to-phonemes (English)
// ---------------------------------------------------------------------------

/// Look up a number word from the dictionary (e.g. "_0", "_1", "_0C", "_0M1").
fn lookup_num_phonemes(dict: &Dictionary, key: &str) -> Vec<u8> {
    let ctx = LookupCtx { lookup_symbol: true, ..Default::default() };
    if let Some(r) = lookup(dict, key, &ctx) {
        if !r.phonemes.is_empty() {
            return r.phonemes;
        }
    }
    Vec::new()
}

/// Convert a number value (0-999) to phonemes using English dict entries.
/// Mirrors C's LookupNum3 for values < 1000.
/// `prev_thousands`: non-zero if there were preceding thousands groups.
fn num3_phonemes(dict: &Dictionary, value: u32, suppress_null: bool, prev_thousands: bool) -> Vec<u8> {
    let hundreds = value / 100;
    let tensunits = value % 100;

    let mut buf1: Vec<u8> = Vec::new(); // hundreds part
    let mut suppress_null = suppress_null; // make mutable; mirrors C's local modification

    if hundreds > 0 {
        // Hundreds: lookup "_N" (number of hundreds) + "_0C" (hundred)
        // Check for exact hundreds with NUM_1900 pattern (not applicable here)
        let ph_hundreds = lookup_num_phonemes(dict, &format!("_{}", hundreds));
        let ph_100 = lookup_num_phonemes(dict, "_0C");

        // Combine: hundreds_digit + "hundred"
        let h_len = ph_hundreds.iter().position(|&b| b == 0).unwrap_or(ph_hundreds.len());
        let c_len = ph_100.iter().position(|&b| b == 0).unwrap_or(ph_100.len());
        buf1.extend_from_slice(&ph_hundreds[..h_len]);
        buf1.extend_from_slice(&ph_100[..c_len]);

        // NUM_HUNDRED_AND: English uses "and" between hundreds and tens (e.g. "one hundred and five")
        // For simplicity, skip the "and" for now.
        suppress_null = true; // mirrors C: if hundreds > 0, suppress trailing zero
    }

    // tensunits part
    let mut buf2: Vec<u8> = Vec::new();
    if tensunits != 0 || !suppress_null {
        if tensunits < 20 {
            // 0-19: direct lookup "_N"
            let ph = lookup_num_phonemes(dict, &format!("_{}", tensunits));
            let l = ph.iter().position(|&b| b == 0).unwrap_or(ph.len());
            buf2.extend_from_slice(&ph[..l]);
        } else {
            let tens = tensunits / 10;
            let units = tensunits % 10;
            // Tens: "_NX" (e.g. "_2X" = "twenty")
            let ph_tens = lookup_num_phonemes(dict, &format!("_{}X", tens));
            let t_len = ph_tens.iter().position(|&b| b == 0).unwrap_or(ph_tens.len());
            buf2.extend_from_slice(&ph_tens[..t_len]);
            if units != 0 {
                // Units: "_N" (e.g. "_2" = "two")
                let ph_units = lookup_num_phonemes(dict, &format!("_{}", units));
                let u_len = ph_units.iter().position(|&b| b == 0).unwrap_or(ph_units.len());
                buf2.extend_from_slice(&ph_units[..u_len]);
            }
        }
    }

    // Combine: buf1 + ph_hundred_and (empty) + phonEND_WORD + buf2
    // This mirrors: sprintf(ph_out, "%s%s%c%s", buf1, ph_hundred_and, phonEND_WORD, buf2)
    let _ = prev_thousands; // used for "and" insertion in C, skipped here
    let mut result = buf1;
    // Only output phonEND_WORD separator if buf1 is non-empty (to avoid leading || for pure tens/units)
    if !result.is_empty() {
        result.push(15); // phonEND_WORD
    }
    result.extend_from_slice(&buf2);
    result
}

/// Convert an English number string to raw phoneme bytes.
/// Handles integers and simple decimals (N.NN format).
/// Mirrors the logic of numbers.c TranslateNumber_1 for common cases.
fn number_to_phonemes(word: &str, dict: &Dictionary) -> Option<Vec<u8>> {
    // Only handle ASCII digits (and one decimal point for decimals)
    if word.is_empty() { return None; }

    let bytes = word.as_bytes();

    // Check for decimal point
    if let Some(dot_pos) = bytes.iter().position(|&b| b == b'.') {
        // Decimal number: split at '.'
        let int_part = &word[..dot_pos];
        let dec_part = &word[dot_pos+1..];
        if int_part.is_empty() || dec_part.is_empty() { return None; }
        if !int_part.bytes().all(|b| b.is_ascii_digit()) { return None; }
        if !dec_part.bytes().all(|b| b.is_ascii_digit()) { return None; }

        let mut result = Vec::new();

        // Integer part
        let int_ph = number_to_phonemes(int_part, dict)?;
        let int_len = int_ph.iter().position(|&b| b == 0).unwrap_or(int_ph.len());
        result.extend_from_slice(&int_ph[..int_len]);

        // "point"
        let ph_dpt = lookup_num_phonemes(dict, "_dpt");
        if ph_dpt.is_empty() {
            // Fallback: lookup "point"
        } else {
            let l = ph_dpt.iter().position(|&b| b == 0).unwrap_or(ph_dpt.len());
            result.push(15); // END_WORD before "point"
            result.extend_from_slice(&ph_dpt[..l]);
        }

        // Decimal digits: each digit spoken separately
        for &d in dec_part.as_bytes() {
            let digit = (d - b'0') as u32;
            let ph = lookup_num_phonemes(dict, &format!("_{}", digit));
            let l = ph.iter().position(|&b| b == 0).unwrap_or(ph.len());
            result.push(15); // END_WORD separator
            result.extend_from_slice(&ph[..l]);
        }

        result.push(15); // trailing END_WORD
        result.push(0);
        return Some(result);
    }

    // Integer
    if !bytes.iter().all(|b| b.is_ascii_digit()) { return None; }

    let value: u64 = word.parse().ok()?;
    let mut result = Vec::new();

    if value == 0 {
        let ph = lookup_num_phonemes(dict, "_0");
        let l = ph.iter().position(|&b| b == 0).unwrap_or(ph.len());
        result.extend_from_slice(&ph[..l]);
        result.push(15);
        result.push(0);
        return Some(result);
    }

    // Handle up to billions
    let millions = (value / 1_000_000) as u32;
    let thousands = ((value % 1_000_000) / 1_000) as u32;
    let remainder = (value % 1_000) as u32;

    // NUM_1900: for values in [1100..9999] that look like years (XY00 pattern)
    // where XY >= 11, treat as "XY hundred" not "X thousand Y hundred"
    let is_year_form = value >= 1100 && value <= 9999 && value % 100 == 0
        && value / 100 >= 11;
    // is_year_form_exact would handle non-round year forms like 1984 = "nineteen eighty-four"
    // not implemented yet
    let _ = value; // suppress potential unused value warning

    // Special: NUM_1900 year forms like 1900, 2400 (XY00 where XY >= 11)
    if is_year_form {
        let year_hundreds = (value / 100) as u32;
        // "nineteen" + "hundred" etc.
        let ph = num3_phonemes(dict, year_hundreds, false, false);
        let ph_len = ph.iter().position(|&b| b == 0).unwrap_or(ph.len());
        let ph_100 = lookup_num_phonemes(dict, "_0C");
        let c_len = ph_100.iter().position(|&b| b == 0).unwrap_or(ph_100.len());
        result.extend_from_slice(&ph[..ph_len]);
        result.extend_from_slice(&ph_100[..c_len]);
        result.push(15);
        result.push(0);
        return Some(result);
    }

    let mut prev_thousands = false;

    if millions > 0 {
        // "N million"
        let ph_m = num3_phonemes(dict, millions, false, false);
        let m_len = ph_m.iter().position(|&b| b == 0).unwrap_or(ph_m.len());
        result.extend_from_slice(&ph_m[..m_len]);
        // "million"
        let ph_mil = lookup_num_phonemes(dict, "_0M2");
        let mil_len = ph_mil.iter().position(|&b| b == 0).unwrap_or(ph_mil.len());
        result.push(15); // END_WORD between millions number and "million"
        result.extend_from_slice(&ph_mil[..mil_len]);
        prev_thousands = true;
    }

    if thousands > 0 {
        // "N thousand"
        if prev_thousands { result.push(15); } // word boundary
        let ph_t = num3_phonemes(dict, thousands, false, false);
        let t_len = ph_t.iter().position(|&b| b == 0).unwrap_or(ph_t.len());
        result.extend_from_slice(&ph_t[..t_len]);
        // "thousand"
        let ph_thou = lookup_num_phonemes(dict, "_0M1");
        let thou_len = ph_thou.iter().position(|&b| b == 0).unwrap_or(ph_thou.len());
        result.push(15); // END_WORD between number and "thousand"
        result.extend_from_slice(&ph_thou[..thou_len]);
        prev_thousands = true;
    }

    if remainder > 0 || !prev_thousands {
        if prev_thousands && remainder > 0 { result.push(15); }
        let ph_r = num3_phonemes(dict, remainder, false, prev_thousands);
        let r_len = ph_r.iter().position(|&b| b == 0).unwrap_or(ph_r.len());
        result.extend_from_slice(&ph_r[..r_len]);
    }

    result.push(15); // trailing END_WORD (as in C)
    result.push(0);
    Some(result)
}

/// Translate a single lowercase word to phoneme bytes (with stress markers).
///
/// Strategy (mirrors `TranslateWord` in translateword.c):
/// 1. Try dictionary lookup
/// 2. Fall back to translation rules
/// 3. Apply SetWordStress to place stress markers
///
/// Returns the raw phoneme byte sequence with stress markers inserted,
/// and the dictionary flags for post-processing (e.g. strend promotion).
pub fn word_to_phonemes(
    word: &str,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
) -> WordResult {
    let ctx = LookupCtx {
        lookup_symbol: true,
        ..Default::default()
    };

    // Try dictionary first
    let dict_result = lookup(dict, word, &ctx);

    // Extract flags from dict even if no phonemes (FLAGS-only entries).
    // Note: FLAGS-only entries have FLAG_FOUND_ATTRIBUTES (bit 30) but NOT FLAG_FOUND (bit 31).
    const FLAG_FOUND_ATTRIBUTES: u32 = 0x4000_0000;
    let dict_flags_from_lookup = dict_result.as_ref()
        .filter(|r| r.flags1.0 & (FLAG_FOUND_ATTRIBUTES | 0x8000_0000) != 0)
        .map(|r| r.flags1.0)
        .unwrap_or(0);

    if let Some(ref result) = dict_result {
        if result.flags1.found() && !result.phonemes.is_empty() {
            let dict_flags = result.flags1.0;
            let mut phonemes = result.phonemes.clone();
            // Apply stress placement
            set_word_stress(&mut phonemes, phdata, stress_opts, Some(dict_flags as u32), -1, 0);
            // Stressed-vowel upgrading (e.g. Turkish e→E, o→O under primary stress)
            if stress_opts.alt_stress_upgrade {
                apply_alt_stress_upgrade(&mut phonemes, phdata);
            }
            // Word-final devoicing (e.g. German Auslautverhärtung)
            if stress_opts.word_final_devoicing {
                apply_word_final_devoicing(&mut phonemes, phdata);
            }
            return WordResult { phonemes, dict_flags };
        }
    }

    // Number translation: detect digit-only words and convert to phonemes.
    // This mirrors C's TranslateNumber() in numbers.c, simplified for common cases.
    let is_numeric = !word.is_empty() && (word.bytes().all(|b| b.is_ascii_digit())
        || (word.contains('.') && word.bytes().all(|b| b.is_ascii_digit() || b == b'.')));
    if is_numeric {
        if let Some(num_phonemes) = number_to_phonemes(word, dict) {
            // Number phonemes contain END_WORD markers but no stress.
            // Stress is encoded in the individual number word dict entries.
            // SetWordStress handles the stress markers already embedded.
            let mut phonemes = num_phonemes;
            // Apply stress (should mostly be a no-op since dict entries have stress marked)
            set_word_stress(&mut phonemes, phdata, stress_opts, Some(0), -1, 0);
            return WordResult { phonemes, dict_flags: 0 };
        }
    }

    // Fall back to translation rules (potentially using dict flags from FLAGS-only entry)
    // Use the dictionary's language-specific letter_bits (Cyrillic, Arabic, etc.)
    let letter_bits = &*dict.letter_bits;
    let mut vowel_count = 0i32;
    let mut stressed_count = 0i32;

    // Prepare word buffer with leading space (for rule pre-context)
    let mut word_buf = Vec::with_capacity(word.len() + 2);
    word_buf.push(b' ');
    word_buf.extend_from_slice(word.as_bytes());
    word_buf.push(b' ');
    word_buf.push(0);

    let result = translate_rules_phdata(
        dict,
        &word_buf,
        1,   // word_start = 1 (skip leading space)
        0,   // word_flags
        0,   // dict_flags
        &letter_bits,
        0,   // dict_condition
        &mut vowel_count,
        &mut stressed_count,
        Some(phdata),
    );

    if !result.phonemes.is_empty() {
        // ── Suffix stripping: re-translate stem if a suffix rule fired ──
        // Only re-translate stem when SUFX_I requires stem reconstruction.
        // SUFX_I: stem had 'y' changed to 'i' (e.g., "happy" → "happi" + "ly").
        // The suffix phonemes don't include the link vowel from the stem's final 'y'.
        if std::env::var("DBG_SUFFIX").is_ok() {
            eprintln!("DBG '{}': end_type={:#010x} SUFX_I={} suffix_start={}",
                word, result.end_type, result.end_type & SUFX_I != 0, result.suffix_start);
        }
        let needs_stem_retranslation = result.end_type != 0
            && (result.end_type & SUFX_I) != 0
            && result.suffix_start > 1;
        let mut phonemes = if needs_stem_retranslation {
            // A suffix rule fired with SUFX_I. Reconstruct the stem by:
            // 1. Stripping suffix_length chars from the end of the word
            // 2. If SUFX_I and stem ends in 'i': change to 'y' (reverse y→i transformation)
            // This mirrors C's RemoveEnding() in dictionary.c:2905
            let suffix_len = (result.end_type & 0x7f) as usize; // low 7 bits = suffix letter count
            let word_bytes = word.as_bytes();
            let stem_end_pos = word_bytes.len().saturating_sub(suffix_len);
            let mut stem_word = word_bytes[..stem_end_pos].to_vec();

            if (result.end_type & SUFX_I) != 0 {
                // Restore 'y' if stem ends in 'i'
                if stem_word.last() == Some(&b'i') {
                    *stem_word.last_mut().unwrap() = b'y';
                }
            }

            // Re-translate the stem with FLAG_SUFFIX_REMOVED to mimic C's TranslateRules
            // call with wflags | FLAG_SUFFIX_REMOVED. This causes rules with RULE_NO_SUFFIX ('N')
            // to be disabled, allowing different rule selection (e.g., 'y' after consonant
            // gives 'I'(ɪ) instead of 'i' when the word originally had a suffix).
            if let Ok(stem_str) = std::str::from_utf8(&stem_word) {
                // Build stem word buffer with leading/trailing space
                let mut stem_buf = Vec::with_capacity(stem_str.len() + 3);
                stem_buf.push(b' ');
                stem_buf.extend_from_slice(stem_str.as_bytes());
                stem_buf.push(b' ');
                stem_buf.push(0);

                let mut stem_vc = 0i32;
                let mut stem_sc = 0i32;
                let stem_rules = translate_rules_phdata(
                    dict, &stem_buf, 1, FLAG_SUFFIX_REMOVED, 0, &letter_bits, 0,
                    &mut stem_vc, &mut stem_sc, Some(phdata));

                // Combine stem phonemes + stem's end_phonemes (which may contain
                // the final phoneme of the stem, e.g. 'I'(ɪ) from 'y' in "happy")
                let mut full_stem_ph = Vec::new();
                let stem_body = &stem_rules.phonemes;
                let body_len = stem_body.iter().position(|&b| b == 0).unwrap_or(stem_body.len());
                full_stem_ph.extend_from_slice(&stem_body[..body_len]);
                let stem_tail = &stem_rules.end_phonemes;
                for &b in stem_tail { if b == 0 { break; } full_stem_ph.push(b); }

                if full_stem_ph.is_empty() {
                    // Stem re-translation gave nothing; fall back to direct rules result
                    let mut combined = Vec::new();
                    let sp = &result.phonemes;
                    let sl = sp.iter().position(|&b| b == 0).unwrap_or(sp.len());
                    combined.extend_from_slice(&sp[..sl]);
                    for &b in &result.end_phonemes { if b == 0 { break; } combined.push(b); }
                    combined.push(0);
                    combined
                } else {
                    // Apply stress to full stem phonemes
                    full_stem_ph.push(0);
                    set_word_stress(&mut full_stem_ph, phdata, stress_opts, Some(0), -1, 0);

                    let stem_len = full_stem_ph.iter().position(|&b| b == 0).unwrap_or(full_stem_ph.len());
                    let mut combined = Vec::new();
                    combined.extend_from_slice(&full_stem_ph[..stem_len]);

                    // Append suffix phonemes from the word rule (result.end_phonemes)
                    for &b in &result.end_phonemes {
                        if b == 0 { break; }
                        // Skip stress markers from suffix (stem stress already applied)
                        if b == 6 || b == 7 || b == 4 || b == 5 { continue; }
                        combined.push(b);
                    }
                    combined.push(0);
                    combined
                }
            } else {
                // UTF-8 error: combine stem + suffix phonemes as fallback
                let mut combined = Vec::new();
                let stem_ph = &result.phonemes;
                let stem_len = stem_ph.iter().position(|&b| b == 0).unwrap_or(stem_ph.len());
                combined.extend_from_slice(&stem_ph[..stem_len]);
                for &b in &result.end_phonemes { if b == 0 { break; } combined.push(b); }
                combined.push(0);
                combined
            }
        } else {
            // No stem re-translation: concatenate stem + suffix phonemes directly
            let mut combined = Vec::new();
            let stem_ph = &result.phonemes;
            let stem_len = stem_ph.iter().position(|&b| b == 0).unwrap_or(stem_ph.len());
            combined.extend_from_slice(&stem_ph[..stem_len]);
            for &b in &result.end_phonemes { if b == 0 { break; } combined.push(b); }
            combined.push(0);
            combined
        };

        // Apply stress placement; use dict flags from FLAGS-only entry if available
        let flags_for_stress = if dict_flags_from_lookup != 0 {
            Some(dict_flags_from_lookup as u32)
        } else {
            Some(0)  // non-NULL mirrors C's behavior of passing non-NULL dictionary_flags
        };
        set_word_stress(&mut phonemes, phdata, stress_opts, flags_for_stress, -1, 0);
        // Stressed-vowel upgrading (e.g. Turkish e→E, o→O under primary stress)
        if stress_opts.alt_stress_upgrade {
            apply_alt_stress_upgrade(&mut phonemes, phdata);
        }
        // Word-final devoicing (e.g. German Auslautverhärtung)
        if stress_opts.word_final_devoicing {
            apply_word_final_devoicing(&mut phonemes, phdata);
        }
        return WordResult { phonemes, dict_flags: dict_flags_from_lookup };
    }

    // Could not translate (unknown word)
    WordResult { phonemes: Vec::new(), dict_flags: dict_flags_from_lookup }
}

// ---------------------------------------------------------------------------
// Translator
// ---------------------------------------------------------------------------

/// Top-level text translator.
///
/// Create with [`Translator::new_default`] for the most common case.
/// Use [`Translator::text_to_ipa`] or [`Translator::translate_to_codes`].
pub struct Translator {
    /// Language and speech-rate configuration.
    pub options: LangOptions,
    /// Resolved espeak-ng data directory.
    data_dir: PathBuf,
}

impl Translator {
    /// Create a new translator for the given language.
    ///
    /// `data_dir` is the path to the espeak-ng data directory.
    /// If `None`, defaults to `/usr/share/espeak-ng-data`.
    pub fn new(lang: &str, data_dir: Option<&Path>) -> Result<Self> {
        let dir = data_dir
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from(default_data_dir()));

        Ok(Translator {
            options: LangOptions {
                lang: lang.to_string(),
                ..Default::default()
            },
            data_dir: dir,
        })
    }

    /// Create with default data directory.
    pub fn new_default(lang: &str) -> Result<Self> {
        Self::new(lang, None)
    }

    /// Split `text` into clauses at sentence / phrase boundaries.
    ///
    /// Simplified version of `ReadClause()` in readclause.c.
    pub fn read_clauses(&self, text: &str) -> Result<Vec<Clause>> {
        // Simple split on sentence-ending punctuation.
        let mut clauses = Vec::new();
        let mut current = String::new();

        for c in text.chars() {
            match c {
                '.' | '!' | '?' => {
                    current.push(c);
                    let intonation = match c {
                        '!' => Intonation::Exclamation,
                        '?' => Intonation::Question,
                        _   => Intonation::FullStop,
                    };
                    let text_trim = current.trim().to_string();
                    if !text_trim.is_empty() {
                        clauses.push(Clause {
                            text: text_trim,
                            intonation,
                            clause_type: ClauseType::Sentence,
                            pause_ms: 400,
                        });
                    }
                    current = String::new();
                }
                ',' | ';' | ':' => {
                    current.push(c);
                    // Commas/semicolons continue within the same clause.
                }
                _ => { current.push(c); }
            }
        }

        // Remainder (no final punctuation)
        let text_trim = current.trim().to_string();
        if !text_trim.is_empty() {
            clauses.push(Clause {
                text: text_trim,
                intonation: Intonation::None,
                clause_type: ClauseType::Eof,
                pause_ms: 0,
            });
        }

        if clauses.is_empty() {
            clauses.push(Clause {
                text: text.trim().to_string(),
                intonation: Intonation::None,
                clause_type: ClauseType::Eof,
                pause_ms: 0,
            });
        }

        Ok(clauses)
    }

    /// High-level convenience: translate free text to an IPA string.
    ///
    /// Equivalent to running:
    ///   `espeak-ng -v <lang> -q --ipa <text>`
    ///
    /// This implementation handles plain text (no SSML) and performs:
    ///   1. Tokenization into words / punctuation
    ///   2. Dictionary lookup + rule-based translation per word
    ///   3. Phoneme code → IPA string rendering
    pub fn text_to_ipa(&self, text: &str) -> Result<String> {
        let lang = &self.options.lang;
        let dict_path = self.data_dir.join(format!("{}_dict", lang));
        let phontab_path = self.data_dir.join("phontab");

        // Load dictionary
        if !dict_path.exists() {
            return Err(Error::NotImplemented("text_to_ipa: dict not found"));
        }
        let dict_bytes = std::fs::read(&dict_path)
            .map_err(Error::Io)?;
        let dict = Dictionary::from_bytes(lang, dict_bytes)?;

        // Load phoneme data
        if !phontab_path.exists() {
            return Err(Error::NotImplemented("text_to_ipa: phontab not found"));
        }
        let mut phdata = PhonemeData::load(&self.data_dir)?;
        phdata.select_table_by_name(lang)?;

        // Build stress options for this language
        let stress_opts = StressOpts::for_lang(lang);

        // Tokenize
        let tokens = tokenize(text);

        // Translate all words first, collecting (phonemes, dict_flags) pairs
        #[derive(Clone, PartialEq)]
        enum EntryKind {
            Word,
            ClauseBoundary,
            Other,
        }

        struct EntryFull {
            phonemes: Vec<u8>,
            dict_flags: u32,
            kind: EntryKind,
        }

        let mut entries: Vec<EntryFull> = Vec::new();

        for token in &tokens {
            match token {
                Token::Word(word) => {
                    let lower = word.to_lowercase();
                    let wr = word_to_phonemes(&lower, &dict, &phdata, &stress_opts);
                    entries.push(EntryFull {
                        phonemes: wr.phonemes,
                        dict_flags: wr.dict_flags,
                        kind: EntryKind::Word,
                    });
                }
                Token::ClauseBoundary(_) => {
                    entries.push(EntryFull {
                        phonemes: Vec::new(),
                        dict_flags: 0,
                        kind: EntryKind::ClauseBoundary,
                    });
                }
                _ => {
                    entries.push(EntryFull {
                        phonemes: Vec::new(),
                        dict_flags: 0,
                        kind: EntryKind::Other,
                    });
                }
            }
        }

        // Split entries into clauses at ClauseBoundary tokens, apply promotions per-clause
        const FLAG_STREND:  u32 = 1 << 9;   // 0x200
        const FLAG_STREND2: u32 = 1 << 10;  // 0x400
        const PHON_STRESS_P_CODE: u8 = 6;
        const PHON_STRESS_P2_CODE: u8 = 7;

        /// Apply strend + clause-level stress promotion to a slice of entries.
        /// Entries in slice form one clause (no ClauseBoundary tokens inside).
        fn promote_clause(entries: &mut [EntryFull], phdata: &PhonemeData) {
            // $strend/$strend2 promotion
            let n = entries.len();
            for i in 0..n {
                if entries[i].kind != EntryKind::Word { continue; }
                let dict_flags = entries[i].dict_flags;
                if dict_flags & (FLAG_STREND | FLAG_STREND2) == 0 { continue; }

                let is_last_word = entries[i+1..].iter().all(|e| e.kind != EntryKind::Word);
                let following_all_unstressed = entries[i+1..].iter()
                    .filter(|e| e.kind == EntryKind::Word)
                    .all(|e| !e.phonemes.iter().any(|&c| c == PHON_STRESS_P_CODE || c == PHON_STRESS_P2_CODE));

                promote_strend_stress(
                    &mut entries[i].phonemes,
                    phdata,
                    dict_flags,
                    is_last_word,
                    following_all_unstressed,
                );
            }

            // Clause-level stress promotion: if no primary stress, promote last stressed word
            let has_primary = entries.iter()
                .filter(|e| e.kind == EntryKind::Word)
                .any(|e| e.phonemes.iter().any(|&c| c == PHON_STRESS_P_CODE || c == PHON_STRESS_P2_CODE));

            if !has_primary {
                let last_secondary = entries.iter().enumerate()
                    .rev()
                    .find(|(_, e)| e.kind == EntryKind::Word && !e.phonemes.is_empty()
                        && e.phonemes.iter().any(|&c| c == 4 || c == 5))
                    .map(|(i, _)| i);

                if let Some(idx) = last_secondary {
                    change_word_stress(&mut entries[idx].phonemes, phdata, 4);
                } else {
                    let last_word = entries.iter().enumerate()
                        .rev()
                        .find(|(_, e)| e.kind == EntryKind::Word && !e.phonemes.is_empty())
                        .map(|(i, _)| i);
                    if let Some(idx) = last_word {
                        change_word_stress(&mut entries[idx].phonemes, phdata, 4);
                    }
                }
            }
        }

        // Find clause boundaries and promote each clause independently
        let clause_boundaries: Vec<usize> = {
            let mut tmp = Vec::new();
            for i in 0..entries.len() {
                if entries[i].kind == EntryKind::ClauseBoundary {
                    tmp.push(i);
                }
            }
            tmp
        };

        if clause_boundaries.is_empty() {
            // Single clause
            promote_clause(&mut entries, &phdata);
        } else {
            // Multiple clauses: promote each separately
            let mut prev_end = 0usize;
            let mut boundaries_with_end: Vec<usize> = clause_boundaries.clone();
            boundaries_with_end.push(entries.len()); // sentinel
            for &bound in &boundaries_with_end {
                let slice_end = if bound < entries.len() { bound } else { entries.len() };
                if slice_end > prev_end {
                    promote_clause(&mut entries[prev_end..slice_end], &phdata);
                }
                prev_end = if bound < entries.len() { bound + 1 } else { entries.len() };
            }
        }

        // Render to IPA with clause boundary newlines
        let mut ipa_out = String::new();
        let mut first_word = true;
        let mut clause_has_output = false;
        let mut stress = PendingStress::None;

        for (ei, entry) in entries.iter().enumerate() {
            match entry.kind {
                EntryKind::Word => {
                    let phonemes = &entry.phonemes;
                    if phonemes.is_empty() { continue; }
                    let use_en_overrides = lang == "en";
                    // Check if next word starts with a vowel (for liaison)
                    let next_starts_vowel = entries[ei+1..].iter()
                        .find(|e| e.kind == EntryKind::Word && !e.phonemes.is_empty())
                        .map(|e| {
                            // Check first real phoneme
                            e.phonemes.iter()
                                .find(|&&c| c > 8 && c != 15)
                                .and_then(|&c| phdata.get(c))
                                .map(|ph| ph.typ == 2) // phVOWEL
                                .unwrap_or(false)
                        })
                        .unwrap_or(false);
                    // Suppress liaison phonemes at word-final when next word is not vowel-initial
                    let suppress_liaison = !next_starts_vowel;
                    let (word_ipa, new_stress) = phonemes_to_ipa_full(
                        phonemes,
                        &phdata,
                        stress,
                        !first_word,
                        use_en_overrides,
                        suppress_liaison,
                    );
                    stress = new_stress;
                    if !word_ipa.is_empty() {
                        ipa_out.push_str(&word_ipa);
                        first_word = false;
                        clause_has_output = true;
                    }
                }
                EntryKind::ClauseBoundary => {
                    // Output \n between clauses (when previous clause had output)
                    if clause_has_output {
                        ipa_out.push('\n');
                        clause_has_output = false;
                        stress = PendingStress::None;
                    }
                    // Reset word spacing for new clause
                    first_word = true;
                }
                EntryKind::Other => {}
            }
        }

        let mut ipa_out = ipa_out.trim_end_matches('\n').to_string();
        if lang == "fr" {
            ipa_out = ipa_out.replace('r', "ʁ");
        }

        Ok(ipa_out)
    }

    /// Translate text into a raw phoneme-code sequence for synthesis.
    ///
    /// Returns a `Vec<PhonemeCode>` where each item describes one phoneme
    /// event (phoneme code + stress level).  This is the intermediate
    /// representation between the dictionary/rule engine and the IPA renderer;
    /// exposing it lets the synthesizer drive waveform generation directly
    /// from espeak-ng's own acoustic data files.
    ///
    /// # Phoneme code conventions (mirroring synthesize.h)
    /// | Code | Meaning                       |
    /// |------|-------------------------------|
    /// | 0    | silence / pause               |
    /// | 1–7  | stress markers                |
    /// | 9    | explicit pause                |
    /// | 12   | length mark (:)               |
    /// | 15   | word boundary (||)            |
    /// | 35+  | actual phoneme                |
    pub fn translate_to_codes(&self, text: &str) -> Result<Vec<PhonemeCode>> {
        let lang = &self.options.lang;
        let dict_path = self.data_dir.join(format!("{}_dict", lang));
        let phontab_path = self.data_dir.join("phontab");

        if !dict_path.exists() {
            return Err(Error::NotImplemented("translate_to_codes: dict not found"));
        }
        let dict_bytes = std::fs::read(&dict_path).map_err(Error::Io)?;
        let dict = Dictionary::from_bytes(lang, dict_bytes)?;

        if !phontab_path.exists() {
            return Err(Error::NotImplemented("translate_to_codes: phontab not found"));
        }
        let mut phdata = PhonemeData::load(&self.data_dir)?;
        phdata.select_table_by_name(lang)?;
        let stress_opts = StressOpts::for_lang(lang);

        let tokens = tokenize(text);
        let mut codes: Vec<PhonemeCode> = Vec::new();

        for token in &tokens {
            match token {
                Token::Word(word) => {
                    let lower = word.to_lowercase();
                    let wr = word_to_phonemes(&lower, &dict, &phdata, &stress_opts);
                    for &b in &wr.phonemes {
                        codes.push(PhonemeCode { code: b, is_boundary: false });
                    }
                }
                Token::Space => {
                    codes.push(PhonemeCode { code: 15, is_boundary: true }); // END_WORD
                }
                Token::ClauseBoundary(_) => {
                    codes.push(PhonemeCode { code: 0, is_boundary: true }); // pause
                }
                _ => {}
            }
        }

        Ok(codes)
    }
}

/// A single phoneme event in the synthesizer's input stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhonemeCode {
    /// espeak-ng phoneme code.  See `synthesize.h` and the phoneme data files.
    pub code: u8,
    /// True if this is a boundary marker (word boundary, clause boundary).
    pub is_boundary: bool,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translator_new_default_succeeds() {
        // Should succeed even if data files don't exist (just builds the struct)
        let t = Translator::new_default("en").unwrap();
        assert_eq!(t.options.lang, "en");
        assert_eq!(t.options.rate, 175);
    }

    #[test]
    fn tokenize_hello_world() {
        let tokens = tokenize("hello world");
        assert_eq!(tokens, vec![
            Token::Word("hello".to_string()),
            Token::Space,
            Token::Word("world".to_string()),
        ]);
    }

    #[test]
    fn tokenize_with_punctuation() {
        let tokens = tokenize("hello, world!");
        assert!(tokens.iter().any(|t| t == &Token::Word("hello".to_string())));
        assert!(tokens.iter().any(|t| t == &Token::Word("world".to_string())));
        assert!(tokens.iter().any(|t| t == &Token::ClauseBoundary(',')));
        assert!(tokens.iter().any(|t| t == &Token::ClauseBoundary('!')));
    }

    #[test]
    fn tokenize_empty() {
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn tokenize_apostrophe() {
        let tokens = tokenize("it's");
        assert_eq!(tokens, vec![Token::Word("it's".to_string())]);
    }

    #[test]
    fn clause_flags_fields_do_not_overlap() {
        assert!(
            (ClauseFlags::PAUSE_MASK & ClauseFlags::INTONATION_MASK).is_empty()
        );
        assert!(
            (ClauseFlags::INTONATION_MASK & ClauseFlags::TYPE_MASK).is_empty()
        );
    }

    #[test]
    fn read_clauses_basic() {
        let t = Translator::new_default("en").unwrap();
        let clauses = t.read_clauses("Hello world. How are you?").unwrap();
        assert_eq!(clauses.len(), 2);
        assert_eq!(clauses[0].intonation, Intonation::FullStop);
        assert_eq!(clauses[1].intonation, Intonation::Question);
    }

    #[test]
    fn read_clauses_no_punctuation() {
        let t = Translator::new_default("en").unwrap();
        let clauses = t.read_clauses("hello world").unwrap();
        assert_eq!(clauses.len(), 1);
        assert_eq!(clauses[0].text, "hello world");
    }

    // ── phonemes_to_ipa ────────────────────────────────────────────────────

    fn make_phdata() -> Option<PhonemeData> {
        let dir = std::path::Path::new("/usr/share/espeak-ng-data");
        if !dir.join("phontab").exists() { return None; }
        let mut phdata = PhonemeData::load(dir).ok()?;
        phdata.select_table_by_name("en").ok()?;
        Some(phdata)
    }

    #[test]
    fn phonemes_to_ipa_the() {
        // "the" dict phonemes: [87, 115] = [D, @2] → ðə
        let phdata = match make_phdata() { Some(d) => d, None => return };
        let (ipa, _) = phonemes_to_ipa(&[87, 115], &phdata, PendingStress::None, false);
        assert_eq!(ipa, "ðə");
    }

    #[test]
    fn phonemes_to_ipa_be() {
        // "be" dict phonemes: [72, 137] = [b, i:] → biː
        let phdata = match make_phdata() { Some(d) => d, None => return };
        let (ipa, _) = phonemes_to_ipa(&[72, 137], &phdata, PendingStress::None, false);
        assert_eq!(ipa, "biː");
    }

    #[test]
    fn phonemes_to_ipa_with_stress() {
        // "not" dict: [4, 50, 129, 47] = [STRESS_2, n, 0, t]
        // secondary stress before the vowel (ɒ), consonant onset comes before stress mark
        // → "nˌɒt" (stress mark immediately before the stressed vowel)
        let phdata = match make_phdata() { Some(d) => d, None => return };
        let (ipa, _) = phonemes_to_ipa(&[4, 50, 129, 47], &phdata, PendingStress::None, false);
        assert_eq!(ipa, "nˌɒt");
    }

    #[test]
    fn text_to_ipa_be() {
        let t = Translator::new_default("en").unwrap();
        if !Path::new("/usr/share/espeak-ng-data/en_dict").exists() { return; }
        let ipa = t.text_to_ipa("be").unwrap();
        // "be" in isolation gets primary stress via clause-level promotion
        assert_eq!(ipa, "bˈiː");
    }

    #[test]
    fn text_to_ipa_he() {
        let t = Translator::new_default("en").unwrap();
        if !Path::new("/usr/share/espeak-ng-data/en_dict").exists() { return; }
        let ipa = t.text_to_ipa("he").unwrap();
        // "he" in isolation gets primary stress via clause-level promotion
        assert_eq!(ipa, "hˈiː");
    }

    #[test]
    fn text_to_ipa_do() {
        let t = Translator::new_default("en").unwrap();
        if !Path::new("/usr/share/espeak-ng-data/en_dict").exists() { return; }
        let ipa = t.text_to_ipa("do").unwrap();
        assert_eq!(ipa, "dˈuː");
    }

    #[test]
    fn text_to_ipa_the() {
        let t = Translator::new_default("en").unwrap();
        if !Path::new("/usr/share/espeak-ng-data/en_dict").exists() { return; }
        let ipa = t.text_to_ipa("the").unwrap();
        // "the" in isolation gets primary stress via clause-level promotion (matches C oracle)
        assert_eq!(ipa, "ðˈə");
    }

    // ── CJK tokenize ────────────────────────────────────────────────────

    #[test]
    fn tokenize_chinese_chars_are_individual_words() {
        let tokens = tokenize("你好世界");
        assert_eq!(tokens, vec![
            Token::Word("你".to_string()),
            Token::Word("好".to_string()),
            Token::Word("世".to_string()),
            Token::Word("界".to_string()),
        ]);
    }

    #[test]
    fn tokenize_cjk_with_spaces() {
        let tokens = tokenize("你好 世界");
        assert_eq!(tokens, vec![
            Token::Word("你".to_string()),
            Token::Word("好".to_string()),
            Token::Space,
            Token::Word("世".to_string()),
            Token::Word("界".to_string()),
        ]);
    }

    #[test]
    fn tokenize_mixed_cjk_and_latin() {
        let tokens = tokenize("Hello你好World世界");
        assert_eq!(tokens, vec![
            Token::Word("Hello".to_string()),
            Token::Word("你".to_string()),
            Token::Word("好".to_string()),
            Token::Word("World".to_string()),
            Token::Word("世".to_string()),
            Token::Word("界".to_string()),
        ]);
    }

    #[test]
    fn tokenize_single_cjk_char() {
        let tokens = tokenize("你");
        assert_eq!(tokens, vec![Token::Word("你".to_string())]);
    }

    #[test]
    fn tokenize_cjk_with_punctuation() {
        let tokens = tokenize("你好，世界！");
        assert!(tokens.contains(&Token::Word("你".to_string())));
        assert!(tokens.contains(&Token::Word("好".to_string())));
        assert!(tokens.contains(&Token::Word("世".to_string())));
        assert!(tokens.contains(&Token::Word("界".to_string())));
    }
}
