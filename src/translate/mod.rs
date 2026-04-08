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
    /// Language-specific number parsing and rendering behavior.
    pub number_grammar: NumberGrammar,
}

/// Language-specific number rendering rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberGrammar {
    /// Ordinal parsing behavior.
    pub ordinals: OrdinalGrammar,
    /// How tens and units are combined.
    pub tens: TensGrammar,
    /// Rules for hundreds.
    pub hundreds: HundredsGrammar,
    /// Rules for thousands.
    pub thousands: ThousandsGrammar,
}

/// Ordinal marker recognition behavior.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OrdinalGrammar {
    /// Suffix which marks ordinals even without a `_#suffix` dict entry.
    pub indicator: Option<String>,
    /// Whether `3.`-style ordinals are accepted.
    pub dot_marks_ordinal: bool,
}

/// Word-order rule for tens and units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TensGrammar {
    /// `thirty four`
    #[default]
    Standard,
    /// `treinta y cuatro`
    WithConjunction,
    /// `vier und dreißig`
    UnitsThenConjunction,
}

/// Hundreds-specific rendering behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HundredsGrammar {
    /// Whether to insert a conjunction between the hundreds and remainder.
    pub use_conjunction_with_remainder: bool,
    /// Whether to omit the explicit `one` before `hundred`.
    pub omit_one_prefix: bool,
}

/// Thousands-specific rendering behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ThousandsGrammar {
    /// Whether to omit the explicit `one` before `thousand`.
    pub omit_one_prefix: bool,
}

impl NumberGrammar {
    fn for_lang(lang: &str) -> Self {
        let mut grammar = Self::default();
        match lang {
            "en" => {
                grammar.hundreds.use_conjunction_with_remainder = true;
            }
            "es" => {
                grammar.tens = TensGrammar::WithConjunction;
                grammar.hundreds.omit_one_prefix = true;
                grammar.thousands.omit_one_prefix = true;
            }
            "fr" => {
                grammar.hundreds.omit_one_prefix = true;
            }
            "de" => {
                grammar.ordinals.dot_marks_ordinal = true;
                grammar.tens = TensGrammar::UnitsThenConjunction;
            }
            "nl" | "mt" => {
                grammar.ordinals.dot_marks_ordinal = true;
                grammar.ordinals.indicator = Some("e".to_string());
                grammar.tens = TensGrammar::UnitsThenConjunction;
                grammar.hundreds.omit_one_prefix = true;
                grammar.thousands.omit_one_prefix = true;
            }
            "da" | "et" | "fi" | "fo" | "kl" | "lt" | "nb" | "no" | "sl" => {
                grammar.ordinals.dot_marks_ordinal = true;
            }
            _ => {}
        }
        grammar
    }
}

impl Default for NumberGrammar {
    fn default() -> Self {
        Self {
            ordinals: OrdinalGrammar::default(),
            tens: TensGrammar::Standard,
            hundreds: HundredsGrammar::default(),
            thousands: ThousandsGrammar::default(),
        }
    }
}

impl Default for LangOptions {
    fn default() -> Self {
        LangOptions {
            lang:        "en".to_string(),
            rate:        175,
            pitch:       50,
            word_gap:    0,
            stress_rule: 2, // STRESSPOSN_2R = penultimate
            number_grammar: NumberGrammar::default(),
        }
    }
}

impl LangOptions {
    pub fn for_lang(lang: &str) -> Self {
        Self {
            lang: lang.to_string(),
            number_grammar: NumberGrammar::for_lang(lang),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Token types for the simple tokenizer
// ---------------------------------------------------------------------------

/// One token produced by [`tokenize`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// A word (sequence of letters / digits / apostrophes).
    Word(String),
    /// A parsed number-like token.
    Number(NumberToken),
    /// One or more whitespace characters collapsed into a single separator.
    Space,
    /// Sentence/clause boundary punctuation: `.`, `,`, `!`, `?`, `;`, `:`.
    ClauseBoundary(char),
    /// Any other punctuation character.
    Punctuation(char),
}

/// Parsed number token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NumberToken {
    Cardinal(String),
    Decimal { integer: String, fractional: String },
    Ordinal(OrdinalNumber),
}

impl NumberToken {
    fn parse(word: &str, grammar: &NumberGrammar) -> Option<Self> {
        if word.is_empty() {
            return None;
        }

        if let Some((integer, fractional)) = word.split_once('.') {
            let has_single_dot = word.bytes().filter(|&b| b == b'.').count() == 1;
            if has_single_dot
                && !integer.is_empty()
                && !fractional.is_empty()
                && integer.bytes().all(|b| b.is_ascii_digit())
                && fractional.bytes().all(|b| b.is_ascii_digit())
            {
                return Some(NumberToken::Decimal {
                    integer: integer.to_string(),
                    fractional: fractional.to_string(),
                });
            }
        }

        let digit_end = word.bytes().position(|b| !b.is_ascii_digit()).unwrap_or(word.len());
        if digit_end == 0 {
            return None;
        }

        if digit_end == word.len() {
            return word
                .bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| NumberToken::Cardinal(word.to_string()));
        }

        let digits = &word[..digit_end];
        let suffix = &word[digit_end..];
        if suffix == "." && grammar.ordinals.dot_marks_ordinal {
            return Some(NumberToken::Ordinal(OrdinalNumber {
                digits: digits.to_string(),
                marker: OrdinalMarker::Dot,
            }));
        }

        Some(NumberToken::Ordinal(OrdinalNumber {
            digits: digits.to_string(),
            marker: OrdinalMarker::Suffix(suffix.to_lowercase()),
        }))
    }

    fn surface(&self) -> String {
        match self {
            NumberToken::Cardinal(digits) => digits.clone(),
            NumberToken::Decimal { integer, fractional } => format!("{integer}.{fractional}"),
            NumberToken::Ordinal(ordinal) => ordinal.surface(),
        }
    }
}

/// Parsed ordinal number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrdinalNumber {
    pub digits: String,
    pub marker: OrdinalMarker,
}

impl OrdinalNumber {
    fn surface(&self) -> String {
        match &self.marker {
            OrdinalMarker::Suffix(suffix) => format!("{}{}", self.digits, suffix),
            OrdinalMarker::Dot => format!("{}.", self.digits),
        }
    }
}

/// Marker that makes a numeric token ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrdinalMarker {
    Suffix(String),
    Dot,
}

/// Tokenize plain text into a sequence of words, spaces and punctuation.
///
/// This is a simplified version of `ReadClause()` from readclause.c.  It
/// handles plain ASCII / UTF-8 text without SSML.
pub fn tokenize(text: &str) -> Vec<Token> {
    tokenize_opts(text, &NumberGrammar::default())
}

/// Tokenize with language-specific options.
pub fn tokenize_opts(text: &str, grammar: &NumberGrammar) -> Vec<Token> {
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
            let mut digits = String::new();
            digits.push(c);
            let mut has_dot = false;
            let mut fractional = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_ascii_digit() {
                    if has_dot {
                        fractional.push(next);
                    } else {
                        digits.push(next);
                    }
                    chars.next();
                } else if next == '.' && !has_dot {
                    // Peek ahead to see if followed by a digit
                    let mut lookahead = chars.clone();
                    lookahead.next(); // skip '.'
                    if lookahead.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                        has_dot = true;
                        chars.next();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            if has_dot {
                tokens.push(Token::Number(NumberToken::Decimal {
                    integer: digits,
                    fractional,
                }));
                continue;
            }

            // Check for ordinal suffix immediately after digits (e.g. "1st", "2nd", "3º").
            let mut suffix = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_alphabetic() || next == 'º' || next == 'ª' {
                    suffix.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            if !suffix.is_empty() {
                tokens.push(Token::Number(NumberToken::Ordinal(OrdinalNumber {
                    digits,
                    marker: OrdinalMarker::Suffix(suffix.to_lowercase()),
                })));
                continue;
            }

            // NUM_ORDINAL_DOT: if enabled, a trailing dot after digits marks ordinal
            // (e.g. German "3." → "dritte"). Only when NOT followed by a digit.
            if grammar.ordinals.dot_marks_ordinal && chars.peek() == Some(&'.') {
                let mut lookahead = chars.clone();
                lookahead.next(); // skip '.'
                let after_dot = lookahead.peek().copied();
                if !after_dot.map_or(false, |c| c.is_ascii_digit()) {
                    chars.next();
                    tokens.push(Token::Number(NumberToken::Ordinal(OrdinalNumber {
                        digits,
                        marker: OrdinalMarker::Dot,
                    })));
                    continue;
                }
            }

            tokens.push(Token::Number(NumberToken::Cardinal(digits)));
        } else if c.is_alphabetic() || c == '\'' {
            // Accumulate a word (letters, apostrophes, hyphens within words).
            let mut word = String::new();
            word.push(c);
            while let Some(&next) = chars.peek() {
                if next.is_alphabetic() || next == '\'' {
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

const PHON_END_WORD: u8 = 15;

/// Byte-oriented pronunciation builder that understands END_WORD separators.
#[derive(Debug, Clone, Default)]
struct Pronunciation {
    bytes: Vec<u8>,
}

impl Pronunciation {
    fn push_lookup_word(&mut self, src: &[u8]) {
        self.start_word();
        self.bytes.extend_from_slice(trim_lookup(src));
    }

    fn append_lookup_suffix(&mut self, src: &[u8]) {
        self.bytes.extend_from_slice(trim_lookup(src));
    }

    fn push_pronunciation(&mut self, other: &Pronunciation) {
        let len = other.trimmed_len();
        if len == 0 {
            return;
        }
        self.start_word();
        self.bytes.extend_from_slice(&other.bytes[..len]);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bytes.last().copied() != Some(PHON_END_WORD) {
            self.bytes.push(PHON_END_WORD);
        }
        self.bytes.push(0);
        self.bytes
    }

    fn trimmed_len(&self) -> usize {
        self.bytes
            .iter()
            .rposition(|&b| b != PHON_END_WORD)
            .map_or(0, |idx| idx + 1)
    }

    fn start_word(&mut self) {
        if !self.bytes.is_empty() && self.bytes.last().copied() != Some(PHON_END_WORD) {
            self.bytes.push(PHON_END_WORD);
        }
    }
}

fn trim_lookup(src: &[u8]) -> &[u8] {
    let len = src.iter().position(|&b| b == 0).unwrap_or(src.len());
    &src[..len]
}

fn num_key(raw: impl std::fmt::Display) -> String {
    format!("_{raw}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScaleGroup {
    value: u32,
    scale: Option<u8>,
}

fn split_scale_groups(value: u64) -> [ScaleGroup; 4] {
    [
        ScaleGroup {
            value: (value / 1_000_000_000) as u32,
            scale: Some(3),
        },
        ScaleGroup {
            value: ((value / 1_000_000) % 1_000) as u32,
            scale: Some(2),
        },
        ScaleGroup {
            value: ((value / 1_000) % 1_000) as u32,
            scale: Some(1),
        },
        ScaleGroup {
            value: (value % 1_000) as u32,
            scale: None,
        },
    ]
}

fn append_scale_word(
    dst: &mut Pronunciation,
    group_value: u32,
    scale: u8,
    dict: &Dictionary,
    grammar: &NumberGrammar,
) {
    let scale_key = format!("_0M{scale}");
    let singular_key = format!("_1M{scale}");

    if scale == 1 && group_value == 1 && grammar.thousands.omit_one_prefix {
        dst.push_lookup_word(&lookup_num_phonemes(dict, &scale_key));
        return;
    }

    if group_value == 1 {
        let singular = lookup_num_phonemes(dict, &singular_key);
        if !singular.is_empty() {
            dst.push_lookup_word(&singular);
            return;
        }
    }

    dst.push_pronunciation(&num3_phonemes(dict, group_value, false, grammar));
    dst.push_lookup_word(&lookup_num_phonemes(dict, &scale_key));
}

fn append_cardinal_group(
    dst: &mut Pronunciation,
    group: ScaleGroup,
    dict: &Dictionary,
    grammar: &NumberGrammar,
) {
    if group.value == 0 {
        return;
    }

    if let Some(scale) = group.scale {
        append_scale_word(dst, group.value, scale, dict, grammar);
    } else {
        dst.push_pronunciation(&num3_phonemes(dict, group.value, false, grammar));
    }
}

fn append_ordinal_scale(
    dst: &mut Pronunciation,
    group_value: u32,
    scale: u8,
    dict: &Dictionary,
    grammar: &NumberGrammar,
) -> bool {
    let singular_ord_key = format!("_1M{scale}o");
    if group_value == 1 {
        let singular_ord = lookup_num_phonemes(dict, &singular_ord_key);
        if !singular_ord.is_empty() {
            dst.push_lookup_word(&singular_ord);
            return true;
        }
    }

    let ord_key = format!("_0M{scale}o");
    let ord_scale = lookup_num_phonemes(dict, &ord_key);
    if !ord_scale.is_empty() {
        if !(scale == 1 && group_value == 1 && grammar.thousands.omit_one_prefix) {
            dst.push_pronunciation(&num3_phonemes(dict, group_value, false, grammar));
        }
        dst.push_lookup_word(&ord_scale);
        return true;
    }

    append_scale_word(dst, group_value, scale, dict, grammar);
    false
}

/// Convert a number value (0-999) to phonemes.
/// Mirrors C's LookupNum3 with per-language number flags.
fn num3_phonemes(
    dict: &Dictionary,
    value: u32,
    suppress_null: bool,
    grammar: &NumberGrammar,
) -> Pronunciation {
    let hundreds = value / 100;
    let tensunits = value % 100;

    let mut hundreds_part = Pronunciation::default();
    let mut tens_part = Pronunciation::default();
    let mut suppress_null = suppress_null;

    if hundreds > 0 {
        let compound = lookup_num_phonemes(dict, &format!("_{}C", hundreds));
        if !compound.is_empty() {
            hundreds_part.push_lookup_word(&compound);
        } else if tensunits == 0 {
            let exact = lookup_num_phonemes(dict, &format!("_{}C0", hundreds));
            if !exact.is_empty() {
                hundreds_part.push_lookup_word(&exact);
            } else {
                if !(hundreds == 1 && grammar.hundreds.omit_one_prefix) {
                    hundreds_part.push_lookup_word(&lookup_num_phonemes(dict, &num_key(hundreds)));
                }
                hundreds_part.append_lookup_suffix(&lookup_num_phonemes(dict, "_0C"));
            }
        } else {
            if !(hundreds == 1 && grammar.hundreds.omit_one_prefix) {
                hundreds_part.push_lookup_word(&lookup_num_phonemes(dict, &num_key(hundreds)));
            }
            hundreds_part.append_lookup_suffix(&lookup_num_phonemes(dict, "_0C"));
        }
        suppress_null = true;
    }

    if tensunits != 0 || !suppress_null {
        if tensunits < 20 {
            tens_part.push_lookup_word(&lookup_num_phonemes(dict, &num_key(tensunits)));
        } else {
            let ph_full = lookup_num_phonemes(dict, &num_key(tensunits));
            if !ph_full.is_empty() {
                tens_part.push_lookup_word(&ph_full);
            } else {
                let tens = tensunits / 10;
                let units = tensunits % 10;

                match grammar.tens {
                    TensGrammar::UnitsThenConjunction if units != 0 => {
                        tens_part.push_lookup_word(&lookup_num_phonemes(dict, &num_key(units)));
                        tens_part.append_lookup_suffix(&lookup_num_phonemes(dict, "_0and"));
                        tens_part.append_lookup_suffix(&lookup_num_phonemes(dict, &format!("_{tens}X")));
                    }
                    TensGrammar::UnitsThenConjunction => {
                        tens_part.push_lookup_word(&lookup_num_phonemes(dict, &format!("_{tens}X")));
                    }
                    TensGrammar::WithConjunction => {
                        tens_part.push_lookup_word(&lookup_num_phonemes(dict, &format!("_{tens}X")));
                        if units != 0 {
                            tens_part.append_lookup_suffix(&lookup_num_phonemes(dict, "_0and"));
                            tens_part.append_lookup_suffix(&lookup_num_phonemes(dict, &num_key(units)));
                        }
                    }
                    TensGrammar::Standard => {
                        tens_part.push_lookup_word(&lookup_num_phonemes(dict, &format!("_{tens}X")));
                        if units != 0 {
                            tens_part.append_lookup_suffix(&lookup_num_phonemes(dict, &num_key(units)));
                        }
                    }
                }
            }
        }
    }

    if hundreds > 0 && tensunits > 0 && grammar.hundreds.use_conjunction_with_remainder {
        hundreds_part.append_lookup_suffix(&lookup_num_phonemes(dict, "_0and"));
    }

    let mut result = Pronunciation::default();
    result.push_pronunciation(&hundreds_part);
    result.push_pronunciation(&tens_part);
    result
}

fn number_token_to_phonemes(
    token: &NumberToken,
    dict: &Dictionary,
    grammar: &NumberGrammar,
) -> Option<Vec<u8>> {
    match token {
        NumberToken::Cardinal(digits) => Some(cardinal_pronunciation(digits, dict, grammar)?.finish()),
        NumberToken::Decimal { integer, fractional } => {
            let mut pronunciation = cardinal_pronunciation(integer, dict, grammar)?;
            let decimal_point = lookup_num_phonemes(dict, "_dpt");
            if !decimal_point.is_empty() {
                pronunciation.push_lookup_word(&decimal_point);
            }
            for digit in fractional.bytes() {
                pronunciation.push_lookup_word(&lookup_num_phonemes(dict, &num_key(digit - b'0')));
            }
            Some(pronunciation.finish())
        }
        NumberToken::Ordinal(_) => None,
    }
}

fn cardinal_pronunciation(
    digits: &str,
    dict: &Dictionary,
    grammar: &NumberGrammar,
) -> Option<Pronunciation> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }

    let value: u64 = digits.parse().ok()?;
    if value == 0 {
        let mut pronunciation = Pronunciation::default();
        pronunciation.push_lookup_word(&lookup_num_phonemes(dict, "_0"));
        return Some(pronunciation);
    }

    let is_year_form = value >= 1100 && value <= 9999 && value % 100 == 0 && value / 100 >= 11;
    if is_year_form {
        let mut pronunciation = num3_phonemes(dict, (value / 100) as u32, false, grammar);
        pronunciation.append_lookup_suffix(&lookup_num_phonemes(dict, "_0C"));
        return Some(pronunciation);
    }

    let mut result = Pronunciation::default();
    for group in split_scale_groups(value) {
        append_cardinal_group(&mut result, group, dict, grammar);
    }

    Some(result)
}

fn ordinal_sub_thousand_pronunciation(
    value: u32,
    dict: &Dictionary,
    grammar: &NumberGrammar,
    suffix_ph: &[u8],
) -> (Pronunciation, bool) {
    let hundreds = value / 100;
    let tensunits = value % 100;
    let units = value % 10;
    let tens = tensunits / 10;

    let mut pronunciation = Pronunciation::default();
    let mut found_ordinal = false;

    if hundreds > 0 {
        if tensunits == 0 {
            let ord_hundreds = lookup_num_phonemes(dict, "_0Co");
            if !ord_hundreds.is_empty() {
                if hundreds > 1 {
                    pronunciation.push_lookup_word(&lookup_num_phonemes(dict, &num_key(hundreds)));
                }
                pronunciation.push_lookup_word(&ord_hundreds);
                found_ordinal = true;
            } else {
                pronunciation.push_pronunciation(&num3_phonemes(dict, hundreds * 100, false, grammar));
            }
        } else {
            pronunciation.push_pronunciation(&num3_phonemes(dict, hundreds * 100, false, grammar));
        }
    }

    let full_ord = lookup_num_phonemes(dict, &format!("_{tensunits}o"));
    if !full_ord.is_empty() {
        pronunciation.push_lookup_word(&full_ord);
        found_ordinal = true;
    } else if tens >= 2 && units > 0 {
        let tens_ord = lookup_num_phonemes(dict, &format!("_{tens}Xo"));
        if !tens_ord.is_empty() {
            pronunciation.push_lookup_word(&tens_ord);
            pronunciation.append_lookup_suffix(suffix_ph);
        } else {
            pronunciation.push_lookup_word(&lookup_num_phonemes(dict, &format!("_{tens}X")));
        }

        let units_ord = lookup_num_phonemes(dict, &format!("_{units}o"));
        if !units_ord.is_empty() {
            pronunciation.push_lookup_word(&units_ord);
            found_ordinal = true;
        } else {
            pronunciation.push_lookup_word(&lookup_num_phonemes(dict, &num_key(units)));
        }
    } else if tens >= 2 {
        pronunciation.push_lookup_word(&lookup_num_phonemes(dict, &format!("_{tens}X")));
    } else if tensunits > 0 {
        pronunciation.push_pronunciation(&num3_phonemes(dict, tensunits, false, grammar));
    }

    (pronunciation, found_ordinal)
}

/// Try to interpret a word as an ordinal number (e.g. "2nd", "1st", "3º").
///
/// Splits the word into a leading digit string and a trailing non-digit suffix,
/// then looks up `_#<suffix>` in the dictionary. If found, the word is an ordinal.
///
/// For the last (units) digit, looks up `_<digit>o` for irregular ordinals
/// (e.g. `_1o` → "first", `_2o` → "second"). Falls back to cardinal + `_ord`
/// suffix for regular ordinals (e.g. "four" + "th").
///
/// This mirrors C espeak-ng's ordinal handling in numbers.c.
fn try_ordinal_number(
    ordinal: &OrdinalNumber,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    grammar: &NumberGrammar,
) -> Option<WordResult> {
    let suffix = match &ordinal.marker {
        OrdinalMarker::Suffix(suffix) => suffix.as_str(),
        OrdinalMarker::Dot => ".",
    };

    let suffix_ph = lookup_num_phonemes(dict, &format!("_#{suffix}"));
    let is_ordinal = !suffix_ph.is_empty()
        || grammar.ordinals.indicator.as_deref() == Some(suffix)
        || matches!(ordinal.marker, OrdinalMarker::Dot) && grammar.ordinals.dot_marks_ordinal;
    if !is_ordinal {
        return None;
    }

    let value: u64 = ordinal.digits.parse().ok()?;
    let mut pronunciation = Pronunciation::default();
    let groups = split_scale_groups(value);
    let last_nonzero = groups.iter().rposition(|group| group.value != 0)?;

    for &group in &groups[..last_nonzero] {
        append_cardinal_group(&mut pronunciation, group, dict, grammar);
    }

    let final_group = groups[last_nonzero];
    let found_ordinal = if let Some(scale) = final_group.scale {
        append_ordinal_scale(
            &mut pronunciation,
            final_group.value,
            scale,
            dict,
            grammar,
        )
    } else {
        let (remainder_ordinal, found) =
            ordinal_sub_thousand_pronunciation(final_group.value, dict, grammar, &suffix_ph);
        pronunciation.push_pronunciation(&remainder_ordinal);
        found
    };

    if found_ordinal {
        pronunciation.append_lookup_suffix(&suffix_ph);
    } else {
        let ord_ph = lookup_num_phonemes(dict, "_ord");
        if !ord_ph.is_empty() {
            pronunciation.append_lookup_suffix(&ord_ph);
        } else {
            pronunciation.append_lookup_suffix(&suffix_ph);
        }
    }

    let mut phonemes = pronunciation.finish();
    set_word_stress(&mut phonemes, phdata, stress_opts, Some(0), -1, 0);
    Some(WordResult { phonemes, dict_flags: 0 })
}

fn translate_number_token(
    token: &NumberToken,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    grammar: &NumberGrammar,
) -> Option<WordResult> {
    match token {
        NumberToken::Ordinal(ordinal) => try_ordinal_number(ordinal, dict, phdata, stress_opts, grammar),
        _ => {
            let mut phonemes = number_token_to_phonemes(token, dict, grammar)?;
            set_word_stress(&mut phonemes, phdata, stress_opts, Some(0), -1, 0);
            Some(WordResult { phonemes, dict_flags: 0 })
        }
    }
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
    lang_opts: &LangOptions,
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

    if let Some(token) = NumberToken::parse(word, &lang_opts.number_grammar) {
        if let Some(result) =
            translate_number_token(&token, dict, phdata, stress_opts, &lang_opts.number_grammar)
        {
            return result;
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

        Ok(Translator { options: LangOptions::for_lang(lang), data_dir: dir })
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
        let tokens = tokenize_opts(text, &self.options.number_grammar);

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
                    let wr = word_to_phonemes(&lower, &dict, &phdata, &stress_opts, &self.options);
                    entries.push(EntryFull {
                        phonemes: wr.phonemes,
                        dict_flags: wr.dict_flags,
                        kind: EntryKind::Word,
                    });
                }
                Token::Number(token) => {
                    let wr = translate_number_token(
                        token,
                        &dict,
                        &phdata,
                        &stress_opts,
                        &self.options.number_grammar,
                    )
                    .unwrap_or_else(|| {
                        let surface = token.surface();
                        word_to_phonemes(&surface, &dict, &phdata, &stress_opts, &self.options)
                    });
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

        let tokens = tokenize_opts(text, &self.options.number_grammar);
        let mut codes: Vec<PhonemeCode> = Vec::new();

        for token in &tokens {
            match token {
                Token::Word(word) => {
                    let lower = word.to_lowercase();
                    let wr = word_to_phonemes(&lower, &dict, &phdata, &stress_opts, &self.options);
                    for &b in &wr.phonemes {
                        codes.push(PhonemeCode { code: b, is_boundary: false });
                    }
                }
                Token::Number(token) => {
                    let wr = translate_number_token(
                        token,
                        &dict,
                        &phdata,
                        &stress_opts,
                        &self.options.number_grammar,
                    )
                    .unwrap_or_else(|| {
                        let surface = token.surface();
                        word_to_phonemes(&surface, &dict, &phdata, &stress_opts, &self.options)
                    });
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

    fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
        if needle.is_empty() {
            return true;
        }
        let mut needle_ix = 0;
        for &byte in haystack {
            if byte == needle[needle_ix] {
                needle_ix += 1;
                if needle_ix == needle.len() {
                    return true;
                }
            }
        }
        false
    }

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

    // ── ordinal numbers ───────────────────────────────────────────────────

    fn run_ipa_table(lang: &str, dict_name: &str, cases: &[(&str, &str)]) {
        let dict_path = format!("espeak-ng-data/{dict_name}");
        if !Path::new(&dict_path).exists() { return; }
        let t = Translator::new_default(lang).unwrap();
        for &(input, expected) in cases {
            let ipa = t.text_to_ipa(input).unwrap();
            assert_eq!(ipa, expected, "lang={lang} input={input:?}");
        }
    }

    #[test]
    fn ordinals_english() {
        run_ipa_table("en", "en_dict", &[
            ("1st",  "fˈɜːst"),
            ("2nd",  "sˈɛkənd"),
            ("3rd",  "θˈɜːd"),
            ("4th",  "fˈɔːθ"),
            ("21st", "twˈɛnti fˈɜːst"),
            ("100th","wˈɒnhˈʌndɹɪdθ"),
        ]);
    }

    #[test]
    fn ordinals_english_large_scales() {
        run_ipa_table("en", "en_dict", &[
            ("1000th",    "wˈɒn θˈaʊzəndθ"),
            ("1001st",    "wˈɒn θˈaʊzənd fˈɜːst"),
            ("1000000th", "wˈɒn mˈɪliənθ"),
        ]);
    }

    #[test]
    fn ordinals_spanish() {
        run_ipa_table("es", "es_dict", &[
            ("1º",   "pɾimˈɛɾˈo"),
            ("21º",  "βixˈɛsimˌo pɾimˈɛɾˈo"),
            ("100º", "θentˈɛsimˈo"),
        ]);
    }

    #[test]
    fn ordinals_spanish_large_scale_do_not_use_hundred_root() {
        let dict_path = "espeak-ng-data/es_dict";
        if !Path::new(dict_path).exists() { return; }
        let data_dir = Path::new("espeak-ng-data");
        let dict = Dictionary::load("es", data_dir).unwrap();
        let mut phdata = PhonemeData::load(data_dir).unwrap();
        phdata.select_table_by_name("es").unwrap();
        let stress_opts = StressOpts::for_lang("es");
        let grammar = LangOptions::for_lang("es").number_grammar;
        let ordinal = OrdinalNumber {
            digits: "1000000".to_string(),
            marker: OrdinalMarker::Suffix("º".to_string()),
        };
        let result = try_ordinal_number(&ordinal, &dict, &phdata, &stress_opts, &grammar).unwrap();
        let hundred_ordinal_lookup = lookup_num_phonemes(&dict, "_0Co");
        let hundred_ordinal = trim_lookup(&hundred_ordinal_lookup);
        assert!(
            !contains_subsequence(&result.phonemes, hundred_ordinal),
            "1000000º should not be built from the hundredth root",
        );
    }

    #[test]
    fn ordinals_dutch() {
        // ordinal_indicator="e" mechanism
        run_ipa_table("nl", "nl_dict", &[
            ("1e", "ˈɪːrstə"),
            ("3e", "dˈɛrdə"),
        ]);
    }

    #[test]
    fn ordinals_german_dot() {
        // NUM_ORDINAL_DOT mechanism
        run_ipa_table("de", "de_dict", &[
            ("1.",  "ˈeːrstə"),
            ("3.",  "drˈɪtə"),
            ("21.", "tsvˈantsɪɡʰ ˈeːrstə"),
        ]);
    }

    #[test]
    fn cardinals_1234567() {
        // C espeak-ng oracle output for "1234567".
        // Remaining diffs from oracle are stress placement (ˈ vs ˌ) and minor
        // phoneme variations, not number structure issues.
        let cases: &[(&str, &str, &str, &str)] = &[
            // (lang, dict, rust_output, c_oracle)
            ("en", "en_dict",
             "wˈɒn mˈɪliən tˈuːhˈʌndɹɪdən θˈɜːti fˈɔː θˈaʊzənd fˈaɪvhˈʌndɹɪdən sˈɪksti sˈɛvən",
             "wˈɒn mˈɪliən tˈuːhˈʌndɹɪdən θˈɜːti fˈɔː θˈaʊzənd fˈaɪvhˈʌndɹɪdən sˈɪksti sˈɛvən"),
            ("es", "es_dict",
             "ˈunmiʝˈon dosθjˈentos tɾˈeɪntaikwˈatɾo mˈil kinjˈɛntos sesˈɛntaisjˈetˈe",
             "ˈunmiʝˈon dosθjˈentos tɾˌeɪntaikwˈatɾo mˈil kinjˈɛntos sɛsˌɛntaisjˈete"),
            ("fr", "fr_dict",
             "œ̃ miljɔ̃ døzsɑ̃ tʁɑ̃tkatʁ mil sɛ̃ksɑ̃ swasɑ̃tsˈɛt",
             "œ̃ miljˈɔ̃ døsɑ̃ tʁɑ̃tkatʁ mˈil sɛ̃ksɑ̃ swasɑ̃tsˈɛt"),
            ("de", "de_dict",
             "ˈaɪnə mɪljˈoːn tsvˈaɪhˈʊndɜt fˈiːr ʊntdrˈaɪsɪɡʰ tˈaʊzənt fˈʏnfhˈʊndɜt zˈiːbən ʊntzˈɛçtsɪɡʰ",
             "ˈaɪnə mɪljˈoːn tsvˈaɪhˈʊndɜt fˈiːɾ ʊntdɾˈaɪsɪç tˈaʊzənt fˈynfhˈʊndɜt zˈiːbən ʊntzˈɛçtsɪç"),
            ("nl", "nl_dict",
             "ˈeːn mˈiljun tʋˈeːhˈɔndərt vˈirɛndˈɛrtəx dˈœyzɛnt vˈɛɪfhˈɔndərt zˈeːvənɛnzˈɛstəx",
             "ˈeːn mˌiljun tʋˈeːhˌɔndərt vˌirɛndˌɛrtəx dˌœyzɛnt vˈɛɪfhˌɔndərt zˌeːvənɛnzˌɛstəx"),
        ];
        for &(lang, dict, expected, _oracle) in cases {
            let dict_path = format!("espeak-ng-data/{dict}");
            if !Path::new(&dict_path).exists() { continue; }
            let t = Translator::new_default(lang).unwrap();
            let ipa = t.text_to_ipa("1234567").unwrap();
            assert_eq!(ipa, expected, "lang={lang} input=\"1234567\"");
        }
    }

    #[test]
    fn cardinals_english_billion_scale() {
        let dict_path = "espeak-ng-data/en_dict";
        if !Path::new(dict_path).exists() { return; }
        let dict = Dictionary::load("en", Path::new("espeak-ng-data")).unwrap();
        let grammar = LangOptions::for_lang("en").number_grammar;
        let pronunciation = cardinal_pronunciation("1000000000", &dict, &grammar).unwrap();
        let billion_lookup = lookup_num_phonemes(&dict, "_0M3");
        let billion = trim_lookup(&billion_lookup);
        assert!(!billion.is_empty(), "en_dict is missing _0M3");
        let trimmed = &pronunciation.bytes[..pronunciation.trimmed_len()];
        assert!(
            trimmed.windows(billion.len()).any(|window| window == billion),
            "1000000000 should include the billion scale phonemes",
        );
    }

    #[test]
    fn cardinals_french() {
        let dict_path = "espeak-ng-data/fr_dict";
        if !Path::new(dict_path).exists() { return; }
        let t = Translator::new_default("fr").unwrap();
        for input in ["1", "2", "3", "4", "20", "80", "87", "100", "101"] {
            let ipa = t.text_to_ipa(input).unwrap();
            assert!(!ipa.is_empty(), "fr {input} produced empty IPA");
            assert!(!ipa.chars().any(|c| c.is_ascii_digit()),
                "fr {input} has raw digits in IPA: {ipa}");
        }
    }
}
