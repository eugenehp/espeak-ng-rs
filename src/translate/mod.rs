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

use crate::bytes::Bounded;
mod cardinal;
pub mod compat;
pub mod ipa_table;
pub mod phoneme_ipa;
pub mod semivowels;
pub mod ssml;

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

/// Normalise a voice / language tag to the form espeak-ng uses internally
/// (lowercase ASCII, hyphen separators).
///
/// Accepts common BCP-47-style input such as `en-us`, `en_US`, or `EN-us`.
pub fn normalize_voice_tag(s: &str) -> String {
    s.trim().replace('_', "-").to_ascii_lowercase()
}

/// Split a voice name into its base language tag and optional `+variant`
/// modifier — e.g. `en-us+f3` → (`en-us`, `Some("f3")`), `en` → (`en`, `None`).
///
/// eSpeak variant voices (`+f3`, `+whisper`, …) live in
/// `espeak-ng-data/voices/!v/` and only modify the *acoustic* voice (pitch,
/// formants, …); the base tag selects the language data (dictionary, phoneme
/// table).  Splitting here lets `-v en+f3` resolve the `en` data instead of
/// failing to find a nonexistent `en+f3_dict`.  A trailing/empty variant
/// (`en+`) yields `None`.
pub fn split_voice_variant(voice: &str) -> (&str, Option<&str>) {
    match voice.split_once('+') {
        Some((base, variant)) if !base.is_empty() => {
            (base, (!variant.is_empty()).then_some(variant))
        }
        _ => (voice, None),
    }
}

/// First subtag of a normalised BCP-47 tag (`en-gb` → `en`).
pub(crate) fn primary_bcp47_subtag(tag: &str) -> &str {
    tag.split('-').find(|part| !part.is_empty()).unwrap_or(tag)
}

/// Pick which `<stem>_dict` file to load: try the longest matching prefix of
/// `voice_tag` that exists under `data_dir`, then shorter prefixes (locale fall back
/// to language-only, e.g. `en-us` → `en_dict`).
/// The path of a compiled `<stem>_dict`, whichever layout the data directory
/// uses.
///
/// Upstream #2261 moves the dictionaries into a `dicts/` subdirectory; both
/// layouts are accepted so a data refresh that adopts it doesn't break every
/// voice at once. The flat path wins when both exist.
pub fn dict_path(data_dir: &Path, stem: &str) -> Option<std::path::PathBuf> {
    let flat = data_dir.join(format!("{stem}_dict"));
    if flat.exists() {
        return Some(flat);
    }
    let nested = data_dir.join("dicts").join(format!("{stem}_dict"));
    nested.exists().then_some(nested)
}

pub fn resolve_dict_stem(data_dir: &Path, voice_tag: &str) -> Option<String> {
    let parts: Vec<&str> = voice_tag.split('-').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    for len in (1..=parts.len()).rev() {
        let stem = parts.get(..len).unwrap_or_default().join("-");
        if dict_path(data_dir, &stem).is_some() {
            return Some(stem);
        }
    }
    // No `<tag>_dict`: a voice may borrow another language's dictionary via a
    // `dictionary <name>` directive (Norwegian Bokmål `nb` → `no`).
    let (dictionary, _) = crate::voices::voice_data_overrides(data_dir, voice_tag);
    if let Some(dict) = dictionary {
        if dict_path(data_dir, &dict).is_some() {
            return Some(dict);
        }
    }
    None
}

/// The phoneme-table name for a voice tag: the language code itself, unless the
/// voice file redirects it via a `phonemes <name>` directive (e.g. `nb` → `no`).
/// Returns the code unchanged for the common case (no directive).
pub fn resolve_phoneme_table(data_dir: &Path, voice_tag: &str) -> String {
    let (_, phonemes) = crate::voices::voice_data_overrides(data_dir, voice_tag);
    phonemes.unwrap_or_else(|| voice_tag.to_string())
}

/// Select `lang`'s phoneme table into `phdata`, trying in turn: the tag itself,
/// its `phonemes <name>` voice directive, then the **primary BCP-47 subtag**.
/// The last covers regional locales that ship only as a base table — `pt-br`→
/// `pt`, `fr-ca`→`fr`, `en-gb`→`en` — which previously errored with "phoneme
/// table '…' not found" (upstream #2248).
pub fn select_phoneme_table(phdata: &mut PhonemeData, data_dir: &Path, lang: &str) -> Result<()> {
    let resolved = resolve_phoneme_table(data_dir, lang);
    let base = primary_bcp47_subtag(lang);
    // The voice file's own `phonemes <name>` directive wins over the bare tag:
    // `pt` (Portuguese (Portugal)) declares `phonemes pt-pt`, while the table
    // literally named `pt` is the *Brazilian* one — trying the tag first gave
    // European Portuguese the Brazilian table (`dente` → `dˈẽŋtʃy`).
    for candidate in [resolved.as_str(), lang, base] {
        if phdata.select_table_by_name(candidate).is_ok() {
            return Ok(());
        }
    }
    // Re-run the base selection to surface a clear error if nothing matched.
    phdata.select_table_by_name(base).map(|_| ())
}

/// Give the phonemes of words in another language codes that name them under
/// `phdata`'s active table, so they are played with their own table's sounds.
/// A tone the clause gave such a word ([`CodeMarker::ClauseTone`]) is already
/// one of the clause's.
pub fn lend_switched_phonemes(
    codes: &mut Vec<PhonemeCode>,
    phdata: &mut PhonemeData,
    data_dir: &Path,
    lang: &str,
) {
    // A switch is a phoneme of its own in C's list, a 10 ms pause — except
    // one to the table already in use, or straight before another switch
    // (`MakePhonemeList`).
    let mut current = lang.to_string();
    let mut k = 0;
    while k < codes.len() {
        let Some(CodeMarker::LangSwitch(target)) = codes.get(k).and_then(|c| c.marker.clone())
        else {
            k += 1;
            continue;
        };
        let rest = codes.get(k + 1..).unwrap_or_default();
        let mut next = rest
            .iter()
            .filter(|c| !(c.is_boundary && c.code == PHON_END_WORD));
        let is_switch = |c: Option<&PhonemeCode>| {
            c.is_some_and(|c| matches!(c.marker, Some(CodeMarker::LangSwitch(_))))
        };
        let first = next.next();
        let before_switch = is_switch(first)
            || (first.is_some_and(|c| {
                c.code == crate::phoneme::PHON_PAUSE && c.marker.is_none() && !c.is_boundary
            }) && is_switch(next.next()));
        if target == current || before_switch {
            k += 1;
            continue;
        }
        current = target.clone();
        codes.insert(
            k + 1,
            PhonemeCode {
                code: crate::phoneme::PHON_SWITCH,
                ..Default::default()
            },
        );
        k += 2;
    }
    let mut table: Option<usize> = None;
    let mut lent: std::collections::HashMap<(usize, u8), u8> = std::collections::HashMap::new();
    for c in codes.iter_mut() {
        if let Some(CodeMarker::LangSwitch(target)) = &c.marker {
            table = (target != lang)
                .then(|| {
                    let resolved = resolve_phoneme_table(data_dir, target);
                    let found = [
                        resolved.as_str(),
                        target.as_str(),
                        primary_bcp47_subtag(target),
                    ]
                    .into_iter()
                    .find_map(|name| phdata.find_table(name).ok());
                    found
                })
                .flatten();
            continue;
        }
        let Some(t) = table else { continue };
        if c.marker.is_some() || c.is_boundary || c.code == crate::phoneme::PHON_SWITCH {
            continue;
        }
        let code = match lent.get(&(t, c.code)) {
            Some(&code) => code,
            None => {
                let code = phdata.lend_phoneme(t, c.code).unwrap_or(c.code);
                lent.insert((t, c.code), code);
                code
            }
        };
        c.code = code;
    }
}

/// The phoneme-table name a language-switch marker shows for `lang`: C prints
/// the table it selects, so `pt` closes a switch with `(pt-pt)`.
pub fn switch_marker_name(data_dir: &Path, lang: &str) -> String {
    let resolved = resolve_phoneme_table(data_dir, lang);
    let Ok(mut phdata) = PhonemeData::load(data_dir) else {
        return lang.to_string();
    };
    for name in [resolved.as_str(), lang, primary_bcp47_subtag(lang)] {
        if phdata.select_table_by_name(name).is_ok() {
            return name.to_string();
        }
    }
    lang.to_string()
}

/// Resolve an SSML `<voice>` selector (a language tag *or* a human-readable
/// voice name) to a language tag backed by data, or `None` if it can't be
/// matched (so the caller falls back to the document language rather than
/// erroring).  Mirrors the CLI `-v` resolution.
fn resolve_voice_lang(selector: &str, data_dir: &Path) -> Option<String> {
    if resolve_dict_stem(data_dir, selector).is_some() {
        return Some(selector.to_string());
    }
    let voices = crate::voices::list_voices(data_dir);
    let query = crate::voices::VoiceQuery {
        name: Some(selector.to_string()),
        ..Default::default()
    };
    crate::voices::find_voice(&voices, &query)
        .and_then(|v| resolve_dict_stem(data_dir, &v.language))
}

use crate::dictionary::file::Dictionary;
use crate::dictionary::lookup::{lookup, LookupCtx};
use crate::dictionary::rules::is_letter_wc;
use crate::dictionary::rules::translate_rules_phdata;
use crate::dictionary::stress::{
    apply_alt_stress_upgrade, apply_special_attribute2, apply_word_final_devoicing,
    change_word_stress, promote_strend_stress, set_word_stress, StressOpts,
};
use crate::dictionary::{
    FLAG_PREFIX_REMOVED, FLAG_SUFFIX_REMOVED, FLAG_SUFFIX_VOWEL, FLAG_SUFX, FLAG_SUFX_E_ADDED,
    FLAG_SUFX_S, LETTERGP_B, LETTERGP_VOWEL2, SUFX_A, SUFX_E, SUFX_I, SUFX_M, SUFX_P,
};
use crate::error::{Error, Result};
use crate::phoneme::load::PhonemeData;

use ipa_table::{
    en_ipa_override, is_pause_code, phoneme_ipa_lang, PendingStress, IPA_STRESS_PRIMARY,
    IPA_STRESS_SECONDARY, PHON_STRESS_2, PHON_STRESS_3, PHON_STRESS_D, PHON_STRESS_P,
    PHON_STRESS_P2, PHON_STRESS_PREV, PHON_STRESS_TONIC, PHON_STRESS_U,
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

/// How a clause ends — the Rust form of the `terminator` out-parameter added by
/// upstream's `espeak_TextToPhonemesWithTerminator` (1.53.0).
///
/// Callers that feed a phonemizer usually need to know whether the text they
/// just converted ended a sentence, a question, or nothing at all, which the
/// plain phoneme string does not say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClauseTerminator {
    /// No terminating punctuation (end of input).
    None,
    /// `,` `;` `:` and the other comma-class separators.
    Comma,
    /// `.` — sentence end.
    Period,
    /// `?`
    Question,
    /// `!`
    Exclamation,
}

impl ClauseTerminator {
    /// The punctuation character, or `None` when the text simply ran out.
    pub fn as_char(self) -> Option<char> {
        match self {
            ClauseTerminator::None => None,
            ClauseTerminator::Comma => Some(','),
            ClauseTerminator::Period => Some('.'),
            ClauseTerminator::Question => Some('?'),
            ClauseTerminator::Exclamation => Some('!'),
        }
    }

    /// Whether this terminator ends a *sentence* (upstream `CLAUSE_TYPE_SENTENCE`).
    pub fn is_sentence(self) -> bool {
        matches!(
            self,
            ClauseTerminator::Period | ClauseTerminator::Question | ClauseTerminator::Exclamation
        )
    }

    /// Classify a clause-terminating character.
    pub fn from_char(c: char) -> Self {
        // The same classification C makes in `clause_type_from_codepoint`, so
        // `।` and `。` end a sentence while `、` and `…` only break a clause.
        clause_punctuation_class(c).map_or(ClauseTerminator::Comma, |(class, _)| class)
    }
}

/// The terminator of the final clause in `text` — the trailing clause
/// punctuation, ignoring whitespace and closing brackets/quotes.
fn clause_terminator_of(text: &str) -> ClauseTerminator {
    for c in text.chars().rev() {
        if c.is_whitespace() || matches!(c, '"' | '\'' | ')' | ']' | '}' | '\u{201d}' | '\u{2019}')
        {
            continue;
        }
        return match c {
            '.' | '?' | '!' | ',' | ';' | ':' | '\u{3002}' | '\u{ff1f}' | '\u{ff01}'
            | '\u{ff0c}' | '\u{2026}' => ClauseTerminator::from_char(c),
            _ => ClauseTerminator::None,
        };
    }
    ClauseTerminator::None
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
    /// BCP-47 language tag, e.g. `"en"`, `"en-us"`, `"fr"`, `"de"`
    pub lang: String,
    /// Words per minute (default 175)
    pub rate: u32,
    /// Base pitch (0–100, default 50)
    pub pitch: u32,
    /// Word gap in units of 10ms (`-g`).
    pub word_gap: i32,
    /// Language's own inter-word pause phoneme (upstream `langopts.word_gap & 7`,
    /// indexing `pause_phonemes[]`): `1` = `_|` (no-link), used by cmn/yue/zh so a
    /// syllable's final vowel isn't merged with the next word's consonant.  `0`
    /// (the default) inserts nothing.
    pub lang_word_gap: u8,
    /// Stress rule index (STRESSPOSN_XXX from translate.h)
    pub stress_rule: u8,
    /// Capital-letter indication (`-k`): 0 = none, 1 = a short capital *sound*
    /// (the `phonCAPITAL` phoneme, doubled for all-caps words), 2 = announce the
    /// word "capital".
    pub capitals: u8,
    /// Announce punctuation by name (`--punct`): `None` = off (default),
    /// `Some(empty)` = all punctuation, `Some(chars)` = only those characters.
    pub punct: Option<Vec<char>>,
    /// Language-specific number parsing and rendering behavior.
    pub number_grammar: NumberGrammar,
    /// Active dict-condition bitmask from the voice's `dictrules` directive —
    /// selects `?n` variant dict/rule groups (e.g. Brazilian Portuguese `?2`).
    pub dict_condition: u32,
    /// Reduce unstressed vowels even when the pronunciation was given in the
    /// dictionary — upstream `LOPT_REDUCE & 1`, set only for Italian.
    ///
    /// C's `StressCondition` uses this to refuse `ChangeIfNotStressed` on a
    /// dictionary-given word ("don't change phonemes which are given for the
    /// word in the dictionary").  Wiring that gate to this port's notion of
    /// "came from the dictionary" made Russian *lose* its reduction while
    /// Spanish kept its allophone, so the flag is parsed and carried but the
    /// gate is not applied — see GAPS.md for the measurement.
    pub reduce_dictionary_vowels: bool,
    /// The dictionary's default entry kind is **replacement text**, not phonemes,
    /// so `FLAG_TEXTMODE`'s meaning is reversed (upstream `langopts.textmode`,
    /// set for cmn/yue/zh: "the meaning of FLAG_TEXTMODE is reversed (to save
    /// data when *_list file is compiled)").  Chinese entries map a character to
    /// its pinyin reading — `你` → `ni3` — and the flag marks the *exceptions*
    /// that really are phonemes.  Reading the bytes as phoneme codes made every
    /// Han character gibberish (`你` → `qr`).
    pub reversed_textmode: bool,
    /// `--tie=<char>`: the character to insert inside a multi-letter phoneme
    /// name in IPA output (`tʃ` → `t͡ʃ`).  `None` = none, as by default.
    pub ipa_tie: Option<char>,
    /// `--sep=<char>`: the character to write *between* phonemes in IPA output.
    /// Upstream shares one slot with [`Self::ipa_tie`], so only one is ever set.
    pub ipa_separator: Option<char>,
    /// `langopts.param[LOPT_BRACKET_PAUSE]` / `[LOPT_BRACKET_PAUSE_ANNOUNCED]`:
    /// how long a bracket or quote pauses.  C's defaults are 4 and 2, and its
    /// pre-pause loop spends 2 per `phonPAUSE`, so 4 means two pauses.  Read
    /// from the voice file's `brackets` / `bracketsAnnounced` (Hungarian sets
    /// both to 0 — `hu "a ( b"` has no pause at all).
    pub bracket_pause: (u8, u8),
    /// `langopts.max_initial_consonants` — how many consonants may precede the
    /// first vowel before a word counts as unpronounceable and is spelled out
    /// letter by letter ("BBC" → `b,i:b,i:s'i:`).
    pub max_initial_consonants: usize,
    /// `langopts.param[LOPT_UNPRONOUNCABLE]`: `1` disables the test entirely,
    /// `2` delegates it to the language's own rules (C's `Unpronouncable2` — a
    /// word the rules can't read, or that a `$unpron` rule marks, is spelled),
    /// and any other value is a letter code to discount when counting.
    pub unpronouncable: u32,
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
    /// Digits attached to a word are **tone numbers**, not a separate number:
    /// the Chinese dictionaries store readings as pinyin + tone (`你` → `ni3`),
    /// and splitting that into "ni" + the number 3 made every Han character read
    /// as a word plus "three" (`你` → `qr` … `sˈan`).
    pub tone_numbers: bool,
    /// Read the fractional digits as **one number** rather than digit by digit —
    /// upstream `NUM_DFRACTION_2` (pl/cs/fi/tr/pt/…: "3,14" is "trzy przecinek
    /// **czternaście**") and `NUM_DFRACTION_4` (es/fr/lt/vi/…, same rule with a
    /// longer limit).  Leading zeros are still spoken one at a time, and a
    /// fraction longer than the limit falls back to digit-by-digit.  `0` = off.
    pub fraction_digits_as_number: u8,
    /// Read a decimal fraction as a *fraction*: numerator + a "tenths / hundredths
    /// / thousandths" suffix, instead of digit-by-digit.  Upstream
    /// `NUM_DFRACTION_5` (ru, hu, ka), driven by the `_0Z<n>` / `_0Z<n>s`
    /// dictionary entries — `ru "0,25"` is "ноль и двадцать пять сотых", not
    /// "ноль и два пять".
    pub fraction_suffix: bool,
    /// The fraction numerator uses feminine forms of 1 and 2 (`_1f`, `_2f`) and
    /// takes the singular suffix when it ends in 1 (but not 11) — upstream
    /// `NUM2_FRACTION_FEMININE`: "одна сотая", "две сотых", "двадцать одна сотая".
    pub fraction_feminine: bool,
    /// Digit-group (thousands) separator, if the language groups large numbers —
    /// e.g. `Some(',')` for English so `1,000` reads as one number.  `None` (the
    /// default) leaves grouping characters as ordinary punctuation.
    pub group_separator: Option<char>,
    /// Decimal-point character — `'.'` in English, `','` in most of continental
    /// Europe (`3,14`).  Must differ from `group_separator`.
    pub decimal_separator: char,
    /// Vigesimal 70s/90s (France French): 70–79 = "soixante" + 10–19,
    /// 90–99 = "quatre-vingt" + 10–19 (there is no simple word for 70 or 90).
    pub vigesimal_70_90: bool,
    /// Whether a space between digit groups marks thousands (`1 234 567`) — the
    /// SI/ISO standard used across continental Europe.  Enabled for the
    /// comma-decimal languages (where a space is unambiguous); English keeps a
    /// space as an ordinary separator.
    pub space_group: bool,
    /// Generate Portuguese cardinals as text and pronounce them via the rules
    /// (the pt dictionary is missing most number keys — `1`/`5`/`100`/`1000` come
    /// out empty or wrong).  Set for the `pt` voice.
    pub portuguese_cardinals: bool,
    /// C's `NUM_1900`: read 1900–1999 as "nineteen hundred (and) …".  Only ten
    /// languages ask for it (af, da, en, fo, is, kl, nb/no, nl, sv); the rest
    /// say "one thousand nine hundred …".
    pub year_1900: bool,
    /// Split a word at a lowercase→uppercase boundary (`CamelCase` → "Camel
    /// Case", `iPhone` → "i Phone"), mirroring espeak's `TranslateClause`
    /// behaviour (a space is inserted before the uppercase letter).  Universal
    /// in espeak; only Irish (`ga`), whose lenition prefixes legitimately mix
    /// case, disables it — so this is `true` by default and `false` for `ga`.
    pub caps_word_split: bool,
    /// C's `LOPT_APOSTROPHE` (also the voice files' `apostrophe` directive):
    /// bit 0 keeps an apostrophe that *starts* a word, bit 1 one that *ends* it.
    /// Only Hebrew sets it (bit 1, for `בָּגָאז׳`).  Everywhere else an
    /// apostrophe belongs to the word only *between* two letters, which is what
    /// keeps `don't` whole while `hello'` is just "hello".
    pub apostrophe: u8,
    /// Catalan keeps a middle dot between two letters inside the word
    /// (`col·legi`): C's `punct_within_word` for `ca`.
    pub middle_dot_in_word: bool,
    /// Treat a capital letter as a *stress marker* rather than a word break
    /// (espeak `LOPT_CAPS_IN_WORD`).  The first capital in a word inserts a
    /// primary-stress mark `ˈ` before it and the word stays whole (`bAstu` →
    /// "bˈastu").  Mutually exclusive with `caps_word_split`; set only for
    /// Lojban (`jbo`), where capitals denote the stressed syllable.
    pub caps_mark_stress: bool,
    /// Dutch (`nl`) exception to the upper→lower CamelCase split: a word-initial
    /// "IJ" digraph (`IJssel`) is not split.  Mirrors espeak's
    /// `translator_name == L('n','l')` guard.
    pub dutch_ij: bool,
    /// Combining ("apocopated") form of the unit *one* used inside a compound —
    /// German "einundzwanzig" uses "ein", not the standalone "eins".  Looked up
    /// as a dictionary word; `None` (the default) keeps the plain `_1` unit.
    pub combining_one: Option<String>,
    /// Italian tens-vowel elision: a tens word (trenta/quaranta/…) drops its
    /// final vowel before a vowel-initial unit (uno/otto) → "trentuno",
    /// "quarantotto".  The 20s already elide via dedicated `_21`/`_28` entries.
    pub elide_tens_vowel: bool,
    /// Slavic scale-word declension (gender + case agreement on "thousand",
    /// "million", …).  See `SlavicThousands`.
    pub thousands_variant: SlavicThousands,
    /// C `NUM_SINGLE_STRESS`: only the last part of a tens+units compound keeps
    /// its primary stress (Spanish "treintaicuatro", Turkish "yirmibir").
    pub single_stress: bool,
    /// Number symbols (`²`, `½`) the dictionary names itself.
    pub dict_symbols: Vec<char>,
    /// The suffixes after a number that the dictionary has a `_#suffix`
    /// entry for, which make it an ordinal (C's `TranslateNumber_1`).
    pub ordinal_suffixes: Vec<String>,
    /// Read a `²`/`³` the dictionary doesn't name as its digit, not "squared".
    pub powers_as_digits: bool,
    /// C's number options for the language.
    flags: Option<cardinal::NumberFlags>,
    /// The language has no number options: every number is read digit by digit.
    pub digits_only: bool,
    /// C `NUM2_THOUSANDPLEX_VAR_*`: the scales whose count is feminine.
    pub feminine_scales: &'static [u8],
    /// C `NUM_AND_HUNDRED`: a conjunction joins a higher scale to the hundreds
    /// (Kurdish "hezar Û nehsed").
    pub and_hundred: bool,
    /// The longest number read as a quantity (C `max_digits`); a longer one is
    /// read digit by digit.
    pub max_digits: usize,
    /// C `break_numbers`: bit `n` starts a new group where `n` digits remain.
    pub break_numbers: u32,
    /// Scales go up in lakhs: the groups of `break_numbers`, not thousands.
    pub lakh: bool,
}

/// Slavic scale-word (thousand/million/…) declension.  The scale word takes a
/// different case by the count before it, and the count's "one"/"two" agree in
/// gender with the scale word — Russian "одна тысяча" (1000, fem), "две тысячи"
/// (2000, fem gen-sg), "пять тысяч" (5000, gen-pl), "один миллион" (masc).
/// (espeak's `M_Variant` / `NUM2_THOUSANDS_VAR*`.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SlavicThousands {
    /// No Slavic declension.
    #[default]
    None,
    /// Russian: scale word `_1MA{n}` for count≡1, `_0MA{n}` for 2–4, `_0M{n}`
    /// otherwise (mod 10, excluding the 11–19 teens); "thousand" is feminine, the
    /// higher scales masculine; the "one" is spoken (not omitted).
    Ru,
    /// Czech: `_0MA{n}` only for count exactly 2–4, else `_0M{n}`; no gender; the
    /// "one" is omitted ("tisíc", "dva tisíce", "pět tisíc").
    Cs,
    /// Polish: `_0MA{n}` for count ≡ 2–4 (mod 10, not the 11–19 teens), else
    /// `_0M{n}`; no gender; the "one" is omitted ("tysiąc", "dwa tysiące").
    Pl,
    /// Lithuanian: `_0MB{n}` (genitive plural "tūkstančių") for a teen count or
    /// count ≡ 0 mod 10, `_0MA{n}` ("tūkstantis") for count ≡ 1 mod 10, else
    /// `_0M{n}` ("tūkstančiai"); no gender; count 1 uses `_1M{n}`.
    Lt,
    /// Slovak: same case rule as Czech (`_0MA{n}` for count exactly 2–4), but
    /// "thousand" is feminine only for a bare single-digit count ("dve tisíc" at
    /// 2000, yet "dvadsať dva tisíc" at 22000 and "dva milióny" for millions).
    Sk,
    /// Croatian: `_1M{n}` (the count-1 word "tisuću") for count ≡ 1 mod 10,
    /// `_0MA{n}` ("tisuće") for count ≡ 2–4, else `_0M{n}` ("tisuća"); "thousand"
    /// is feminine (dvije) for any count, like Russian.
    Hr,
}

/// Ordinal marker recognition behavior.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OrdinalGrammar {
    /// Suffix which marks ordinals even without a `_#suffix` dict entry.
    pub indicator: Option<String>,
    /// Whether `3.`-style ordinals are accepted.
    pub dot_marks_ordinal: bool,
    /// French letter-suffix ordinals (`1er`, `1re`, `2e`, `2nd`, `3ème`, …).
    /// These carry gender/second-vs-deuxième information the renderer uses to
    /// generate the ordinal word (`premier`/`première`/`deuxième`/`second`).
    pub french: bool,
    /// Italian ordinals: 1–10 are irregular (`primo`…`decimo`) and absent from
    /// the dict, so they're generated; 11+ use the regular `-esimo` rule.
    pub italian: bool,
    /// Germanic compound ordinals: a compound like 21 forms its ordinal as the
    /// whole *cardinal* + the ordinal suffix ("einundzwanzig" + "ste" =
    /// "einundzwanzigste"), not tens-ordinal + units-ordinal ("zwanzig erste").
    pub compound_cardinal_suffix: bool,
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

/// C `BREAK_THOUSANDS`: a break every three digits.
const BREAK_THOUSANDS: u32 = 0x4924_9248;

/// Hundreds-specific rendering behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HundredsGrammar {
    /// Whether to insert a conjunction between the hundreds and remainder,
    /// unconditionally (English "one hundred AND thirty-four").
    pub use_conjunction_with_remainder: bool,
    /// Insert the conjunction between hundreds and remainder only when the
    /// remainder is a *single word* — a unit, a single-word teen, or a round ten
    /// — but not a compound that already carries its own conjunction.  Afrikaans
    /// "honderd EN een" / "honderd EN twintig" but "honderd vier-en-dertig";
    /// Swahili "mia moja NA moja" but "mia moja thelethini na nne".
    pub conjunction_before_simple_remainder: bool,
    /// Whether to omit the explicit `one` before `hundred` (in the compose path,
    /// `_{n}` + `_0C`): French "cent", not "un cent".
    pub omit_one_prefix: bool,
    /// For a bare 1-hundred, skip the dedicated `_1C`/`_1C0` word (which bakes in
    /// the "one") and use the bare hundred word instead — `_0C0` for an exact
    /// hundred, else `_0C`.  Malayalam "100" = "നൂറ്" (`_0C0`), not "ഒരു നൂറ്"
    /// (`_1C0`); "200"+ keep their dedicated `_2C0`/`_2C` forms.  (espeak's
    /// `NUM2_OMIT_1_HUNDRED_ONLY`.)
    pub omit_one_hundred_word: bool,
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
        grammar.flags = cardinal::NumberFlags::for_lang(lang);
        grammar.digits_only = matches!(
            primary_bcp47_subtag(lang),
            "jbo" | "mi" | "my" | "piqd" | "py" | "qu" | "quc" | "th" | "uz" | "xex"
        );
        let lang = primary_bcp47_subtag(lang);
        match lang {
            "en" => {
                grammar.hundreds.use_conjunction_with_remainder = true;
                grammar.group_separator = Some(','); // 1,000 → one thousand
            }
            "es" => {
                grammar.tens = TensGrammar::WithConjunction;
                grammar.hundreds.omit_one_prefix = true;
                grammar.thousands.omit_one_prefix = true;
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
            }
            "fr" => {
                grammar.ordinals.french = true; // 1er, 2e, 2nd, 3ème …
                grammar.hundreds.omit_one_prefix = true;
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
                grammar.vigesimal_70_90 = true; // soixante-dix, quatre-vingt-dix
            }
            "it" => {
                // "cento", not "uno cento" (1000 is already "mille").
                grammar.hundreds.omit_one_prefix = true;
                grammar.ordinals.italian = true; // 1º=primo … 10º=decimo
                grammar.elide_tens_vowel = true; // "trentuno", "quarantotto"
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
            }
            "de" => {
                grammar.ordinals.dot_marks_ordinal = true;
                grammar.ordinals.compound_cardinal_suffix = true;
                grammar.tens = TensGrammar::UnitsThenConjunction;
                // "einundzwanzig", not "einsundzwanzig" (standalone 1 = "eins").
                grammar.combining_one = Some("_1a".to_string());
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
                // German keeps the "ein": upstream reads 100 as "einhundert" and
                // 1000 as "ein tausend" (its `langopts.numbers` sets neither
                // `NUM_OMIT_1_HUNDRED` nor `NUM_OMIT_1_THOUSAND`).
            }
            // Chinese / Cantonese / Hakka: dictionary readings are pinyin with a
            // trailing tone digit, so digits glued to letters stay in the word.
            "cmn" | "yue" | "hak" | "zh" => {
                grammar.tone_numbers = true;
            }
            // (the reversed-textmode flag for these is set on LangOptions below)
            // Russian: "сто"/"тысяча", not "один сто"/"один тысяча".
            "ru" | "bg" | "uk" | "be" => {
                grammar.hundreds.omit_one_prefix = true;
                // (Ukrainian sets no such flag: "один тисяча".)
                grammar.thousands.omit_one_prefix = lang != "uk";
                grammar.decimal_separator = ',';
                if lang == "ru" {
                    // Decimal fractions are read as fractions with feminine
                    // numerators — "0,25" = "ноль и двадцать пять сотых"
                    // (upstream NUM_DFRACTION_5 | NUM2_FRACTION_FEMININE).
                    grammar.fraction_suffix = true;
                    grammar.fraction_feminine = true;
                    // Russian scale words decline (gender + case): "одна тысяча",
                    // "две тысячи", "пять тысяч", "один миллион".  The "one" is
                    // spoken (unlike bg/uk/be), so undo the omit set above.
                    grammar.thousands_variant = SlavicThousands::Ru;
                    grammar.thousands.omit_one_prefix = false;
                }
            }
            // Turkish/Hungarian: "yüz"/"bin", "száz"/"ezer" — no "one" prefix.
            "tr" | "hu" => {
                grammar.hundreds.omit_one_prefix = true;
                grammar.thousands.omit_one_prefix = true;
                grammar.decimal_separator = ',';
                // Hungarian also always names the fraction ("tizedes"), without
                // the feminine numerator forms Russian uses.
                grammar.fraction_suffix = lang == "hu";
            }
            // Slovak/Croatian/Serbian: "sto" not "jedensto" (1000 already "tisíc"/
            // "tisuću"/"hiljadu").
            "sr" => {
                grammar.hundreds.omit_one_prefix = true;
                grammar.decimal_separator = ',';
            }
            // Croatian: the scale word declines — "tisuću" (…1), "tisuće" (…2–4),
            // "tisuća" (5+) — and "thousand" is feminine ("dvije tisuće").
            "hr" => {
                grammar.hundreds.omit_one_prefix = true;
                grammar.decimal_separator = ',';
                grammar.thousands_variant = SlavicThousands::Hr;
            }
            // Slovak: the scale word declines for count 2–4 ("dva milióny"), and
            // "thousand" is feminine for a bare 2 ("dve tisíc").
            "sk" => {
                grammar.hundreds.omit_one_prefix = true;
                grammar.decimal_separator = ',';
                grammar.thousands_variant = SlavicThousands::Sk;
            }
            // Macedonian: "сто"/"илјада", not "еден сто"/"еден илјада".
            "mk" => {
                grammar.hundreds.omit_one_prefix = true;
                grammar.thousands.omit_one_prefix = true;
                grammar.decimal_separator = ',';
            }
            // Arabic/Persian/Urdu: comma groups thousands (the native `٬` is
            // normalised to `,`; the native `٫` decimal maps to `.`).
            "ar" | "fa" | "ur" => {
                grammar.group_separator = Some(',');
            }
            "nl" | "mt" => {
                grammar.ordinals.dot_marks_ordinal = true;
                grammar.ordinals.indicator = Some("e".to_string());
                // Dutch, like German, suffixes the whole cardinal for compound
                // ordinals ("eenentwintigste"); Maltese ordinals differ, so it's
                // Dutch-only.
                grammar.ordinals.compound_cardinal_suffix = lang == "nl";
                grammar.tens = TensGrammar::UnitsThenConjunction;
                grammar.hundreds.omit_one_prefix = true;
                grammar.thousands.omit_one_prefix = true;
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
            }
            // Portuguese: comma decimal / period thousands like the rest of the
            // Romance set.  (`portuguese_cardinals` spelled the numbers out as
            // text because the dict looked key-less; it is not — pt's `_N`
            // entries are behind `?1`/`?2` conditions, which the port now
            // honours, so the generic machinery reads them.)
            // `NUM_AND_UNITS | NUM_HUNDRED_AND`: the conjunction "e" joins
            // tens→units ("vinte E um") and hundreds→remainder ("cento E um"),
            // and `NUM_DFRACTION_2` reads two fraction digits as one number.
            "pt" => {
                grammar.portuguese_cardinals = false;
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
                grammar.tens = TensGrammar::WithConjunction;
                grammar.hundreds.use_conjunction_with_remainder = true;
                grammar.fraction_digits_as_number = 2;
            }
            // Continental-European comma-decimal languages.
            // Units-first cardinals: Danish/Faroese "enogtyve", Slovenian
            // "enaindvajset" (units + "og"/"in" + tens), like German — and their
            // compound ordinals suffix the whole cardinal ("enogtyvende").
            "da" | "fo" | "kl" | "sl" => {
                grammar.ordinals.dot_marks_ordinal = true;
                grammar.tens = TensGrammar::UnitsThenConjunction;
                grammar.ordinals.compound_cardinal_suffix = true;
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
                // C `NUM_HUNDRED_AND` — "hundrede OG en", "tusind OG en".
                // Slovenian is the one of the four without it.
                grammar.hundreds.use_conjunction_with_remainder = lang != "sl";
                // C `NUM_OMIT_1_HUNDRED` for all but Danish.
                grammar.hundreds.omit_one_prefix = lang != "da";
            }
            // Tens-first, dot-ordinal, comma-decimal (Finnish, Estonian,
            // Lithuanian, Norwegian "tjueen", Greenlandic).
            "et" | "fi" | "nb" | "no" => {
                grammar.ordinals.dot_marks_ordinal = true;
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
                // Norwegian has C's `NUM_HUNDRED_AND` ("nitten hundre OG …");
                // Finnish and Estonian have no conjunction at all.
                grammar.hundreds.use_conjunction_with_remainder = matches!(lang, "nb" | "no");
            }
            // Lithuanian: like the above, plus the scale word declines —
            // "tūkstantis" (1/…1), "tūkstančiai" (…2–9), "tūkstančių" (teens/×10).
            "lt" => {
                grammar.ordinals.dot_marks_ordinal = true;
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
                grammar.thousands_variant = SlavicThousands::Lt;
            }
            // More comma-decimal languages (period/space thousands).  Only the
            // decimal + group separators are set here — the ordinal-dot marker
            // varies (Swedish "3:e", Romanian "al 3-lea"…), so it's left off.
            // Romanian & Icelandic: a conjunction joins tens and units
            // ("treizeci ȘI patru", "þrjátíu OG fjórir") via the dict `_0and`.
            "ro" | "is" => {
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
                grammar.tens = TensGrammar::WithConjunction;
                // Icelandic also joins hundreds→remainder, but only when the
                // remainder is a single word (`NUM_SINGLE_AND | NUM_HUNDRED_AND`):
                // "hundrað OG einn", but "nítján hundruð áttatíu og fjórir".
                grammar.hundreds.conjunction_before_simple_remainder = lang == "is";
            }
            // Albanian: the conjunction "e" joins both tens→unit and
            // hundreds→remainder ("njëzet E një", "njëqind E tridhjetë").
            "sq" => {
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
                grammar.tens = TensGrammar::WithConjunction;
                grammar.hundreds.use_conjunction_with_remainder = true;
            }
            // Armenian: "100" is "harjur", not "mek harjur".
            "hy" => {
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
                grammar.hundreds.omit_one_prefix = true;
            }
            // Kazakh: "100" is "жүз", not "бір жүз" (but "1000" keeps "бір мың").
            // Kazakh keeps the period decimal separator ("3,14" is two numbers).
            "kk" => {
                grammar.hundreds.omit_one_prefix = true;
            }
            // Azerbaijani: "100"/"1000" are "yüz"/"min", not "bir yüz"/"bir min".
            "az" => {
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
                grammar.hundreds.omit_one_prefix = true;
                grammar.thousands.omit_one_prefix = true;
            }
            // Afrikaans: Germanic units-first tens ("vier-en-dertig"), like Dutch,
            // but keeps "een honderd" / "een duisend" (no omit-one prefix) and puts
            // "en" before a single-word remainder ("een honderd en een/twintig").
            // (Afrikaans reads "3,14" as two numbers — its decimal separator is
            // the period, like English: `af "3.14"` → "drie punt een vier".)
            "af" => {
                grammar.tens = TensGrammar::UnitsThenConjunction;
                grammar.hundreds.conjunction_before_simple_remainder = true;
            }
            // Czech: the scale word declines for count exactly 2–4 ("dva tisíce",
            // "dva milióny"); the "one" word (`_1M1`) is spoken for 1000.
            "cs" => {
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
                grammar.thousands_variant = SlavicThousands::Cs;
            }
            // Polish: the scale word declines for count ≡ 2–4 mod 10 ("dwa
            // tysiące"), else the genitive-plural form ("pięć tysięcy").
            "pl" => {
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
                grammar.thousands_variant = SlavicThousands::Pl;
            }
            // (Georgian is *not* here: it reads "3,14" as two numbers and uses the
            // period as its decimal separator, as upstream does.)
            "sv" | "el" | "lv" | "ca" | "eu" | "bs" | "id" | "vi" => {
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
            }
            // Welsh: "100" is "cant", not "un cant" (period decimal, unchanged).
            "cy" => {
                grammar.hundreds.omit_one_prefix = true;
            }
            // Amharic: "100" is "መቶ", not "አንድ መቶ" (but "1000" keeps "አንድ ሺ").
            "am" => {
                grammar.hundreds.omit_one_prefix = true;
            }
            // Swahili: "na" joins tens and units ("ishirini na moja"), and also
            // hundreds→remainder when the remainder is a single word ("mia moja
            // na moja/kumi/ishirini", but "mia moja thelethini na nne").
            "sw" => {
                grammar.tens = TensGrammar::WithConjunction;
                grammar.hundreds.conjunction_before_simple_remainder = true;
            }
            // Malayalam: "100" is the bare "നൂറ്" (not "ഒരു നൂറ്"); 200+ keep
            // their dedicated words.  "1000" is the bare "ആയിരം" (no "ഒന്ന്").
            "ml" => {
                grammar.hundreds.omit_one_hundred_word = true;
                grammar.thousands.omit_one_prefix = true;
            }
            // Tamil / Sinhala: "1000" is "ஆயிரம்" / "අහස", not "ஒன்று ஆயிரம்" /
            // "එක අහස" (the "one" is dropped before "thousand").
            "ta" | "si" => {
                grammar.thousands.omit_one_prefix = true;
            }
            _ => {}
        }
        // Space (SI/ISO) thousands grouping is unambiguous exactly where the
        // decimal point is a comma — all the continental-European languages.
        // English (comma-group/period-decimal) and Arabic-style (comma-group)
        // keep a space as an ordinary separator.
        grammar.space_group = grammar.decimal_separator == ',';
        // Irish keeps lenition prefixes ("tAthair", "nGael") whole — the only
        // language whose `UpperCaseInWord` suppresses the CamelCase split.
        grammar.caps_word_split = lang != "ga";
        // Lojban writes the stressed syllable with capitals (`LOPT_CAPS_IN_WORD`)
        // — a capital marks stress instead of splitting the word.
        grammar.caps_mark_stress = lang == "jbo";
        // Dutch keeps a word-initial "IJ" digraph intact under the upper→lower
        // split.
        grammar.dutch_ij = lang == "nl";
        grammar.fraction_digits_as_number = match primary_bcp47_subtag(lang) {
            // NUM_DFRACTION_2 — up to two digits read as a number.
            "et" | "fi" | "he" | "hr" | "bs" | "sr" | "mk" | "pl" | "pt" | "sk" | "cs" | "sl"
            | "smj" | "tr" | "az" => 2,
            // NUM_DFRACTION_4 — the same, up to five digits.
            "es" | "an" | "ca" | "ia" | "pap" | "fr" | "ht" | "lt" | "lv" | "ltg" | "sq" | "tt"
            | "vi" => 5,
            _ => 0,
        };
        // C `NUM_HUNDRED_AND` for the languages whose arm above doesn't set it:
        // a conjunction joins hundreds→remainder ("mi'a WA wahid", "sto I edno").
        // `NUM_SINGLE_AND` (Bulgarian, Hebrew) restricts it to a remainder that
        // is a single word, as it does for Icelandic.
        if matches!(lang, "ar" | "eu" | "fa" | "io" | "ku" | "om" | "tn") {
            grammar.hundreds.use_conjunction_with_remainder = true;
        }
        if matches!(lang, "bg" | "he") {
            grammar.hundreds.conjunction_before_simple_remainder = true;
        }
        // C `NUM_AND_UNITS`: a conjunction joins tens→units ("si O chahar",
        // "shloshim VE arba").  Arabic also has `NUM_SWAP_TENS`, so its units
        // come first ("arba'a WA thalathun").
        if matches!(lang, "bg" | "fa" | "he" | "io" | "ka" | "ku" | "mk" | "tn") {
            grammar.tens = TensGrammar::WithConjunction;
        }
        if lang == "ar" {
            grammar.tens = TensGrammar::UnitsThenConjunction;
        }
        grammar.single_stress = matches!(
            primary_bcp47_subtag(lang),
            "an" | "az"
                | "bs"
                | "ca"
                | "el"
                | "es"
                | "eu"
                | "fr"
                | "hr"
                | "ht"
                | "ia"
                | "pap"
                | "sr"
                | "sv"
                | "tr"
                | "tt"
        );
        grammar.feminine_scales = match primary_bcp47_subtag(lang) {
            "ro" => &[1, 2, 3],
            "mk" => &[1, 3],
            "el" | "grc" | "is" => &[1],
            _ => &[],
        };
        grammar.and_hundred = matches!(lang, "ar" | "ku");
        // `langopts.max_digits`, transcribed from `tr_languages.c`.
        grammar.max_digits = match primary_bcp47_subtag(lang) {
            "en" | "lv" => 33,
            "ka" | "ru" | "uk" => 32,
            "ne" => 22,
            "ko" => 20,
            _ => 14,
        };
        grammar.lakh = matches!(
            primary_bcp47_subtag(lang),
            "as" | "bn"
                | "bpy"
                | "gu"
                | "hi"
                | "kn"
                | "ml"
                | "mr"
                | "ne"
                | "or"
                | "pa"
                | "sd"
                | "si"
                | "ta"
                | "te"
                | "ur"
        );
        // `langopts.break_numbers`: where a long number splits into groups.
        grammar.break_numbers = match primary_bcp47_subtag(lang) {
            "ko" => 0x1111_1110,
            "ne" => 0xaaaa_aaa8,
            "bn" | "as" | "bpy" => 0x2492_4aa8,
            "ta" | "kn" | "ml" | "te" => 0x0000_14a8,
            "hi" | "or" | "pa" | "gu" | "mr" | "si" => 0x0001_4aa8,
            "ur" | "sd" => 0x0000_52a8,
            "vi" | "yue" => 0x0000_0018,
            _ => BREAK_THOUSANDS,
        };
        // C `LOPT_APOSTROPHE`: Hebrew keeps a word-final apostrophe.
        grammar.apostrophe = if lang == "he" { 2 } else { 0 };
        grammar.middle_dot_in_word = lang == "ca";
        // C `NUM_1900`: 1900–1999 read as "nineteen hundred …".
        grammar.year_1900 = matches!(
            lang,
            "af" | "da" | "en" | "fo" | "is" | "kl" | "nb" | "nl" | "no" | "sv"
        );
        // C's `NUM_ORDINAL_DOT`, and `NUM_DECIMAL_COMMA` (`ProcessLanguageOptions`).
        if let Some(flags) = grammar.flags {
            grammar.ordinals.dot_marks_ordinal = flags.numbers() & 0x1_0000 != 0;
            if flags.numbers() & 0x8 != 0 && grammar.decimal_separator != ',' {
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
            }
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
            tone_numbers: false,
            fraction_digits_as_number: 0,
            fraction_suffix: false,
            fraction_feminine: false,
            group_separator: None,
            decimal_separator: '.',
            vigesimal_70_90: false,
            space_group: false,
            portuguese_cardinals: false,
            year_1900: false,
            apostrophe: 0,
            middle_dot_in_word: false,
            caps_word_split: true,
            caps_mark_stress: false,
            dutch_ij: false,
            combining_one: None,
            elide_tens_vowel: false,
            thousands_variant: SlavicThousands::None,
            single_stress: false,
            dict_symbols: Vec::new(),
            ordinal_suffixes: Vec::new(),
            powers_as_digits: false,
            flags: None,
            digits_only: false,
            feminine_scales: &[],
            and_hundred: false,
            max_digits: 14,
            break_numbers: BREAK_THOUSANDS,
            lakh: false,
        }
    }
}

impl Default for LangOptions {
    fn default() -> Self {
        LangOptions {
            lang: "en".to_string(),
            rate: 175,
            pitch: 50,
            word_gap: 0,
            lang_word_gap: 0,
            stress_rule: 2, // STRESSPOSN_2R = penultimate
            capitals: 0,
            punct: None,
            number_grammar: NumberGrammar::default(),
            dict_condition: 0,
            reduce_dictionary_vowels: false,
            reversed_textmode: false,
            bracket_pause: (4, 2),
            ipa_tie: None,
            ipa_separator: None,
            max_initial_consonants: 3,
            unpronouncable: 0,
        }
    }
}

/// Whether `--punct` is active and covers character `c`.  An empty set means
/// "all punctuation" (`--punct` with no argument); the SSML clause-break
/// sentinel is never announced.
fn punct_covers(opts: &LangOptions, c: char) -> bool {
    if c == ssml::SSML_BREAK {
        return false;
    }
    match &opts.punct {
        Some(chars) => chars.is_empty() || chars.contains(&c),
        None => false,
    }
}

impl LangOptions {
    pub fn for_lang(lang: &str) -> Self {
        let lang = normalize_voice_tag(lang);
        // cmn/yue/zh compile their dictionaries as replacement *text* (hanzi →
        // pinyin), so `FLAG_TEXTMODE` marks the phoneme entries instead
        // (upstream `langopts.textmode`).
        let reversed_textmode = matches!(primary_bcp47_subtag(&lang), "cmn" | "yue" | "zh");
        // `LOPT_UNPRONOUNCABLE`, transcribed from `tr_languages.c`.
        let unpronouncable = match primary_bcp47_subtag(&lang) {
            "am" | "ar" | "be" | "chr" | "el" | "grc" | "fa" | "ko" | "si" | "ur" | "sd"
            | "cmn" | "yue" | "zh" => 1,
            // (Of the Spanish family, only Spanish itself: Catalan,
            // Interlingua, Aragonese and Papiamento keep the default.)
            "de" | "en" | "es" => 2,
            "ga" | "gd" => 3,
            "bg" => 0x432,
            "sl" => 0x76,
            // The default disregards an initial "s" (Dutch "schrompel").
            _ => u32::from(b's'),
        };
        let max_initial_consonants = match primary_bcp47_subtag(&lang) {
            "az" | "kk" | "ku" | "tr" => 2,
            "sw" | "tn" => 4,
            "bs" | "cmn" | "cs" | "hr" | "sk" | "sr" | "yue" | "zh" => 5,
            "hy" => 6,
            "ka" | "pl" => 7,
            _ => 3,
        };
        // `langopts.word_gap & 7` — the language's own inter-word pause phoneme
        // (upstream `pause_phonemes[]`): `0x21` for Chinese and Vietnamese
        // (→ `_|`), `1` for Mongolian, `0x8` for German (no pause bits set).
        let lang_word_gap = match primary_bcp47_subtag(&lang) {
            "cmn" | "yue" | "zh" | "vi" => 1,
            "mn" => 1,
            _ => 0,
        };
        let reduce_dictionary_vowels = primary_bcp47_subtag(&lang) == "it";
        Self {
            reduce_dictionary_vowels,
            number_grammar: NumberGrammar::for_lang(&lang),
            reversed_textmode,
            bracket_pause: (4, 2),
            ipa_tie: None,
            ipa_separator: None,
            max_initial_consonants,
            unpronouncable,
            lang_word_gap,
            lang,
            ..Default::default()
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
    /// A parsed number-like token.
    Number(NumberToken),
    /// One or more whitespace characters collapsed into a single separator.
    Space,
    /// A word break that prints no space: the parts either side are looked up
    /// separately but spoken as one word.  A hyphen between two letters makes
    /// one — upstream reads "well-known" as `w'Eln'oUn`, not `w'El n'oUn`.
    WordJoin,
    /// Sentence/clause boundary punctuation: `.`, `,`, `!`, `?`, `;`, `:`.
    ClauseBoundary(char),
    /// Any other punctuation character.
    Punctuation(char),
    /// An arithmetic *operator* the tokenizer synthesised from the input (the
    /// unary minus of `-5`).  It reads as the language's word for the operation
    /// rather than as the dictionary's name for the character, which is often
    /// the name of the sign itself ("minus sign").
    Operator(char),
    /// A silent dot after a word of Roman numerals that doesn't end the
    /// clause (C's `FLAG_HAS_DOT`, which makes "XIV. kam" an ordinal).  It
    /// becomes a [`Token::Space`] before the words are translated.
    Dot,
    /// Inline phonemes given directly in `[[ … ]]` (espeak phoneme mnemonics).
    /// These bypass the dictionary and are emitted verbatim.
    InlinePhonemes(String),
    /// An inline embedded command (`\x01[±]<value><letter>`) — a rate, pitch or
    /// amplitude change that takes effect **from this point on**, not for the
    /// whole utterance.
    Embedded(EmbeddedCmd),
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

        let digit_end = word
            .bytes()
            .position(|b| !b.is_ascii_digit())
            .unwrap_or(word.len());
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
            NumberToken::Decimal {
                integer,
                fractional,
            } => format!("{integer}.{fractional}"),
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
/// C's `NormalizeStyledLatin` (`TranslateChar`): a stylised Latin letter or
/// digit read as plain ASCII, so it forms words as the plain one does — the
/// Mathematical Alphanumeric Symbols (bold, italic, script, fraktur,
/// double-struck, sans-serif, monospace; not the Greek ones), the Letterlike
/// Symbols that fill that block's holes, and the full-width forms.
fn normalize_styled_latin(c: char) -> Option<char> {
    let c = c as u32;
    let ascii = |base: u8, off: u32| char::from(base + u8::try_from(off).unwrap_or(0));
    match c {
        0x1d7ce..=0x1d7ff => Some(ascii(b'0', (c - 0x1d7ce) % 10)),
        0x1d400..=0x1d6a3 => {
            let o = (c - 0x1d400) % 52;
            Some(if o < 26 { ascii(b'A', o) } else { ascii(b'a', o - 26) })
        }
        0x1d6a4 => Some('i'),
        0x1d6a5 => Some('j'),
        0xff21..=0xff3a => Some(ascii(b'A', c - 0xff21)),
        0xff41..=0xff5a => Some(ascii(b'a', c - 0xff41)),
        0xff10..=0xff19 => Some(ascii(b'0', c - 0xff10)),
        0x210e => Some('h'),
        0x212c => Some('B'),
        0x2130 => Some('E'),
        0x2131 => Some('F'),
        0x210b | 0x210c | 0x210d => Some('H'),
        0x2110 | 0x2111 => Some('I'),
        0x2112 => Some('L'),
        0x2133 => Some('M'),
        0x211b | 0x211c | 0x211d => Some('R'),
        0x212f => Some('e'),
        0x210a => Some('g'),
        0x2134 => Some('o'),
        0x212d | 0x2102 => Some('C'),
        0x2128 | 0x2124 => Some('Z'),
        0x2115 => Some('N'),
        0x2119 => Some('P'),
        0x211a => Some('Q'),
        _ => None,
    }
}

pub fn tokenize(text: &str) -> Vec<Token> {
    tokenize_opts(text, &NumberGrammar::default())
}

/// True if `suffix` (already lower-cased) is a recognised written ordinal marker
/// for the number `digits` — the English `st`/`nd`/`rd`/`th`, the Iberian
/// masculine/feminine `º`/`ª`, or the language's own ordinal indicator (e.g.
/// Dutch `e`).  Anything else after a number (a plural `s`, a unit `km`) is
/// *not* an ordinal.
///
/// The English suffix must **match** the number (`1st`, `2nd`, `3rd`, `11th`,
/// `21st`): a mismatched one like `2st`/`1nd`/`3th` is not an ordinal, so the
/// number is spoken and the stray letters kept as a separate word rather than
/// producing a mangled ordinal ("2st" → "secst"; upstream #96).
fn is_ordinal_suffix(suffix: &str, digits: &str, grammar: &NumberGrammar) -> bool {
    if grammar.flags.is_some() {
        return grammar.ordinals.indicator.as_deref() == Some(suffix)
            || grammar.ordinal_suffixes.iter().any(|s| s == suffix);
    }
    if matches!(suffix, "st" | "nd" | "rd" | "th") {
        if suffix == english_ordinal_suffix(digits) {
            return true;
        }
        // Not the right English suffix — but it may still be a French written
        // ordinal (`2nd` = second), handled below; otherwise it's not ordinal.
    } else if matches!(suffix, "º" | "ª") {
        return true; // Iberian gender markers, independent of the number
    }
    grammar.ordinals.indicator.as_deref() == Some(suffix)
        || (grammar.ordinals.french && is_french_ordinal_suffix(suffix))
}

/// The French written ordinal markers: `1er`/`1ers` (premier), `1re`/`1ère`
/// (première), `2e`/`2ème`/`2eme`/`2es` (deuxième), `2nd`/`2nde` (second).
fn is_french_ordinal_suffix(suffix: &str) -> bool {
    matches!(
        suffix,
        "er" | "ers"
            | "re"
            | "res"
            | "ère"
            | "ères"
            | "e"
            | "es"
            | "ème"
            | "èmes"
            | "eme"
            | "emes"
            | "nd"
            | "nds"
            | "nde"
            | "ndes"
    )
}

/// Spell a French cardinal number in the range `0..=999` as text (the vigesimal
/// system: `71` → "soixante et onze", `80` → "quatre-vingts", `234` → "deux cent
/// trente-quatre").  Used only to build ordinals (the phoneme path renders
/// cardinals directly), so it stays under 1000.
fn fr_cardinal_text(n: u32) -> String {
    const U: [&str; 20] = [
        "zéro", "un", "deux", "trois", "quatre", "cinq", "six", "sept", "huit", "neuf", "dix",
        "onze", "douze", "treize", "quatorze", "quinze", "seize", "dix-sept", "dix-huit",
        "dix-neuf",
    ];
    if n < 20 {
        return U.at(n as usize).to_string();
    }
    if n < 60 {
        let (tens, u) = (n / 10, n % 10);
        let base = match tens {
            2 => "vingt",
            3 => "trente",
            4 => "quarante",
            5 => "cinquante",
            _ => unreachable!(),
        };
        return match u {
            0 => base.to_string(),
            1 => format!("{base} et un"),
            _ => format!("{base}-{}", U.at(u as usize)),
        };
    }
    if n < 80 {
        // soixante + (0..=19): "soixante", "soixante et un/onze", "soixante-dix"…
        return match n - 60 {
            0 => "soixante".to_string(),
            1 => "soixante et un".to_string(),
            11 => "soixante et onze".to_string(),
            inner => format!("soixante-{}", U.at(inner as usize)),
        };
    }
    if n < 100 {
        // quatre-vingt(s) + (0..=19); no "et", plural "s" only on bare 80.
        return match n - 80 {
            0 => "quatre-vingts".to_string(),
            inner => format!("quatre-vingt-{}", U.at(inner as usize)),
        };
    }
    // 100..=999.
    let (h, rest) = (n / 100, n % 100);
    let mut head = if h == 1 {
        "cent".to_string()
    } else {
        format!("{} cent", U.at(h as usize))
    };
    if h > 1 && rest == 0 {
        head.push('s'); // "deux cents"
    }
    if rest == 0 {
        head
    } else {
        format!("{head} {}", fr_cardinal_text(rest))
    }
}

/// Turn the final cardinal word into its ordinal stem + "ième"
/// (`quatre`→"quatrième", `cinq`→"cinquième", `neuf`→"neuvième", `un`→"unième").
fn ordinalize_fr_word(w: &str) -> String {
    match w {
        "un" => return "unième".to_string(),
        "cinq" => return "cinquième".to_string(),
        "neuf" => return "neuvième".to_string(),
        // The only plural forms that surface as a final word: "quatre-vingts"
        // (80) and "deux cents" (200) → "…vingtième"/"…centième".
        "vingt" | "vingts" => return "vingtième".to_string(),
        "cent" | "cents" => return "centième".to_string(),
        _ => {}
    }
    // Drop a silent final "e" (quatre→quatr, onze→onz, trente→trent); words
    // ending in a *sounded* consonant (trois, six, sept) keep it.
    let stem = w.strip_suffix('e').unwrap_or(w);
    format!("{stem}ième")
}

/// Build the French ordinal *word* for `digits` (a written ordinal like `1er`,
/// `2e`, `21e`) using its `suffix` for gender/`second` disambiguation.  Returns
/// `None` outside `1..=1000` (the caller then falls back to the cardinal).
///
/// The fr dictionary ships no `_No` ordinal entries, so the number path can't
/// produce "premier"/"deuxième"; instead we generate the word and let the
/// French letter-to-sound rules pronounce it (verified to render cleanly).
fn french_ordinal_word(digits: &str, suffix: &str) -> Option<String> {
    let n: u32 = digits.parse().ok()?;
    if n == 0 {
        return None;
    }
    if n == 1 {
        let feminine = matches!(suffix, "re" | "res" | "ère" | "ères");
        return Some(if feminine { "première" } else { "premier" }.to_string());
    }
    if n == 2 && matches!(suffix, "nd" | "nds" | "nde" | "ndes") {
        let feminine = matches!(suffix, "nde" | "ndes");
        return Some(if feminine { "seconde" } else { "second" }.to_string());
    }
    if n == 1000 {
        return Some("millième".to_string());
    }
    if n > 999 {
        return None;
    }
    // Ordinalise the last hyphen/space-delimited word of the cardinal.
    let card = fr_cardinal_text(n);
    let (head, last) = match card.rfind(['-', ' ']) {
        Some(i) => (&card[..=i], &card[i + 1..]),
        None => ("", card.as_str()),
    };
    Some(format!("{head}{}", ordinalize_fr_word(last)))
}

/// The irregular Italian ordinal word for `1..=10` (`3º` → "terzo", `3ª` →
/// "terza"), which the dict lacks — 11+ use the regular `-esimo` rule and are
/// left to the normal path.  Gender comes from the indicator (`º`/`°` masculine,
/// `ª` feminine).  `None` outside 1–10.
fn italian_ordinal_word(digits: &str, suffix: &str) -> Option<String> {
    let n: u32 = digits.parse().ok()?;
    if !(1..=10).contains(&n) {
        return None;
    }
    const MASC: [&str; 10] = [
        "primo", "secondo", "terzo", "quarto", "quinto", "sesto", "settimo", "ottavo", "nono",
        "decimo",
    ];
    const FEM: [&str; 10] = [
        "prima", "seconda", "terza", "quarta", "quinta", "sesta", "settima", "ottava", "nona",
        "decima",
    ];
    let table = if suffix == "ª" { &FEM } else { &MASC };
    Some(table.at((n - 1) as usize).to_string())
}

/// Spell a Portuguese cardinal group `0..=999` as text (European Portuguese).
/// Components are joined with "e" (`234` → "duzentos e trinta e quatro"); `100`
/// alone is "cem", within a larger number it's "cento".  Empty for `0`.
fn pt_group(n: u32) -> String {
    const U: [&str; 20] = [
        "zero",
        "um",
        "dois",
        "três",
        "quatro",
        "cinco",
        "seis",
        "sete",
        "oito",
        "nove",
        "dez",
        "onze",
        "doze",
        "treze",
        "catorze",
        "quinze",
        "dezasseis",
        "dezassete",
        "dezoito",
        "dezanove",
    ];
    const T: [&str; 10] = [
        "",
        "",
        "vinte",
        "trinta",
        "quarenta",
        "cinquenta",
        "sessenta",
        "setenta",
        "oitenta",
        "noventa",
    ];
    const H: [&str; 10] = [
        "",
        "cento",
        "duzentos",
        "trezentos",
        "quatrocentos",
        "quinhentos",
        "seiscentos",
        "setecentos",
        "oitocentos",
        "novecentos",
    ];
    let tens_units = |tu: u32| -> String {
        if tu < 20 {
            U.at(tu as usize).to_string()
        } else {
            let (t, u) = ((tu / 10) as usize, tu % 10);
            if u == 0 {
                T.at(t).to_string()
            } else {
                format!("{} e {}", T.at(t), U.at(u as usize))
            }
        }
    };
    if n == 0 {
        return String::new();
    }
    if n == 100 {
        return "cem".to_string(); // bare 100 is "cem", 101+ is "cento"
    }
    let mut parts: Vec<String> = Vec::new();
    if n / 100 > 0 {
        parts.push(H.at((n / 100) as usize).to_string());
    }
    if n % 100 > 0 {
        parts.push(tens_units(n % 100));
    }
    parts.join(" e ")
}

/// Build the Portuguese cardinal word for `digits` in `0..=999_999_999` (the pt
/// dict lacks number keys).  `None` above that range (the caller keeps the
/// existing path).
fn portuguese_cardinal_word(digits: &str) -> Option<String> {
    let n: u64 = digits.parse().ok()?;
    if n > 999_999_999 {
        return None;
    }
    if n == 0 {
        return Some("zero".to_string());
    }
    let millions = (n / 1_000_000) as u32;
    let thousands = ((n / 1000) % 1000) as u32;
    let units = (n % 1000) as u32;

    // (group value, rendered text with its scale word) for each non-zero group.
    let mut pieces: Vec<(u32, String)> = Vec::new();
    if millions > 0 {
        pieces.push((
            millions,
            if millions == 1 {
                "um milhão".to_string()
            } else {
                format!("{} milhões", pt_group(millions))
            },
        ));
    }
    if thousands > 0 {
        pieces.push((
            thousands,
            if thousands == 1 {
                "mil".to_string()
            } else {
                format!("{} mil", pt_group(thousands))
            },
        ));
    }
    if units > 0 {
        pieces.push((units, pt_group(units)));
    }

    // Join with spaces, but "e" before the *final* group when its value is < 100
    // or a round hundred ("dois milhões e quinhentos mil", "mil e vinte"); other
    // group boundaries are bare ("um milhão duzentos e trinta e quatro mil …").
    let last = pieces.len() - 1;
    let mut out = String::new();
    for (i, (value, text)) in pieces.iter().enumerate() {
        if i > 0 {
            let use_e = i == last && (*value < 100 || *value % 100 == 0);
            out.push_str(if use_e { " e " } else { " " });
        }
        out.push_str(text);
    }
    Some(out)
}

/// Build the spoken Portuguese text for a cardinal or decimal number token
/// (`1,5` → "um vírgula cinco"; the fractional part is read digit by digit).
/// `None` for ordinals or values the cardinal speller doesn't cover.
fn portuguese_number_word(token: &NumberToken) -> Option<String> {
    match token {
        NumberToken::Cardinal(digits) => portuguese_cardinal_word(digits),
        NumberToken::Decimal {
            integer,
            fractional,
        } => {
            let mut s = portuguese_cardinal_word(integer)?;
            s.push_str(" vírgula");
            // Portuguese reads the fraction as one number when it is short
            // enough (upstream `NUM_DFRACTION_2`): "3,14" is "três vírgula
            // catorze", not "… um quatro".  Leading zeros stay individual.
            let zeros = fractional.bytes().take_while(|&b| b == b'0').count();
            let rest = &fractional[zeros..];
            if !rest.is_empty() && rest.len() <= 2 {
                for _ in 0..zeros {
                    s.push(' ');
                    s.push_str(&portuguese_cardinal_word("0")?);
                }
                s.push(' ');
                s.push_str(&portuguese_cardinal_word(rest)?);
            } else {
                for d in fractional.chars() {
                    s.push(' ');
                    s.push_str(&portuguese_cardinal_word(&d.to_string())?);
                }
            }
            Some(s)
        }
        NumberToken::Ordinal(_) => None,
    }
}

/// Replace stand-alone Unicode number symbols that would otherwise be dropped
/// (they are neither letters nor ASCII digits) with a spoken ASCII form:
/// super/subscript digits → the digit, and vulgar fractions → `n/m`.  Spaces are
/// inserted so the substitution doesn't merge with an adjacent number
/// (`10³` → `10 3`, not `103`).  Returns the input unchanged (borrowed) when it
/// contains none of these characters.  Language-neutral — the resulting digits
/// and `/` are read by the normal number/symbol path in the target language.
/// Map the typographic apostrophes onto the plain one — C's
/// `TranslateClause` character loop ("'microsoft' quote or sexed closing single
/// quote, or prime - possibly used as apostrophe").  Without it `don’t` split
/// into "don" and a spoken "T", and Irish `D’fhuascail` into two words.
///
/// Applied to the **clause's own text** only, exactly as C does: a dictionary
/// `$textmode` replacement is re-translated without passing through that loop,
/// so Italian's `€ simbolo dell’euro` keeps its curly apostrophe and reads as
/// two words (`s'imbolo d'El 'EURo`), not one.
fn normalize_apostrophes(text: &str) -> std::borrow::Cow<'_, str> {
    const SMART: [char; 4] = ['\u{2019}', '\u{00B4}', '\u{2032}', '\u{0092}'];
    if !text.chars().any(|c| SMART.contains(&c)) {
        return std::borrow::Cow::Borrowed(text);
    }
    std::borrow::Cow::Owned(
        text.chars()
            .map(|c| if SMART.contains(&c) { '\'' } else { c })
            .collect(),
    )
}

/// A superscript power or vulgar fraction, which many dictionaries name.
fn is_dict_number_symbol(c: char) -> bool {
    matches!(c, '²' | '³' | '¼' | '½' | '¾' | '⅓' | '⅔')
}

#[cfg(test)]
fn normalize_number_symbols(text: &str) -> std::borrow::Cow<'_, str> {
    normalize_number_symbols_for(text, &NumberGrammar::default())
}

fn normalize_number_symbols_for<'a>(
    text: &'a str,
    grammar: &NumberGrammar,
) -> std::borrow::Cow<'a, str> {
    fn mapping(c: char) -> Option<&'static str> {
        Some(match c {
            // Superscript digits (⁰¹²³⁴⁵⁶⁷⁸⁹).
            '\u{2070}' | '⁴' | '⁵' | '⁶' | '⁷' | '⁸' | '⁹' => match c {
                '\u{2070}' => " 0",
                '⁴' => " 4",
                '⁵' => " 5",
                '⁶' => " 6",
                '⁷' => " 7",
                '⁸' => " 8",
                _ => " 9",
            },
            // Superscript ²/³ are powers, read "squared"/"cubed" (not "two").
            '¹' => " 1",
            '²' => " squared ",
            '³' => " cubed ",
            // Subscript digits (₀₁₂₃₄₅₆₇₈₉ = U+2080..2089).
            '₀' => " 0",
            '₁' => " 1",
            '₂' => " 2",
            '₃' => " 3",
            '₄' => " 4",
            '₅' => " 5",
            '₆' => " 6",
            '₇' => " 7",
            '₈' => " 8",
            '₉' => " 9",
            // Arabic/Persian native separators → their Latin equivalents (kept
            // adjacent to the digits so `١٫٥`→`1.5`, `١٬٠٠٠`→`1,000`, `٥٪`→`5%`).
            '\u{066B}' => ".", // ARABIC DECIMAL SEPARATOR ٫
            '\u{066C}' => ",", // ARABIC THOUSANDS SEPARATOR ٬
            '\u{066A}' => "%", // ARABIC PERCENT SIGN ٪

            _ => return None,
        })
    }

    let changed = |c: char| mapping(c).is_some() || native_digit_to_ascii(c).is_some();
    if !text.chars().any(changed) {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        if grammar.dict_symbols.contains(&c) {
            out.push(' ');
            out.push(c);
            out.push(' ');
        } else if grammar.powers_as_digits && matches!(c, '²' | '³') {
            out.push_str(if c == '²' { " 2" } else { " 3" });
        } else if let Some(s) = mapping(c) {
            out.push_str(s);
        } else if let Some(d) = native_digit_to_ascii(c) {
            // Consecutive native digits stay adjacent so they form one number.
            out.push(d);
        } else {
            out.push(c);
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Map a non-ASCII decimal digit (Arabic-Indic `٥`, Devanagari `५`, Thai `๕`, …)
/// to its ASCII equivalent so the normal number path reads it in the target
/// language.  `None` for anything that isn't a decimal digit of a known script.
fn native_digit_to_ascii(c: char) -> Option<char> {
    let cp = c as u32;
    let base = match cp {
        0x0660..=0x0669 => 0x0660, // Arabic-Indic
        0x06F0..=0x06F9 => 0x06F0, // Extended Arabic-Indic (Persian/Urdu)
        0x0966..=0x096F => 0x0966, // Devanagari
        0x09E6..=0x09EF => 0x09E6, // Bengali
        0x0A66..=0x0A6F => 0x0A66, // Gurmukhi
        0x0AE6..=0x0AEF => 0x0AE6, // Gujarati
        0x0B66..=0x0B6F => 0x0B66, // Oriya
        0x0BE6..=0x0BEF => 0x0BE6, // Tamil
        0x0C66..=0x0C6F => 0x0C66, // Telugu
        0x0CE6..=0x0CEF => 0x0CE6, // Kannada
        0x0D66..=0x0D6F => 0x0D66, // Malayalam
        0x0E50..=0x0E59 => 0x0E50, // Thai
        0x0ED0..=0x0ED9 => 0x0ED0, // Lao
        0x0F20..=0x0F29 => 0x0F20, // Tibetan
        _ => return None,
    };
    char::from_digit(cp - base, 10)
}

/// Tokenize with language-specific options.
/// CTRL_EMBEDDED marker (0x01) that starts an inline command (`\x01+10P`).
pub const CTRL_EMBEDDED: char = '\u{01}';

/// The command letters accepted after `\x01[±]<value>` — pitch/speed/amplitude/
/// range/reverb/tone/… (C `EmbeddedCommand`'s `"PSARHTIVYMUBF"`), plus `Z`,
/// which carries an SSML pause length in milliseconds (see
/// [`ssml::BREAK_LETTER`]).  C has no equivalent — it packs the pause into the
/// clause terminator — and drops the letter, so the two never disagree about
/// text written for either.
const EMBED_LETTERS: &str = "PSARHTIVYMUBFZ";

/// A parsed inline embedded command `\x01[+|-]<value><letter>` (C
/// `EmbeddedCommand`).  `relative`: 0 = absolute, 1 = `+`, -1 = `-`.  A missing
/// value is `-1` (the caller substitutes the parameter default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddedCmd {
    pub letter: char,
    pub value: i32,
    pub relative: i8,
}

/// Parse and strip inline embedded commands (`\x01…`) from `text`, returning the
/// command-free text and the commands in order.  Mirrors C's handling of
/// `CTRL_EMBEDDED`; a stray `\x01` (or one not followed by a valid letter) is
/// dropped so it can never be spoken as garbage.
pub fn parse_embedded_commands(text: &str) -> (String, Vec<EmbeddedCmd>) {
    let mut out = String::with_capacity(text.len());
    let mut cmds = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != CTRL_EMBEDDED {
            out.push(c);
            continue;
        }
        let relative = match chars.peek() {
            Some('+') => {
                chars.next();
                1
            }
            Some('-') => {
                chars.next();
                -1
            }
            _ => 0,
        };
        let mut digits = String::new();
        while let Some(d) = chars.next_if(char::is_ascii_digit) {
            digits.push(d);
        }
        match chars.next() {
            Some(l) if EMBED_LETTERS.contains(l.to_ascii_uppercase()) => {
                cmds.push(EmbeddedCmd {
                    letter: l.to_ascii_uppercase(),
                    value: digits.parse().unwrap_or(-1),
                    relative,
                });
            }
            // Not a valid command (or `\x01` at end): the whole sequence —
            // including the trailing char — is consumed and dropped, matching
            // C's `EmbeddedCommand` (which advances past the letter regardless).
            _ => {}
        }
    }
    (out, cmds)
}

pub fn tokenize_opts(text: &str, grammar: &NumberGrammar) -> Vec<Token> {
    let styled: Option<String> = text
        .chars()
        .any(|c| normalize_styled_latin(c).is_some())
        .then(|| text.chars().map(|c| normalize_styled_latin(c).unwrap_or(c)).collect());
    let text = styled.as_deref().unwrap_or(text);
    let text = normalize_number_symbols_for(text, grammar);
    let text = text.as_ref();
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c == CTRL_EMBEDDED {
            // `\x01[±]<value><letter>` — keep it as a token so the change applies
            // from here on rather than to the whole utterance.
            let relative = match chars.peek() {
                Some('+') => {
                    chars.next();
                    1
                }
                Some('-') => {
                    chars.next();
                    -1
                }
                _ => 0,
            };
            let mut digits = String::new();
            while let Some(d) = chars.next_if(char::is_ascii_digit) {
                digits.push(d);
            }
            match chars.next() {
                Some(l) if EMBED_LETTERS.contains(l.to_ascii_uppercase()) => {
                    tokens.push(Token::Embedded(EmbeddedCmd {
                        letter: l.to_ascii_uppercase(),
                        value: digits.parse().unwrap_or(-1),
                        relative,
                    }));
                }
                // A stray `\x01` is dropped rather than spoken as garbage.
                _ => {}
            }
            continue;
        }
        if c.is_whitespace() {
            // Collapse runs of whitespace into a single Space token.
            while chars.peek().map(|c| c.is_whitespace()).unwrap_or(false) {
                chars.next();
            }
            tokens.push(Token::Space);
        } else if c == '_'
            && CLAUSE_REDUCE.with(std::cell::Cell::get).is_some()
            && chars.peek().is_some_and(char::is_ascii_lowercase)
        {
            // Letters the clause's language spelled (`_x`, see
            // `is_spelled_abbreviation`).
            let mut word = String::from("_");
            while let Some(&n) = chars.peek().filter(|n| n.is_ascii_lowercase()) {
                word.push(n);
                chars.next();
            }
            tokens.push(Token::Word(word));
        } else if is_dict_number_symbol(c) && !grammar.dict_symbols.contains(&c) {
            // A fraction or power the dictionary doesn't name is a symbol,
            // read like any other (in English, if need be).
            tokens.push(Token::Punctuation(c));
        } else if c == ssml::SSML_BREAK {
            // SSML clause boundary (<break/>, </p>, </s>) — silent boundary,
            // never printed even in preserve-punctuation mode.
            tokens.push(Token::ClauseBoundary(c));
        } else if c == '[' && chars.peek() == Some(&'[') {
            // Inline phonemes: [[ mnemonics ]] — read verbatim to the closing ]].
            chars.next(); // consume second '['
            let mut content = String::new();
            while let Some(nc) = chars.next() {
                if nc == ']' {
                    if chars.peek() == Some(&']') {
                        chars.next(); // consume the closing ']]'
                        break;
                    }
                    content.push(']');
                } else {
                    content.push(nc);
                }
            }
            tokens.push(Token::InlinePhonemes(content));
        } else if c == grammar.decimal_separator
            && chars.peek().map_or(false, |d| d.is_ascii_digit())
        {
            // A decimal point with no integer part.  Standalone (".5") it reads
            // "zero point five"; but immediately after a number it is a *continued*
            // decimal separator — a dotted "1.2.3" version → "one point two point
            // three" — and takes no "zero".
            let integer = if matches!(tokens.last(), Some(Token::Number(_))) {
                ""
            } else {
                "0"
            };
            let mut fractional = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    fractional.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(Token::Number(NumberToken::Decimal {
                integer: integer.into(),
                fractional,
            }));
        } else if !matches!(c, '.' | ',' | '!' | '?' | ';' | ':')
            && clause_punctuation_class(c).is_some_and(|(_, optional_space)| {
                // C only ends the clause when the character is followed by
                // space, a bracket or end of text — unless the character's
                // property says a space is optional, which is what gives
                // space-less scripts their sentence breaks (`।`, `。`).
                optional_space
                    || chars
                        .peek()
                        .is_none_or(|&n| n.is_whitespace() || is_bracket(n))
            })
        {
            // Non-ASCII clause punctuation: the Devanagari danda, the CJK full
            // stop and comma, the Armenian/Syriac/Ethiopic/Mongolian/Tibetan
            // terminators, the ellipsis, the en/em dash.  Unspaced (`a–b`) they
            // are dropped instead, which is C's behaviour too.
            while chars.peek().map(|ch| ch.is_whitespace()).unwrap_or(false) {
                chars.next();
            }
            tokens.push(Token::ClauseBoundary(c));
        } else if matches!(c, '.' | ',' | '!' | '?' | ';' | ':') {
            // A '.' sitting tightly between single letters is an acronym
            // separator ("U.S.A.", "e.g.", "i.e."), not a sentence end — espeak
            // does not break there.  Drop it: the letters are already separate
            // word tokens and get spelled.  Only fires when the '.' immediately
            // follows a one-letter word and is immediately followed by a letter,
            // so trailing periods ("U.S.A." + space/end) and decimals are
            // untouched.
            if c == '.'
                && matches!(chars.peek(), Some(n) if n.is_alphabetic())
                && matches!(tokens.last(), Some(Token::Word(w)) if is_single_letter_word(w))
            {
                continue;
            }
            // A '.' immediately followed by a letter or digit is not a sentence
            // end — espeak reads it as the "dot"/"point"/"punkt" symbol
            // ("cat.Dog" → "cat dot dog", "5.and" → "five dot and", and in a
            // comma-decimal locale de "3.14" → "drei punkt vierzehn").  A real
            // full stop is always followed by whitespace or end of text.  (An
            // English "3.14" never reaches here — its '.' is the decimal
            // separator, consumed by the number tokenizer above.)
            if c == '.' && matches!(chars.peek(), Some(n) if n.is_alphanumeric()) {
                tokens.push(Token::Punctuation('.'));
                continue;
            }
            // A ':' immediately followed by a digit is a time or ratio separator,
            // not a clause break — "10:30" or "3:2", where the language's rules
            // read it, or nothing does.  A ':' before whitespace/
            // end ("Note: this") stays a clause boundary below.
            if c == ':' && matches!(chars.peek(), Some(n) if n.is_ascii_digit()) {
                tokens.push(Token::Punctuation(':'));
                continue;
            }
            // Three dots or more are an ellipsis for C's `ReadClause`, which
            // ends the clause whatever follows ("wait... what").
            if c == '.' && chars.peek() == Some(&'.') && chars.clone().nth(1) == Some('.') {
                while chars.peek() == Some(&'.') {
                    chars.next();
                }
                while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
                    chars.next();
                }
                tokens.push(Token::ClauseBoundary('\u{2026}'));
                continue;
            }
            // Clause/sentence boundary punctuation
            // (A mark glued to it is one with it for C's `ReadClause`: "?!".)
            while chars.peek().is_some_and(|&n| matches!(n, '!' | '?')) && matches!(c, '!' | '?') {
                chars.next();
            }
            // Absorb trailing whitespace after punctuation
            while chars.peek().map(|ch| ch.is_whitespace()).unwrap_or(false) {
                chars.next();
            }
            tokens.push(Token::ClauseBoundary(c));
        } else if c == '-'
            && minus_precedes_number(&chars)
            && matches!(
                tokens.last(),
                None | Some(Token::Space | Token::WordJoin) | Some(Token::ClauseBoundary(_))
            )
        {
            // A unary minus written with an ASCII hyphen directly before a
            // number ("-5") reads as the language's
            // "minus" word.  Gated to a clause start or just-after-a-space — a
            // hyphen following a word or number ("COVID-19", "5-10") is a
            // separator/range, not a sign, and is left to the punctuation path.
            // Keep the hyphen as written and mark it an *operator*: C reads a
            // leading `-` by translating the character as a word — Finnish
            // `*_rules` has `__) - (  mi:nus`, Greek a `- m'ion` list entry —
            // and reads the *operation*, not the dictionary's name for the sign,
            // which in several languages is "minus sign" (uk "знак мінуса", he
            // "סימן מינוס").  The currency symbol and digits tokenise normally
            // on the following iterations.
            tokens.push(Token::Operator('-'));
        } else if c.is_ascii_digit() {
            let mut digits = String::new();
            digits.push(c);
            let mut has_dot = false;
            let mut fractional = String::new();
            let mut first_group: Option<usize> = None;
            while let Some(&next) = chars.peek() {
                if next.is_ascii_digit() {
                    if has_dot {
                        fractional.push(next);
                    } else {
                        digits.push(next);
                    }
                    chars.next();
                } else if next == grammar.decimal_separator && !has_dot {
                    // Peek ahead to see if followed by a digit (a real decimal).
                    let mut lookahead = chars.clone();
                    lookahead.next(); // skip the decimal separator
                    if lookahead
                        .peek()
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false)
                    {
                        has_dot = true;
                        chars.next();
                    } else {
                        break;
                    }
                } else if grammar.group_separator == Some(next) && !has_dot {
                    // Digit-group separator (e.g. `1,000`): only a genuine group —
                    // exactly three digits that are *not* themselves followed by a
                    // digit — is absorbed; otherwise the char is left as punctuation.
                    let mut lookahead = chars.clone();
                    lookahead.next(); // skip the separator
                    let d1 = lookahead.next();
                    let d2 = lookahead.next();
                    let d3 = lookahead.next();
                    let three_digits = [d1, d2, d3]
                        .iter()
                        .all(|c| c.map_or(false, |c| c.is_ascii_digit()));
                    let more_digits = lookahead.peek().map_or(false, |c| c.is_ascii_digit());
                    if three_digits && !more_digits {
                        first_group.get_or_insert(digits.len());
                        chars.next(); // consume the separator; the 3 digits accumulate below
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

            // Check for an ordinal suffix immediately after the digits ("1st",
            // "2nd", "3º", Dutch "1e").  Only a *recognised* ordinal suffix is
            // absorbed — other trailing letters (a plural "1990s", a unit "5km")
            // are left for the next token so the number is still spoken instead of
            // being swallowed into a bogus ordinal.
            let mut lookahead = chars.clone();
            // (A dot before the indicator, "1.º", is read as a dot: C has no
            // ordinal there.)
            let dot_before_indicator = false;
            let mut suffix = String::new();
            while let Some(&next) = lookahead.peek() {
                if next.is_alphabetic() || next == 'º' || next == 'ª' {
                    suffix.push(next);
                    lookahead.next();
                } else {
                    break;
                }
            }
            let suffix_lc = suffix.to_lowercase();
            if !suffix.is_empty() && is_ordinal_suffix(&suffix_lc, &digits, grammar) {
                if dot_before_indicator {
                    chars.next(); // consume the skipped '.'
                }
                for _ in 0..suffix.chars().count() {
                    chars.next(); // now consume the suffix from the real iterator
                }
                tokens.push(Token::Number(NumberToken::Ordinal(OrdinalNumber {
                    digits,
                    marker: OrdinalMarker::Suffix(suffix_lc),
                })));
                continue;
            }

            // NUM_ORDINAL_DOT: if enabled, a trailing dot after digits marks ordinal
            // (e.g. German "3." → "dritte"). Only when NOT followed by a digit.
            if grammar.ordinals.dot_marks_ordinal && chars.peek() == Some(&'.') {
                let mut lookahead = chars.clone();
                lookahead.next(); // skip '.'
                let after_dot = lookahead.peek().copied();
                // C's `ReadClause` keeps the dot in the clause only before a
                // lower-case word, and `CheckDotOrdinal` needs a word (or a
                // comma) after it: "22. B", "22." and "22.)" are cardinals.
                let mut probe = lookahead.clone();
                while probe.peek().is_some_and(|c| c.is_whitespace()) {
                    probe.next();
                }
                let next_word = probe.peek().copied();
                let follows = after_dot == Some(',')
                    || after_dot.is_some_and(char::is_whitespace)
                        && next_word.is_some_and(|c| c.is_lowercase() || c == '-');
                if follows {
                    chars.next();
                    tokens.push(Token::Number(NumberToken::Ordinal(OrdinalNumber {
                        digits,
                        marker: OrdinalMarker::Dot,
                    })));
                    continue;
                }
            }

            // A short number written with a group separator keeps it: C reads
            // "3,007" as two words, with a pause after the thousands.
            if let Some(at) = first_group.filter(|_| digits.len() <= 4) {
                if let Some(sep) = grammar.group_separator {
                    digits.insert(at, sep);
                }
            }
            tokens.push(Token::Number(NumberToken::Cardinal(digits)));
        } else if is_cjk_ideograph(c) {
            // Each ideograph is its own word.  Upstream *joins* a pair that
            // undergoes tone sandhi (你好 → `ni35X'Au214_|`, no gap and no
            // stress on the first syllable), which `apply_tone_sandhi` does on
            // the code list rather than here.
            tokens.push(Token::Word(c.to_string()));
            while let Some(&next) = chars.peek() {
                if is_cjk_ideograph(next) {
                    tokens.push(Token::Space);
                    tokens.push(Token::Word(next.to_string()));
                    chars.next();
                } else {
                    break;
                }
            }
        } else if c.is_alphabetic()
            || is_braille(c)
            || (c == '\'' && grammar.apostrophe & 1 != 0)
            // An apostrophe is within a word for C, so one straight after a
            // number starts the word that follows ("80's" is `80 's`).
            || (c == '\''
                && matches!(tokens.last(), Some(Token::Number(_)))
                && chars.peek().is_some_and(|n| n.is_alphabetic()))
        {
            // Accumulate a word (letters, apostrophes, hyphens within words).
            let mut word = String::new();
            // Lojban (`caps_mark_stress`): a capital marks the stressed syllable,
            // so the first capital — even a leading one — gets a `ˈ` before it
            // and the letter is kept (lowercased downstream at lookup).
            let mut syllable_marked = false;
            if grammar.caps_mark_stress && c.is_uppercase() {
                word.push('\u{02c8}');
                syllable_marked = true;
            }
            word.push(c);
            // Track the last consumed character for the CamelCase splits (espeak
            // breaks at a lowercase→uppercase boundary, and at the last uppercase
            // of a run when it is followed by lowercase + another letter).
            let mut prev_char = c;
            let mut word_break = false;
            let mut hyphen_join = false;
            while let Some(&next) = chars.peek() {
                if is_cjk_ideograph(next) {
                    // Stop word accumulation at CJK boundary.
                    break;
                } else if grammar.caps_mark_stress && next.is_uppercase() {
                    // Lojban: keep the word whole; the first capital inserts a
                    // primary-stress mark, later capitals are just lowercased.
                    chars.next();
                    if !syllable_marked {
                        word.push('\u{02c8}');
                        syllable_marked = true;
                    }
                    word.push(next);
                    prev_char = next;
                } else if grammar.caps_word_split && prev_char.is_lowercase() && next.is_uppercase()
                {
                    // lowercase→uppercase: CamelCase word boundary.  Leave `next`
                    // for the outer loop to start the following word (`CamelCase`
                    // → "Camel" + "Case", `iPhone` → "i" + "Phone").  A `Space` is
                    // emitted below so the boundary is identical to a literal
                    // space (espeak sets `c = ' '`), spacing every output surface.
                    word_break = true;
                    break;
                } else if grammar.caps_word_split
                    && prev_char.is_uppercase()
                    && next.is_uppercase()
                    && caps_run_ends_here(&chars, next, &word, prev_char, grammar.dutch_ij)
                {
                    // upper→lower: split at the LAST uppercase of a run when it is
                    // followed by a lowercase letter + another letter, so an
                    // acronym glued to a capitalised word separates (`HTMLParser`
                    // → "HTML" + "Parser", `JABberwocky` → "JA" + "Bberwocky").
                    // `next` starts the new word.
                    word_break = true;
                    break;
                } else if grammar.tone_numbers && next.is_ascii_digit() {
                    // Tone digit belongs to the syllable (`ni3`), not to a number.
                    word.push(next);
                    chars.next();
                    prev_char = next;
                } else if next == '\''
                    && !(grammar.apostrophe & 2 != 0
                        || chars
                            .clone()
                            .nth(1)
                            .is_some_and(|after| after.is_alphanumeric()))
                {
                    // C keeps an apostrophe in the word only *between* two
                    // letters (`don't`, `O'Brien`); elsewhere it becomes a space
                    // — so `hello'` is "hello", not a word the dictionary has
                    // never seen.  Leave it for the outer loop.
                    break;
                } else if next.is_alphabetic()
                    || next == '\''
                    // Catalan's middle dot (`col·legi`), between two letters.
                    || (grammar.middle_dot_in_word
                        && next == '\u{b7}'
                        && chars.clone().nth(1).is_some_and(char::is_alphabetic))
                    || is_indic_combining(next)
                    || is_braille(next)
                {
                    // Indic combining marks (virama ्, nukta ़) aren't classed as
                    // alphabetic but are part of the word — keeping them lets the
                    // rules see a conjunct (`स्त`) instead of two broken words.
                    word.push(next);
                    chars.next();
                    prev_char = next;
                } else if next == '-' {
                    // A hyphen between two letters is a word break, not part of
                    // the word (espeak `TranslateClause`: `well-known` → "well
                    // known", `X-ray` → "eks ray").  Consume and drop the hyphen
                    // and emit a `Space` so the two parts read as separate words;
                    // a trailing/standalone hyphen just ends the word.
                    let mut lookahead = chars.clone();
                    lookahead.next(); // skip '-'
                    if lookahead.peek().map(|c| c.is_alphabetic()).unwrap_or(false) {
                        chars.next(); // consume (drop) the hyphen
                        word_break = true;
                        hyphen_join = true;
                        break;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            tokens.push(Token::Word(word));
            if word_break {
                tokens.push(if hyphen_join {
                    Token::WordJoin
                } else {
                    Token::Space
                });
            }
        } else if (0x3100..=0x9fff).contains(&(c as u32))
            && !c.is_alphanumeric()
            && !is_clause_punctuation(c)
            && !is_bracket(c)
        {
            // A symbol in the CJK alphabet's range (`㎏`) is a word for C,
            // named by its alphabet: "Chinese symbol".
            tokens.push(Token::Word(c.to_string()));
        } else if is_emoji_char(c) && !is_clause_punctuation(c) {
            // Emoji read via the dict's emoji-name entries.  A whole sequence
            // (variation selector, skin-tone modifier, ZWJ-joined parts, or a
            // regional-indicator flag pair) stays one word so it matches the
            // multi-codepoint dictionary entry.
            //
            // A few codepoints in the emoji blocks are *clause punctuation* to C
            // (`❓` and `❗` are a question and an exclamation mark, `‼` a double
            // one): those end a clause instead of being read, so they fall
            // through to the punctuation arm below.  `⁉` carries both properties,
            // matches none of C's cases, and so is read by name — as it is here.
            let mut emoji = String::from(c);
            let is_regional = |ch: char| (0x1F1E6..=0x1F1FF).contains(&(ch as u32));
            if is_regional(c) {
                if let Some(n) = chars.next_if(|&n| is_regional(n)) {
                    emoji.push(n);
                }
            } else {
                while let Some(&m) = chars.peek() {
                    if is_emoji_tag(m) {
                        // Tag sequence (subdivision flag): keep every tag byte.
                        chars.next();
                        emoji.push(m);
                    } else if is_emoji_modifier(m) {
                        chars.next();
                        // Drop the emoji-presentation variation selector (U+FE0F) —
                        // the dictionary keys the base emoji without it — but keep
                        // the ZWJ and skin-tone, which change the emoji's identity.
                        if m != '\u{FE0F}' {
                            emoji.push(m);
                        }
                        // A ZWJ binds the following emoji into the same sequence.
                        if m == '\u{200D}' {
                            if let Some(n) = chars.next_if(|&n| is_emoji_char(n)) {
                                emoji.push(n);
                            }
                        }
                    } else {
                        break;
                    }
                }
            }
            tokens.push(Token::Word(emoji));
        } else {
            tokens.push(Token::Punctuation(c));
            // A non-letter mark inside a word (a Myanmar asat) ends it, and
            // the letter after it starts the next, as C's tokenizer splits it:
            // `ၵႂၢမ်း` is `k'O1a:1m` and a separate `း`.
            let after_word = matches!(tokens.iter().rev().nth(1), Some(Token::Word(_)));
            if after_word
                && c as u32 >= 0x80
                && !c.is_whitespace()
                && chars.peek().is_some_and(|&n| n.is_alphabetic())
                && !is_clause_punctuation(c)
                && !is_bracket(c)
                && !matches!(c, '\u{2019}' | '\u{2018}' | '\u{b7}')
            {
                tokens.push(Token::Space);
            }
        }
    }

    tokens
}

/// Whether an uppercase `next` is the last uppercase of a run that should be
/// split off under espeak's upper→lower CamelCase rule (`HTMLParser` → "HTML"
/// + "Parser").  It splits when `next` is followed by a lowercase letter and
/// then another alphabetic character.  The Dutch word-initial "IJ" digraph is
/// exempt (`dutch_ij`).  `chars` is positioned with `next` still at its front
/// (already `peek`ed), so the clone skips it before reading the two look-ahead
/// characters — mirroring C's `next_in` / `next2_in`.
fn caps_run_ends_here<I>(
    chars: &std::iter::Peekable<I>,
    next: char,
    word: &str,
    prev_char: char,
    dutch_ij: bool,
) -> bool
where
    I: Iterator<Item = char> + Clone,
{
    // Dutch keeps "IJ" at word start whole (prev 'I', this 'J', 2nd letter).
    if dutch_ij && prev_char == 'I' && next == 'J' {
        let letters = word.chars().filter(|c| c.is_alphabetic()).count();
        if letters == 1 {
            return false;
        }
    }
    let mut la = chars.clone();
    la.next(); // skip `next` itself (the cached peek)
    let after = la.next();
    let after2 = la.next();
    after.map(|c| c.is_lowercase()).unwrap_or(false)
        && after2.map(|c| c.is_alphabetic()).unwrap_or(false)
}

/// Hindi word-final schwa deletion (simplified).  The inherent vowel `V` at the
/// end of a Devanagari word — after a consonant, and only when the word has
/// another vowel — is dropped (`भारत` "bharatə" → "bharat", `स्कूल` "skulə" →
/// "skul").  espeak does the full positional deletion in the `V` phoneme program
/// (`ChangePhoneme(NULL)`); this covers the dominant, most audible case.
fn delete_final_schwa(phonemes: &mut Vec<u8>, phdata: &PhonemeData) {
    use crate::phoneme::{PHON_END_WORD, PH_PAUSE, PH_VOWEL};
    let v = phdata.lookup_phoneme("V");
    if v == 0 {
        return;
    }
    let had_null = phonemes.last() == Some(&0);
    if had_null {
        phonemes.pop();
    }
    let is_marker = |c: u8| (1..=8).contains(&c) || c == 26 || c == PHON_END_WORD;
    if phonemes.last() == Some(&v) {
        let body = phonemes.get(..phonemes.len() - 1).unwrap_or_default();
        let prev_is_consonant = body
            .iter()
            .rev()
            .find(|&&c| !is_marker(c))
            .and_then(|&c| phdata.get(c))
            .map(|p| p.typ != PH_VOWEL && p.typ != PH_PAUSE)
            .unwrap_or(false);
        // Keep the schwa only when it's the word's sole vowel (a lone consonant
        // `क`); if any earlier vowel remains — including another inherent V — the
        // final one is deletable (`घर` "ghɔrə" → "ghɔr").
        let has_other_vowel = body
            .iter()
            .any(|&c| phdata.get(c).map(|p| p.typ == PH_VOWEL).unwrap_or(false));
        if prev_is_consonant && has_other_vowel {
            phonemes.pop(); // drop the final inherent-vowel V
            if matches!(phonemes.last(), Some(&c) if is_marker(c)) {
                phonemes.pop(); // and the stress marker attached to it
            }
        }
    }
    if had_null {
        phonemes.push(0);
    }
}

/// An Indic combining mark (virama / nukta) that continues a word but isn't
/// classified as alphabetic, so the plain word scanner would split on it.
fn is_indic_combining(c: char) -> bool {
    matches!(
        c as u32,
        0x093C | 0x094D   // Devanagari nukta, virama
        | 0x09BC | 0x09CD // Bengali
        | 0x0A3C | 0x0A4D // Gurmukhi
        | 0x0ABC | 0x0ACD // Gujarati
        | 0x0B3C | 0x0B4D // Oriya
        | 0x0BCD          // Tamil
        | 0x0C3C | 0x0C4D // Telugu
        | 0x0CBC | 0x0CCD // Kannada
        | 0x0D3B | 0x0D3C | 0x0D4D // Malayalam
        | 0x0DCA          // Sinhala
        // ZWJ / ZWNJ: a joiner between letters is part of the word, not a break.
        // Sinhala writes conjuncts as consonant + virama + ZWJ + consonant
        // (`ක්‍ර` = kra); breaking there read the parts as separate letters
        // (`k ɹˈə` instead of `kɹˈɐ`) — upstream fixed the same bug in 1.53.0.
        | 0x200C | 0x200D
    )
}

/// A base emoji / pictograph codepoint (emoticons, symbols & pictographs,
/// transport, dingbats, misc symbols, stars/arrows, regional-indicator flags).
/// These are read via the dictionary's emoji-name entries.
fn is_emoji_char(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF   // emoji, pictographs, supplemental & extended-A
                            // (includes 1F1E6..1F1FF regional-indicator flags)
        | 0x2600..=0x27BF   // miscellaneous symbols + dingbats
        | 0x2B00..=0x2BFF   // stars, arrows
    )
}

/// A codepoint that modifies/joins an emoji rather than starting a new one:
/// the emoji variation selector, skin-tone modifiers, and the ZWJ.
fn is_emoji_modifier(c: char) -> bool {
    matches!(c as u32, 0xFE0F | 0x1F3FB..=0x1F3FF | 0x200D)
}

/// A skin-tone modifier (Fitzpatrick type 1-2 … 6).
fn is_skin_tone(c: char) -> bool {
    matches!(c as u32, 0x1F3FB..=0x1F3FF)
}

/// A TAG character (U+E0020–U+E007F).  Subdivision flags are a black-flag base
/// followed by tag letters and a cancel tag — 🏴 + `gbeng` + CANCEL is England.
/// The dictionary keys the whole sequence, so the tags must stay in the token
/// (upstream 1.53.0 fixed these being read as "black flag").
fn is_emoji_tag(c: char) -> bool {
    matches!(c as u32, 0xE0020..=0xE007F)
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
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
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
    word_sep: bool, // prepend a space before the first vowel?
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
    phonemes_to_ipa_full(
        phoneme_bytes,
        phdata,
        pending_stress_in,
        word_sep,
        use_en_overrides,
        LiaisonCtx::default(),
    )
}

/// Full phoneme-to-IPA renderer.
/// Codes that are *not* a spoken sound and so are transparent when a French
/// liaison phoneme looks ahead for the next real phoneme: stress markers
/// (2..=8, tonic 26), pause codes, and the `END_WORD` separator (liaison can
/// cross a word boundary, e.g. "deux enfants").  The `0` terminator is handled
/// by the caller's `take_while`, so it is not listed here.
fn liaison_skip(c: u8) -> bool {
    (2..=8).contains(&c) || c == PHON_STRESS_TONIC || c == PHON_END_WORD || is_pause_code(c)
}

/// Context a token's neighbours provide for resolving French liaison phonemes at
/// the *end* of the token (see `ph_french`: `z2`/`z3`/`t2`/`t3`).
#[derive(Clone, Copy, Default)]
pub struct LiaisonCtx {
    /// French: resolve liaison phonemes by the following sound.  Off for every
    /// other language ("X2"/"X3" are real consonants there, e.g. Hungarian "öt").
    pub resolve: bool,
    /// The next spoken token in this clause starts with a vowel — a token-final
    /// liaison surfaces (`z`/`t`), as in "deux enfants", "six ans".
    pub next_starts_vowel: bool,
    /// A pause follows (clause boundary or end of text).  Selects the *citation*
    /// form of a `3`-liaison — `z3`→[s] ("six"→"sis"), `t3`→[t] ("huit"→"yit") —
    /// whereas a `2`-liaison (and `z3` before a consonant-initial word) is silent.
    pub next_is_pause: bool,
    /// The language reduces unstressed vowels even in dictionary-given words
    /// (`LOPT_REDUCE & 1`; Italian only).  Carried for the `StressCondition`
    /// gate described on [`LangOptions::reduce_dictionary_vowels`], which is not
    /// applied yet.
    pub reduce_dict_vowels: bool,
    /// This word's phonemes came from the dictionary (or a number lookup), not
    /// from the letter-to-sound rules — Turkish/Bashkir/Tatar suppress vowel
    /// reduction then (`isTranslationGiven`).
    pub translation_given: bool,
    /// First real phoneme of the next spoken word (`0` at a pause / end of text).
    ///
    /// Context-dependent phonemes test their *neighbours*, and some of those
    /// conditions legitimately reach across the word boundary: Portuguese coda
    /// `s#` is `IF nextPh(isVowel) THEN z ELIF nextPh(isVoiced) THEN Z ELSE S`,
    /// so "três vírgula" is `tɾˈeʒ …` but "três" alone is `tɾˈeʃ`.  Without this
    /// the token-final phoneme saw nothing after it and always took the
    /// voiceless branch.  (`nextPhW` conditions still stop at the boundary — the
    /// neighbour is flagged as a word start.)
    pub next_phoneme: u8,
}

/// `liaison`: French liaison resolution (see `LiaisonCtx`).  When enabled, a
/// liaison phoneme (a 2-char consonant mnemonic ending in '2' or '3', e.g. `z2`,
/// `t2`, `z3`) is resolved by the following sound: kept before a vowel, dropped
/// before a consonant.  The scan looks past internal `END_WORD` separators, so it
/// works both across word boundaries ("deux enfants" keeps the /z/, "deux chats"
/// drops it) and *within* a single composed token — the number "200" =
/// "deux·cent" drops the /z/ because "cent" is consonant-initial, "2000" =
/// "deux·mille" likewise.  At the end of the token, `LiaisonCtx.next_starts_vowel`
/// / `next_is_pause` decide: a `3`-liaison surfaces its citation form before a
/// pause ("six"→"sis"), while a `2`-liaison is silent.  Only enabled for French;
/// other languages use `X2`/`X3` for real consonants (Hungarian "öt" = ö + `t2`),
/// which must never be dropped.
/// Resolve a context-dependent ("virtual") phoneme through its own program.
///
/// Phonemes whose mnemonic ends in `#` — `d#`, `t#`, `z#`, … — are defined in
/// each language's phoneme source as a program of conditions plus
/// `ChangePhoneme(x)` instructions, and the definitions differ per language:
///
/// * `ph_english`: `d#` → `IF prevPh(isVoiced) THEN d ELSE t`
/// * `ph_portugal`: `d#` → `virtual` + unconditional `ChangePhoneme(d)`
/// * `ph_pt_brazil`: branches to the affricate before a close front vowel
///
/// Hardcoding English's voicing rule (as this function's predecessor did) gave
/// Portuguese `dente` → `tˈẽŋtʰy` instead of `dˈẽntɨ` once the refreshed pt
/// rules started emitting `d#`.  Running the phoneme's actual program via
/// [`interpret_phoneme`] (upstream `InterpretPhoneme`'s `ChangePhoneme` path)
/// keeps every language's own definition.
///
/// Returns the replacement code, or `None` when the program specifies no change.
fn resolve_virtual_phoneme(
    code: u8,
    idx: usize,
    phoneme_bytes: &[u8],
    prev_phcode: u8,
    phdata: &PhonemeData,
    next_word_phoneme: u8,
    stress_level: u8,
    translation_given: bool,
) -> Option<u8> {
    phoneme_program_effects(
        code,
        idx,
        phoneme_bytes,
        prev_phcode,
        phdata,
        next_word_phoneme,
        stress_level,
        translation_given,
    )
    .change_phoneme_code
    .filter(|&c| c != 0 && c != code)
}

/// Run a phoneme's program with neighbour context and return everything it asks
/// for: the `ChangePhoneme` target *and* the list-editing instructions
/// (`InsertPhoneme`, `AppendPhoneme`, `IfNextVowelAppend`, `ChangeNextPhoneme`).
///
/// C keeps these in `phdata->pd_param[]` and applies them in `MakePhonemeList`;
/// they were no-ops here, which is why English lost its linking `r-`
/// ("sofa area" → `sˈəʊfə ˈeəɹiə` instead of `sˈəʊfəɹ ˈeəɹiə`) and the `;` glide
/// after `aI` ("my apple").
fn phoneme_program_effects(
    code: u8,
    idx: usize,
    phoneme_bytes: &[u8],
    prev_phcode: u8,
    phdata: &PhonemeData,
    next_word_phoneme: u8,
    stress_level: u8,
    translation_given: bool,
) -> crate::synthesize::bytecode::PhonemeExtract {
    let empty = crate::synthesize::bytecode::PhonemeExtract::default();
    let Some(ph) = phdata.get(code) else {
        return empty;
    };
    let program = ph.program;
    if program == 0 {
        return empty;
    }
    // Next two real phonemes, skipping stress markers, pauses and END_WORD.
    let mut following = phoneme_bytes
        .get(idx + 1..)
        .unwrap_or_default()
        .iter()
        .copied()
        .take_while(|&c| c != 0)
        .filter(|&c| !liaison_skip(c));
    // Fall back to the next *word*'s first phoneme when this token has nothing
    // more, flagging it as a word start so `nextPhW` conditions still stop at
    // the boundary while plain `nextPh` ones see through it (as in C, where the
    // phoneme list is continuous and `sourceix` marks the boundary).
    let in_token_next = following.next();
    let crosses_word = in_token_next.is_none();
    let next = in_token_next.unwrap_or(next_word_phoneme);
    let nb = crate::synthesize::bytecode::Neighbours {
        prev: prev_phcode,
        this: code,
        next,
        next2: following.next().unwrap_or(0),
        // Stress conditions in the program must see this phoneme's real stress;
        // leaving it at 0 made every vowel look "diminished" and fired changes
        // that shouldn't (Turkish "yüz" → `jˈøz`, Hindi schwa → `nə`).
        stress: stress_level,
        translation_given,
        next_wordstart: crosses_word,
        next2_wordstart: crosses_word,
        ..Default::default()
    };
    crate::synthesize::bytecode::interpret_phoneme_ctl(
        program,
        &phdata.phonindex,
        &nb,
        |c| phdata.get(c).cloned(),
        true, // PhonemeList stage: first ChangePhoneme wins
    )
}

pub fn phonemes_to_ipa_full(
    phoneme_bytes: &[u8],
    phdata: &PhonemeData,
    pending_stress_in: PendingStress,
    word_sep: bool,
    use_en_overrides: bool,
    liaison: LiaisonCtx,
) -> (String, PendingStress) {
    let mut out = String::new();
    let mut stress = pending_stress_in;
    let mut need_space = word_sep;
    let mut prev_phcode: u8 = 0; // track previous real phoneme for d#/z# logic
    const PH_VOICED_FLAG: u32 = 1 << 4; // phFLAGBIT_VOICED

    for (idx, &code) in phoneme_bytes.iter().enumerate() {
        if code == 0 {
            break;
        }

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
        // Code 17 is a miscellaneous control code in most tables but
        // `phonDEFAULTTONE` in a tone language, where it is a real tone phoneme
        // — Vietnamese's toneless syllables take it, and skipping it as a pause
        // dropped the tone from the IPA (`sˈin` instead of `sˈi1n`).
        let is_tone = is_tone_phoneme(code, phdata);
        if is_pause_code(code) && !is_tone {
            // END_WORD (||, code 15) marks a word boundary within a single token.
            // This is used in number phoneme sequences to create spaces between
            // components (e.g., "forty-two" → "fˈɔːti tˈuː").
            // Other pause codes (9, 10, 11) are silently skipped.
            if code == 15 {
                // PHON_END_WORD
                need_space = true;
                // Reset stress so next word gets fresh stress
                stress = PendingStress::None;
            }
            continue;
        }

        // ── Real phoneme ─────────────────────────────────────────────────
        // English `--ipa` output matches the C dictionary path: keep phoneme codes
        // from rules without synthesis-stage stress reductions (e.g. /V/ → schwa),
        // or "unseen" renders as ə… instead of ʌ….
        // Other languages (e.g. Spanish, Russian) still need ChangeIf resolution
        // for stressed vowel allophones.
        // Stress-conditioned allophones (`ChangeIfNotStressed`).  C additionally
        // refuses to apply these to dictionary-given words unless the language
        // sets `LOPT_REDUCE & 1` (`synthdata.c` `StressCondition`), but wiring
        // that to this port's notion of "came from the dictionary" made Russian
        // *lose* its reduction while Spanish kept its allophone, so the gate is
        // not applied here — see GAPS.md.
        // NOTE: the *stressed* branch resolves a citation phoneme to its
        // stressed form, which several languages need (Czech `milió-` renders
        // `mˈiliˌoːni`, not `mˈiliʲˌoːni`).  It also over-applies for Russian,
        // where the stressed allophone depends on the neighbours — `--ipa` shows
        // `prʲivʲˈɛt` where `-x` and the synthesizer (which run the
        // context-aware pass) correctly give `prʲivʲˈet`.  Skipping it here
        // regressed the languages above, so it stays; see GAPS §2.
        let code = if use_en_overrides {
            code
        } else {
            let is_primary = stress == PendingStress::Primary;
            phdata.resolve_stressed_phoneme(code, is_primary)
        };

        if let Some(ph) = phdata.get(code) {
            let is_vowel = ph.typ == 2; // phVOWEL
            let is_stress_type = ph.typ == 1; // phSTRESS

            if is_stress_type {
                // Stress-type phoneme in phontab (e.g. code 6 has type=1).
                // Decode as in DecodePhonemes: `std_length <= 4` with no program
                // is a stress MARKER, not an acoustic phoneme.
                if ph.program == 0 {
                    // Stress markers (and the emphasis marks `''`/`'!`, which
                    // have no program either) are not spoken.
                    if ph.std_length <= 4 {
                        match ph.std_length {
                            4 => {
                                stress = PendingStress::Primary;
                            }
                            2 | 3 => {
                                stress = PendingStress::Secondary;
                            }
                            _ => {}
                        }
                    }
                    continue;
                }
                // Everything else of this type is a **tone phoneme** — Mandarin's
                // `55`/`35`/`214`/`51`/`11` are phSTRESS entries carrying their own
                // program (C attaches them to the syllable as `tone_ph` and
                // `WritePhMnemonic`s them after the vowel).  Dropping them made
                // every Chinese syllable toneless (`你` → `nˈi`, upstream
                // `nˈi214`).
                if need_space {
                    out.push(' ');
                    need_space = false;
                }
                let ipa = phdata
                    .phoneme_ipa_string(ph.program)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| {
                        crate::translate::ipa_table::phoneme_ipa_lang(
                            code,
                            ph.mnemonic,
                            false,
                            use_en_overrides,
                        )
                    });
                // An empty IPA name would silently drop the tone: Vietnamese's
                // default tone (`phonDEFAULTTONE`) has none, and upstream prints
                // its mnemonic (`s'i1n`).
                let ipa = if ipa.is_empty() {
                    ph.mnemonic_display()
                } else {
                    ipa
                };
                out.push_str(&ipa);
                prev_phcode = code;
                continue;
            }

            // Resolve French liaison phonemes by the following sound.  A "liaison
            // phoneme" has a 2-char mnemonic ending in '2' or '3' (e.g. n2, z2, t2,
            // z3) and is a consonant; it surfaces only before a vowel.  We scan
            // forward to the next real phoneme (skipping stress/pause markers and
            // internal END_WORD separators): keep before a vowel, drop before a
            // consonant.  If nothing real follows in this token it is token-final:
            // kept before a vowel-initial next token; a `3`-liaison additionally
            // takes its citation form before a pause (`z3`→[s], `t3`→[t]); a
            // `2`-liaison (and `z3` before a consonant) is silent.
            if liaison.resolve {
                let mnemonic = ph.mnemonic;
                // Little-endian u32 mnemonic bytes [b0,b1,b2,b3].
                let b0 = (mnemonic & 0xff) as u8;
                let b1 = ((mnemonic >> 8) & 0xff) as u8;
                let b2 = ((mnemonic >> 16) & 0xff) as u8;
                // liaison "level": 2 = silent at pause, 3 = citation form at pause.
                let level = ((b1 == b'2' || b1 == b'3') && b2 == 0 && !is_vowel).then(|| b1 - b'0');
                if let Some(level) = level {
                    let next_is_vowel = phoneme_bytes
                        .get(idx + 1..)
                        .unwrap_or_default()
                        .iter()
                        .take_while(|&&c| c != 0)
                        .find(|&&c| !liaison_skip(c))
                        .and_then(|&c| phdata.get(c))
                        .map(|ph| ph.typ == 2 /* phVOWEL */);
                    match next_is_vowel {
                        // A real sound follows in-token: keep before a vowel, else drop.
                        Some(true) => {}
                        Some(false) => continue,
                        // Token-final: the neighbouring token / pause decides.
                        None if liaison.next_starts_vowel => {}
                        None if level == 3 && liaison.next_is_pause => {
                            // Citation form before a pause: z3→[s], t3→[t]
                            // (ph_french `ChangePhoneme`).  These are the only two
                            // `3`-liaisons; the citation is the voiceless base.
                            let citation = match b0 {
                                b'z' => "s",
                                b't' => "t",
                                _ => "",
                            };
                            if !citation.is_empty() {
                                if need_space {
                                    out.push(' ');
                                    need_space = false;
                                }
                                out.push_str(citation);
                                prev_phcode = code;
                            }
                            continue;
                        }
                        None => continue, // `2`-liaison at pause, or `3` before a consonant word
                    }
                }
            }

            // Output space separator between words if needed
            if need_space {
                out.push(' ');
                need_space = false;
            }

            let _word_final = phoneme_bytes
                .get(idx + 1..)
                .unwrap_or_default()
                .iter()
                .all(|&c| c == 0 || c <= 8 || c == 15);

            // This phoneme's own stress level, captured before the pending
            // stress is consumed below — the phoneme's program tests it.
            let this_stress_level: u8 = if is_vowel {
                match stress {
                    PendingStress::Primary => 4,
                    PendingStress::Secondary => 3,
                    PendingStress::None => 1,
                }
            } else {
                1
            };

            // Emit pending stress before vowels
            if is_vowel {
                match stress {
                    PendingStress::Primary => {
                        out.push_str(IPA_STRESS_PRIMARY);
                    }
                    PendingStress::Secondary => {
                        out.push_str(IPA_STRESS_SECONDARY);
                    }
                    PendingStress::None => {}
                }
                stress = PendingStress::None;
            }

            // Special handling for phonemes that change based on previous phoneme voice:
            // d# → 'd' if prev is voiced, else 't'
            // z# → 'z' if prev is voiced, else 's'
            // These are common in English past tense and plural suffixes.
            let b1 = ((ph.mnemonic >> 8) & 0xff) as u8;
            // Any phoneme may resolve to another through its own program, not
            // just the `#`-suffixed "virtual" ones: English `@2` (the schwa used
            // only for "the") is `IF nextPh(isVowel) THEN I2 ELSE @`, which is
            // what makes "the apple" `ðɪ ˈapəl`, and `@5` does the same for "to".
            {
                if let Some(changed) = resolve_virtual_phoneme(
                    code,
                    idx,
                    phoneme_bytes,
                    prev_phcode,
                    phdata,
                    liaison.next_phoneme,
                    this_stress_level,
                    liaison.translation_given,
                ) {
                    // `ChangePhoneme(NULL)` (code 1) deletes the phoneme —
                    // C: "NULL phoneme, discard".  Russian's palatalisation
                    // marker uses it, and printing its mnemonic put a stray `_`
                    // inside words ("пять" → `pʲˈɑ_tʲ`).
                    if changed == 1 {
                        prev_phcode = code;
                        continue;
                    }
                    if let Some(changed_ph) = phdata.get(changed) {
                        let ipa = phdata
                            .phoneme_ipa_string(changed_ph.program)
                            .unwrap_or_else(|| {
                                crate::translate::ipa_table::phoneme_ipa_lang(
                                    changed,
                                    changed_ph.mnemonic,
                                    changed_ph.typ == 2, /* phVOWEL */
                                    use_en_overrides,
                                )
                            });
                        if !ipa.is_empty() {
                            out.push_str(&ipa);
                            prev_phcode = changed;
                            continue;
                        }
                    }
                }

                // The English voicing fallback below only applies to the `#`
                // class (`d#`, `z#`); anything else falls through to normal
                // rendering when its program specified no change.
                let b0 = if b1 == b'#' {
                    (ph.mnemonic & 0xff) as u8
                } else {
                    0
                };
                // Check if previous real phoneme is voiced
                let prev_voiced = if let Some(prev_ph) = phdata.get(prev_phcode) {
                    prev_ph.typ == 2 /* phVOWEL */ ||
                    prev_ph.typ == 3 /* phLIQUID */ ||
                    (prev_ph.phflags & PH_VOICED_FLAG) != 0
                } else {
                    false
                };

                let ipa_char = if b0 == b'd' {
                    if prev_voiced {
                        "d"
                    } else {
                        "t"
                    }
                } else if b0 == b'z' {
                    if prev_voiced {
                        "z"
                    } else {
                        "s"
                    }
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

            // Look up IPA, most authoritative first:
            //   1. the ACTIVE phoneme table's own `i_IPA_NAME` bytecode,
            //   2. the curated English overrides (accent-neutral ones always,
            //      en-GB's vowel qualities only when the `en` table is active),
            //   3. the ipa1 mnemonic fallback.
            //
            // Order matters: the overrides were derived from `espeak-ng -v en`,
            // so letting them outrank phondata made every English voice speak
            // en-GB — `en-us` has its own table whose phondata says `3` is `ɚ`,
            // not `ə`. For the `en` table this order is a no-op: every override
            // code's phondata is either identical to the override or absent.
            let ipa = if let Some(ipa_str) = phdata.phoneme_ipa_string(ph.program) {
                ipa_str
            } else if let Some(ipa) = use_en_overrides
                .then(|| en_ipa_override(code, phdata.active_table_name()))
                .flatten()
            {
                ipa.to_string()
            } else {
                phoneme_ipa_lang(code, ph.mnemonic, is_vowel, false)
            };
            // (English linking `r` used to be approximated here by inspecting the
            // mnemonic for a `3`; the phoneme's own `IfNextVowelAppend(r-)`
            // program now does it properly, below.)
            out.push_str(&ipa);

            // List-editing instructions from the phoneme's own program: the
            // English linking `r-` (`IfNextVowelAppend`) and the `;` glide after
            // `aI` (`AppendPhoneme`).  C applies these in `MakePhonemeList`.
            let effects = phoneme_program_effects(
                code,
                idx,
                phoneme_bytes,
                prev_phcode,
                phdata,
                liaison.next_phoneme,
                this_stress_level,
                liaison.translation_given,
            );
            let mut appended: Option<u8> = effects.append_phoneme;
            if appended.is_none() {
                if let Some(c) = effects.append_if_next_vowel {
                    let next_real = phoneme_bytes
                        .get(idx + 1..)
                        .unwrap_or_default()
                        .iter()
                        .copied()
                        .take_while(|&b| b != 0)
                        .find(|&b| !liaison_skip(b))
                        .unwrap_or(liaison.next_phoneme);
                    if matches!(phdata.get(next_real), Some(n) if n.typ == 2 /* phVOWEL */) {
                        appended = Some(c);
                    }
                }
            }
            if let Some(c) = appended {
                // Only render an appended phoneme that has an IPA name of its
                // own: the linking `r-` is `ɹ`, while the `;` glide after `aI`
                // is a synthesis-only phoneme that upstream doesn't show in IPA.
                if let Some(extra) = phdata
                    .get(c)
                    .and_then(|aph| phdata.phoneme_ipa_string(aph.program))
                {
                    out.push_str(&extra);
                }
            }
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
    /// The pronunciation came from the dictionary *list* rather than from the
    /// letter-to-sound rules — C's `FLAG_FOUND`, which becomes `SFLAG_DICTIONARY`
    /// on every phoneme of the word and stops the stress-conditioned phoneme
    /// changes (`StressCondition`).  A word can carry dictionary *flags* without
    /// this (a flags-only `$` entry still leaves the rules to pronounce it), so
    /// it is tracked separately from `dict_flags`.
    pub found_in_list: bool,
    /// How many following words a multi-word dictionary entry took in with
    /// this one (C's `dictionary_skipwords`): French `(parce que)` is one
    /// entry, so "que" is not translated again.
    pub skipwords: usize,
}

fn append_raw_phonemes(dst: &mut Vec<u8>, src: &[u8]) {
    for &b in src {
        if b == 0 {
            break;
        }
        dst.push(b);
    }
}

fn combine_rules_result(result: &crate::dictionary::rules::RulesResult) -> Vec<u8> {
    let mut combined = Vec::new();
    append_raw_phonemes(&mut combined, &result.phonemes);
    append_raw_phonemes(&mut combined, &result.end_phonemes);
    combined
}

fn english_suffix_needs_e(stem: &str, dict: &Dictionary) -> bool {
    const ADD_E_EXCEPTIONS: &[&str] = &["ion"];
    const ADD_E_ADDITIONS: &[&str] = &[
        "c", "rs", "ir", "ur", "ath", "ns", "u", "spong", "rang", "larg",
    ];

    let chars: Vec<char> = stem.chars().collect();
    if chars.len() < 2 {
        return false;
    }

    let penultimate = chars.at(chars.len() - 2) as u32;
    let last = chars.at(chars.len() - 1) as u32;
    if is_letter_wc(
        &dict.letter_bits,
        penultimate,
        dict.letter_bits_offset,
        LETTERGP_VOWEL2,
    ) && is_letter_wc(&dict.letter_bits, last, dict.letter_bits_offset, LETTERGP_B)
    {
        return !ADD_E_EXCEPTIONS.iter().any(|suffix| stem.ends_with(suffix));
    }

    ADD_E_ADDITIONS.iter().any(|suffix| stem.ends_with(suffix))
}

/// C's `nl` branch of `RemoveEnding`: does taking a `SUFX_E` suffix off this
/// stem restore a doubled vowel?  True when the stem ends in an ASCII vowel plus
/// a consonant and the letter before the vowel is not a vowel — `grad` (from
/// `graden`) doubles to `graad`, while `grond` (from `gronden`) does not.
fn dutch_suffix_doubles_vowel(stem: &str, dict: &Dictionary) -> bool {
    let chars: Vec<char> = stem.chars().collect();
    if chars.len() < 2 {
        return false;
    }
    // Before a two-letter stem C reads the space ahead of the word: `open`
    // is `oop` + `-en`.
    let (last, vowel, before) = (
        chars.at(chars.len() - 1),
        chars.at(chars.len() - 2),
        chars.len().checked_sub(3).map_or(' ', |k| chars.at(k)),
    );
    if !last.is_ascii() || !vowel.is_ascii() {
        return false;
    }
    let is_vowel = |c: char| {
        is_letter_wc(
            &dict.letter_bits,
            c as u32,
            dict.letter_bits_offset,
            LETTERGP_VOWEL2,
        )
    };
    is_vowel(vowel)
        && is_letter_wc(
            &dict.letter_bits,
            last as u32,
            dict.letter_bits_offset,
            crate::dictionary::LETTERGP_C,
        )
        && !is_vowel(before)
}

fn remove_standard_prefix(word: &str, end_type: u32) -> Option<(String, u32)> {
    if end_type & SUFX_P == 0 {
        return None;
    }
    let n_chars = (end_type & 0x3f) as usize;
    if n_chars == 0 {
        return None;
    }
    let cut_end = word
        .char_indices()
        .nth(n_chars.saturating_sub(1))
        .map(|(idx, c)| idx + c.len_utf8());
    let Some(start_after) = cut_end else {
        return None;
    };
    if start_after > word.len() {
        return None;
    }
    let stem = word[start_after..].to_string();
    if stem.is_empty() {
        return None;
    }
    Some((stem, FLAG_PREFIX_REMOVED))
}

fn remove_standard_suffix(
    word: &str,
    end_type: u32,
    dict: &Dictionary,
) -> Option<(String, u32, u32)> {
    let suffix_len_chars = (end_type & 0x3f) as usize;
    if suffix_len_chars == 0 {
        return None;
    }

    let mut chars: Vec<char> = word.chars().collect();
    if suffix_len_chars > chars.len() {
        return None;
    }

    let suffix_start = chars.len() - suffix_len_chars;
    let ending: String = chars
        .get(suffix_start..)
        .unwrap_or_default()
        .iter()
        .collect();
    chars.truncate(suffix_start);

    if let Some(last) = chars
        .last_mut()
        .filter(|c| end_type & SUFX_I != 0 && **c == 'i')
    {
        *last = 'y';
    }

    let mut stem: String = chars.iter().collect();
    let mut end_flags = (end_type & 0xfff0) | FLAG_SUFX;

    if (end_type & SUFX_E) != 0 {
        match dict.lang.as_str() {
            "en" if english_suffix_needs_e(&stem, dict) => {
                stem.push('e');
                end_flags |= FLAG_SUFX_E_ADDED;
            }
            // Dutch spells a long vowel doubled in a closed syllable, so taking
            // the suffix off has to put the second vowel back: `graden` − `en`
            // is the stem `graad`, not `grad`, which is why C reads `Qr'a:d@n`
            // and not `Qr'Ad@n`.  C's own test (`RemoveEnding`, the `nl` branch)
            // is: the stem ends ASCII vowel + consonant and the letter before
            // the vowel is not itself a vowel.
            "nl" if dutch_suffix_doubles_vowel(&stem, dict) => {
                let mut chars: Vec<char> = stem.chars().collect();
                let last = chars.len() - 1;
                chars.insert(last, chars.at(last - 1));
                stem = chars.into_iter().collect();
            }
            _ => {}
        }
    }

    if ending == "s" || ending == "es" {
        end_flags |= FLAG_SUFX_S;
    }
    if ending.starts_with('\'') {
        end_flags &= !FLAG_SUFX;
    }

    let mut stem_word_flags = 0;
    if (end_flags & FLAG_SUFX) != 0 {
        stem_word_flags |= FLAG_SUFFIX_REMOVED;
    }
    if (end_type & SUFX_A) != 0 {
        stem_word_flags |= FLAG_SUFFIX_VOWEL;
    }

    Some((stem, end_flags, stem_word_flags))
}

// ---------------------------------------------------------------------------
// Number-to-phonemes (English)
// ---------------------------------------------------------------------------

/// Look up a number word from the dictionary (e.g. "_0", "_1", "_0C", "_0M1").
fn lookup_num_phonemes(dict: &Dictionary, key: &str) -> Vec<u8> {
    // The number words are ordinary list entries and can be `?n`-conditional:
    // European `pt` says `k&t'o*zy` for `_14`, Brazilian `pt-BR` `kat'o*zy`.
    let ctx = LookupCtx {
        lookup_symbol: true,
        dict_condition: dict.dict_condition,
        ..Default::default()
    };
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

    /// Drop the trailing phoneme (a real phoneme, not an END_WORD/stress
    /// marker) — used for vowel elision in compound numbers, e.g. Italian
    /// "trenta"+"uno" → "trent"+"uno" = "trentuno".
    fn drop_final_phoneme(&mut self) {
        if matches!(self.bytes.last(), Some(&b) if b > 8 && b != PHON_END_WORD) {
            self.bytes.pop();
        }
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
        self.bytes
            .extend_from_slice(other.bytes.get(..len).unwrap_or_default());
    }

    /// Reduce every primary stress but the last (C's `LookupNum2`).
    fn keep_last_primary(&mut self) {
        let mut found = false;
        for b in self.bytes.iter_mut().rev() {
            if *b == PHON_STRESS_P {
                if found {
                    *b = PHON_STRESS_3;
                }
                found = true;
            }
        }
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
    src.get(..len).unwrap_or_default()
}

fn num_key(raw: impl std::fmt::Display) -> String {
    format!("_{raw}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScaleGroup {
    value: u32,
    scale: Option<u8>,
}

/// The largest scale group spoken: C splits numbers of up to 32 digits, whose
/// first group is at 10^30 (scale 10).
const MAX_SCALE_GROUP: u32 = 10;

/// Split a value into three-digit groups, most-significant first.  Scale `k`
/// covers 10^(3k): 1 = thousand, 2 = million, 3 = billion, 4 = trillion, … up
/// to [`MAX_SCALE_GROUP`].  Leading zero groups are emitted but
/// skipped by `append_cardinal_group`, so small numbers are unaffected; this
/// fixes large numbers ≥ 10^12, which previously overflowed the billion group
/// (`1_000_000_000_000` → "ten hundred billion" instead of "one trillion").
fn split_scale_groups(value: u128, grammar: &NumberGrammar) -> Vec<ScaleGroup> {
    let mut groups: Vec<ScaleGroup> = Vec::with_capacity(MAX_SCALE_GROUP as usize + 1);
    let mut rest = value;
    let breaks = scale_breaks(grammar);
    for (scale, pair) in breaks
        .iter()
        .zip(breaks.iter().skip(1).map(Some).chain([None]))
        .enumerate()
    {
        let (&from, to) = pair;
        // The top group takes whatever is left.
        let divisor = to.map(|&to| 10u128.pow(to - from));
        let group = divisor.map_or(rest, |d| rest % d);
        rest = divisor.map_or(0, |d| rest / d);
        groups.push(ScaleGroup {
            value: u32::try_from(group).unwrap_or(u32::MAX),
            scale: u8::try_from(scale).ok().filter(|&n| n > 0),
        });
    }
    groups.reverse();
    groups
}

/// The digit positions where a scale begins: 0, then every third digit, or for
/// the languages that count in lakhs the breaks of `break_numbers`.
fn scale_breaks(grammar: &NumberGrammar) -> Vec<u32> {
    if !grammar.lakh {
        return (0..=MAX_SCALE_GROUP).map(|scale| scale * 3).collect();
    }
    std::iter::once(0)
        .chain((1..32).filter(|n| grammar.break_numbers & (1 << n) != 0))
        .collect()
}

/// [`split_scale_groups`] with the groups a language has no scale word for
/// carried into the next-lower one, for the ordinal reader.
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
fn split_scale_groups_folded(value: u128, dict: &Dictionary) -> Vec<ScaleGroup> {
    let mut groups = split_scale_groups(value, &NumberGrammar::default());
    // Fold any non-zero group whose scale word this language does not define
    // down into the next-lower scale (×1000), so magnitude is still conveyed.
    // A language with every scale word (en/de/ru up to 10^18) is untouched; one
    // that stops earlier (French has no `_0M4` past "milliard") still reads
    // 10^12 as "dix cent milliards" rather than dropping to a bare "un".
    // Groups are ordered most-significant first, so each carry lands on a group
    // processed later in the loop and can cascade further down if needed.
    for i in 0..groups.len() {
        let Some(scale) = groups[i].scale else {
            continue;
        };
        let val = groups[i].value;
        if val == 0 {
            continue;
        }
        let has_word = !lookup_num_phonemes(dict, &format!("_0M{scale}")).is_empty()
            || !lookup_num_phonemes(dict, &format!("_1M{scale}")).is_empty();
        if !has_word && i + 1 < groups.len() {
            groups[i + 1].value = groups[i + 1].value.saturating_add(val.saturating_mul(1000));
            groups[i].value = 0;
        }
    }
    groups
}

fn append_scale_word(
    dst: &mut Pronunciation,
    group_value: u32,
    scale: u8,
    dict: &Dictionary,
    grammar: &NumberGrammar,
    exact: bool,
    // The group really is thousands, not a higher scale with no word of its own.
    thousands: bool,
) {
    // Slavic scale-word declension: the scale word takes a case by the count
    // before it (`M_Variant`), and — in Russian — the count's trailing "one"/"two"
    // agrees in gender.
    if grammar.thousands_variant != SlavicThousands::None {
        let count = group_value;
        // A complete "{count} {scale}" word (Czech/Polish `_1M1` = "tisíc"/
        // "tysiąc" — the "one" is baked in, no separate multiplier).
        let dedicated = lookup_num_phonemes(dict, &format!("_{count}M{scale}"));
        if !dedicated.is_empty() {
            dst.push_lookup_word(&dedicated);
            return;
        }
        let teen = (11..=19).contains(&(count % 100));
        let d = count % 10;
        // Per-language `M_Variant`: the scale-word key prefix.
        let prefix = match grammar.thousands_variant {
            SlavicThousands::None => unreachable!(),
            SlavicThousands::Ru if !teen && d == 1 => "1MA",
            SlavicThousands::Ru if !teen && (2..=4).contains(&d) => "0MA",
            SlavicThousands::Cs | SlavicThousands::Sk if (2..=4).contains(&count) => "0MA",
            SlavicThousands::Pl if !teen && (2..=4).contains(&d) => "0MA",
            SlavicThousands::Lt if teen || d == 0 => "0MB",
            SlavicThousands::Lt if d == 1 => "0MA",
            SlavicThousands::Hr if !teen && d == 1 => "1M",
            SlavicThousands::Hr if !teen && (2..=4).contains(&d) => "0MA",
            _ => "0M",
        };
        let mut scale_word = lookup_num_phonemes(dict, &format!("_{prefix}{scale}"));
        if scale_word.is_empty() {
            scale_word = lookup_num_phonemes(dict, &format!("_0M{scale}"));
        }
        // Gender of the count's trailing "one"/"two".  "thousand" (scale 1) is
        // feminine in Russian (одна/две, for any count) and Slovak (дve, but only
        // for a bare single-digit count); the higher scales are masculine.
        let feminine = scale == 1
            && match grammar.thousands_variant {
                SlavicThousands::Ru | SlavicThousands::Hr => true,
                SlavicThousands::Sk => count < 10,
                _ => false,
            };
        let form = if feminine {
            UnitForm::Feminine
        } else {
            UnitForm::Plain
        };
        dst.push_pronunciation(&num3_phonemes_g(dict, count, false, grammar, form));
        dst.push_lookup_word(&scale_word);
        return;
    }

    // A word for this exact count of the scale, multiplier included — Arabic
    // `_2M1` "alfān", `_3M1` "thalāthatu ālāf" (C's `LookupThousands`).
    if group_value > 0 {
        let exact_key = exact.then(|| format!("_{group_value}M{scale}x"));
        let dedicated = exact_key
            .into_iter()
            .chain([format!("_{group_value}M{scale}")])
            .map(|key| lookup_num_phonemes(dict, &key))
            .find(|ph| !ph.is_empty());
        if let Some(dedicated) = dedicated {
            dst.push_lookup_word(&dedicated);
            return;
        }
    }

    // For an *exact* scale group (no hundreds/tens/units below it), some languages
    // use a variant scale word `_0M{n}x`: Tamil/Malayalam "ஆயிரம்"/"ആയിരം" for a
    // round N000, vs "ஆயிரத்தி"/"ആയിരത്തി" (`_0M{n}`) before more digits.  (Also
    // Kannada, Telugu, Sinhala, Persian, Georgian; mirrors espeak's
    // `LookupThousands`.)
    let scale_word = {
        let variant = if exact {
            lookup_num_phonemes(dict, &format!("_0M{scale}x"))
        } else {
            Vec::new()
        };
        if !variant.is_empty() {
            variant
        } else {
            lookup_num_phonemes(dict, &format!("_0M{scale}"))
        }
    };
    // "of" before the scale word of a count of twenty or more (Romanian "de").
    let scale_word = if group_value % 100 >= 20 {
        let mut with_of = trim_lookup(&lookup_num_phonemes(dict, "_0of")).to_vec();
        with_of.extend_from_slice(&scale_word);
        with_of
    } else {
        scale_word
    };
    let singular_key = format!("_1M{scale}");

    if thousands && scale == 1 && group_value == 1 && grammar.thousands.omit_one_prefix {
        dst.push_lookup_word(&scale_word);
        return;
    }

    if group_value == 1 {
        let singular = lookup_num_phonemes(dict, &singular_key);
        if !singular.is_empty() {
            dst.push_lookup_word(&singular);
            return;
        }
    }

    // A higher scale read as "thousand" doesn't agree with it.
    let agrees_with = if scale == 1 && !thousands { 0 } else { scale };
    dst.push_pronunciation(&num3_phonemes_before_scale(
        dict,
        group_value,
        agrees_with,
        grammar,
    ));
    dst.push_lookup_word(&scale_word);
}

fn append_cardinal_group(
    dst: &mut Pronunciation,
    group: ScaleGroup,
    dict: &Dictionary,
    grammar: &NumberGrammar,
    exact: bool,
    preceded: bool,
) {
    if group.value == 0 {
        return;
    }

    // C `NUM_AND_HUNDRED`: the conjunction is part of the hundreds word.
    let and_hundred = (grammar.and_hundred && preceded && group.value >= 100)
        .then(|| lookup_num_phonemes(dict, "_0and"));
    if let Some(scale) = group.scale {
        let mut part = Pronunciation::default();
        let thousands = scale == 1;
        let scale = spoken_scale(dict, scale);
        append_scale_word(
            &mut part,
            group.value,
            scale,
            dict,
            grammar,
            exact,
            thousands,
        );
        match and_hundred {
            Some(and) => {
                dst.push_lookup_word(&and);
                dst.append_lookup_suffix(&part.bytes);
            }
            None => {
                // C `NUM_THOUSAND_AND`, which only Arabic sets as well.
                if grammar.and_hundred
                    && grammar.tens == TensGrammar::UnitsThenConjunction
                    && preceded
                {
                    dst.push_lookup_word(&lookup_num_phonemes(dict, "_0and"));
                }
                dst.push_pronunciation(&part)
            }
        }
    } else {
        // The last group: no scale word follows, so the plain digit form.
        //
        // C's `LookupNum3` takes the conjunction when the group *follows* a
        // higher scale as well as when it has hundreds of its own — its
        // condition is `(value > 100) || (control & 1 && thousandplex == 0)`.
        // So "1001" is "one thousand AND one" / "mil E um", and only a group
        // below a hundred needs it here (one with hundreds already gets it
        // inside `num3_phonemes`).
        let wants_and = group.value < 100
            && (grammar.hundreds.use_conjunction_with_remainder
                || (grammar.hundreds.conjunction_before_simple_remainder
                    && (group.value % 10 == 0
                        || !lookup_num_phonemes(dict, &num_key(group.value)).is_empty())));
        if preceded && wants_and {
            let and = lookup_num_phonemes(dict, "_0and");
            if !and.is_empty() {
                dst.push_lookup_word(&and);
            }
        }
        let part = num3_phonemes(dict, group.value, false, grammar);
        match and_hundred {
            Some(and) => {
                dst.push_lookup_word(&and);
                dst.append_lookup_suffix(&part.bytes);
            }
            None => dst.push_pronunciation(&part),
        }
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
            dst.push_pronunciation(&num3_phonemes_before_scale(
                dict,
                group_value,
                scale,
                grammar,
            ));
        }
        dst.push_lookup_word(&ord_scale);
        return true;
    }

    // Ordinal fallback (no `_0M{n}o` key): keep the plain scale word, not the
    // exact `x` variant — that variant is a cardinal-only distinction.
    append_scale_word(dst, group_value, scale, dict, grammar, false, scale == 1);
    false
}

/// Convert a number value (0-999) to phonemes.
/// Mirrors C's LookupNum3 with per-language number flags.
/// Phonemes for a unit digit, using the feminine form (`_1f`/`_2f`) for 1 and 2
/// when `feminine` — Slavic scale-word agreement ("одна", "две").
fn unit_ph(dict: &Dictionary, n: u32, form: UnitForm) -> Vec<u8> {
    let variant = match form {
        UnitForm::Plain => Vec::new(),
        UnitForm::Feminine => lookup_num_phonemes(dict, &format!("_{n}f")),
        UnitForm::Combining => lookup_num_phonemes(dict, &format!("_{n}a")),
    };
    if variant.is_empty() {
        lookup_num_phonemes(dict, &num_key(n))
    } else {
        variant
    }
}

/// Which form the units of a group take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitForm {
    Plain,
    /// `_<n>f`, agreeing with a feminine scale word.
    Feminine,
    /// `_<n>a`, before a scale word.
    Combining,
}

/// A digit's *combining* form — C's `_<n>a`, used when a scale word follows
/// ("hundred", "thousand", …).  German's `_1a  _'aIn` is what makes 1000
/// "ein tausend" rather than "eins tausend"; falls back to the plain `_<n>`.
fn unit_before_scale(dict: &Dictionary, n: u32) -> Vec<u8> {
    let a = lookup_num_phonemes(dict, &format!("_{n}a"));
    if a.is_empty() {
        lookup_num_phonemes(dict, &num_key(n))
    } else {
        a
    }
}

fn num3_phonemes(
    dict: &Dictionary,
    value: u32,
    suppress_null: bool,
    grammar: &NumberGrammar,
) -> Pronunciation {
    num3_phonemes_g(dict, value, suppress_null, grammar, UnitForm::Plain)
}

/// [`num3_phonemes`] for a group that a *scale word* follows, so a bare digit
/// takes the combining `_<n>a` form — German 1000 is "ein tausend", not "eins
/// tausend".  Only the languages whose list defines the variant are affected.
fn num3_phonemes_before_scale(
    dict: &Dictionary,
    value: u32,
    scale: u8,
    grammar: &NumberGrammar,
) -> Pronunciation {
    let form = if grammar.feminine_scales.contains(&scale) {
        UnitForm::Feminine
    } else {
        UnitForm::Combining
    };
    num3_phonemes_g(dict, value, false, grammar, form)
}

/// Like [`num3_phonemes`], but with `feminine` gender for a trailing 1/2 unit
/// (Slavic thousands agreement).
fn num3_phonemes_g(
    dict: &Dictionary,
    value: u32,
    suppress_null: bool,
    grammar: &NumberGrammar,
    form: UnitForm,
) -> Pronunciation {
    let hundreds = value / 100;
    let tensunits = value % 100;

    let mut hundreds_part = Pronunciation::default();
    let mut tens_part = Pronunciation::default();
    let mut suppress_null = suppress_null;

    if hundreds > 0 {
        // Malayalam-style: for a bare 1-hundred, skip the dedicated `_1C`/`_1C0`
        // (which bakes in "one") and use the bare hundred word.
        let skip_dedicated = hundreds == 1 && grammar.hundreds.omit_one_hundred_word;
        // A dedicated "N hundred" word.  For an *exact* hundred (no remainder)
        // prefer `_{n}C0`, which some languages use as a distinct standalone form
        // vs. the `_{n}C` combining form spoken before more digits (Tamil "நூறு"
        // alone vs "நூற்று" in 101; also Kannada/Telugu).  A remainder skips the
        // exact form and takes `_{n}C` directly.  (Mirrors espeak's LookupNum3.)
        let exact = if tensunits == 0 && !skip_dedicated {
            lookup_num_phonemes(dict, &format!("_{}C0", hundreds))
        } else {
            Vec::new()
        };
        let compound = if skip_dedicated {
            Vec::new()
        } else {
            lookup_num_phonemes(dict, &format!("_{}C", hundreds))
        };
        if !exact.is_empty() {
            hundreds_part.push_lookup_word(&exact);
        } else if !compound.is_empty() {
            hundreds_part.push_lookup_word(&compound);
        } else {
            if !(hundreds == 1 && (grammar.hundreds.omit_one_prefix || skip_dedicated)) {
                // A hundred word follows, so the digit takes its combining form.
                hundreds_part.push_lookup_word(&unit_before_scale(dict, hundreds));
            }
            // Bare hundred word.  For the Malayalam bare-1-hundred, an exact
            // hundred uses the dedicated `_0C0` form ("നൂറ്"); everyone else keeps
            // `_0C`, so no other language is affected.
            let bare = if skip_dedicated && tensunits == 0 {
                let e = lookup_num_phonemes(dict, "_0C0");
                if e.is_empty() {
                    lookup_num_phonemes(dict, "_0C")
                } else {
                    e
                }
            } else {
                lookup_num_phonemes(dict, "_0C")
            };
            hundreds_part.append_lookup_suffix(&bare);
        }
        suppress_null = true;
    }

    if tensunits != 0 || !suppress_null {
        if tensunits < 20 {
            // For a bare 1/2 (the whole number is just the unit) use the feminine
            // form when required — "одна"/"две" before a feminine scale word.
            let whole = unit_ph(dict, tensunits, form);
            // Resolve the word for "ten": a dedicated `_10`, else the `_1X`
            // ten-prefix (Vietnamese "mười" has only `_1X`).
            let ten_word = || {
                let t = lookup_num_phonemes(dict, "_10");
                if t.is_empty() {
                    lookup_num_phonemes(dict, "_1X")
                } else {
                    t
                }
            };
            if !whole.is_empty() {
                // A dedicated teen word ("fifteen", "quinze").
                tens_part.push_lookup_word(&whole);
            } else if tensunits == 10 {
                // Exactly ten with no dedicated `_10` word — use the ten-prefix
                // (Vietnamese `_1X` = "mười"); previously this produced nothing.
                let ten = ten_word();
                if !ten.is_empty() {
                    tens_part.push_lookup_word(&ten);
                }
            } else if tensunits > 10 {
                // Compositional teens: languages like Turkish ("on beş") and
                // Turkmen ("on bäş") have "ten" (`_10`/`_1X`) but no teen words —
                // the dict lookup above is empty, so compose ten + unit.  (Without
                // this, `tr`/`tk`/`hy`/`hu` produced nothing for 11–19.)
                let ten = lookup_num_phonemes(dict, "_1X");
                let ten = if ten.is_empty() {
                    lookup_num_phonemes(dict, "_10")
                } else {
                    ten
                };
                if !ten.is_empty() {
                    tens_part.push_lookup_word(&ten);
                    // Some languages join the compositional teen with a
                    // conjunction ("kumi NA tano" = 15 in Swahili), the same one
                    // used between tens and units.  Turkish/Turkmen ("on beş")
                    // do not — they are `Standard`, so this stays gated.
                    if grammar.tens == TensGrammar::WithConjunction {
                        tens_part.append_lookup_suffix(&lookup_num_phonemes(dict, "_0and"));
                    }
                    tens_part.append_lookup_suffix(&unit_ph(dict, tensunits - 10, form));
                }
            }
        } else {
            let ph_full = lookup_num_phonemes(dict, &num_key(tensunits));
            if !ph_full.is_empty() {
                tens_part.push_lookup_word(&ph_full);
            } else if grammar.vigesimal_70_90 && (70..80).contains(&tensunits) {
                // French 70–79 = "soixante" (60) + (10–19).
                tens_part.push_lookup_word(&lookup_num_phonemes(dict, "_6X"));
                tens_part
                    .append_lookup_suffix(&lookup_num_phonemes(dict, &num_key(tensunits - 60)));
            } else if grammar.vigesimal_70_90 && (90..100).contains(&tensunits) {
                // French 90–99 = "quatre-vingt" (80) + (10–19).
                tens_part.push_lookup_word(&lookup_num_phonemes(dict, "_8X"));
                tens_part
                    .append_lookup_suffix(&lookup_num_phonemes(dict, &num_key(tensunits - 80)));
            } else {
                let tens = tensunits / 10;
                let units = tensunits % 10;

                match grammar.tens {
                    TensGrammar::UnitsThenConjunction if units != 0 => {
                        // The unit "1" apocopates inside a compound in German
                        // ("einundzwanzig", not "einsundzwanzig").
                        let unit = match (units, grammar.combining_one.as_deref()) {
                            (1, Some(one)) => lookup_num_phonemes(dict, one),
                            _ => lookup_num_phonemes(dict, &num_key(units)),
                        };
                        tens_part.push_lookup_word(&unit);
                        tens_part.append_lookup_suffix(&lookup_num_phonemes(dict, "_0and"));
                        tens_part
                            .append_lookup_suffix(&lookup_num_phonemes(dict, &format!("_{tens}X")));
                    }
                    TensGrammar::UnitsThenConjunction => {
                        tens_part
                            .push_lookup_word(&lookup_num_phonemes(dict, &format!("_{tens}X")));
                    }
                    TensGrammar::WithConjunction => {
                        tens_part
                            .push_lookup_word(&lookup_num_phonemes(dict, &format!("_{tens}X")));
                        if units != 0 {
                            tens_part.append_lookup_suffix(&lookup_num_phonemes(dict, "_0and"));
                            tens_part
                                .append_lookup_suffix(&lookup_num_phonemes(dict, &num_key(units)));
                        }
                    }
                    TensGrammar::Standard => {
                        tens_part
                            .push_lookup_word(&lookup_num_phonemes(dict, &format!("_{tens}X")));
                        if units != 0 {
                            // Italian: the tens word elides its final vowel before
                            // a vowel-initial unit (uno=1, otto=8) → "trentuno".
                            if grammar.elide_tens_vowel && (units == 1 || units == 8) {
                                tens_part.drop_final_phoneme();
                            }
                            // A trailing 1/2 agrees in gender ("двадцать одна").
                            tens_part.append_lookup_suffix(&unit_ph(dict, units, form));
                        }
                    }
                }
            }
        }
    }

    if grammar.single_stress {
        tens_part.keep_last_primary();
    }

    if hundreds > 0 && tensunits > 0 && grammar.hundreds.use_conjunction_with_remainder {
        hundreds_part.append_lookup_suffix(&lookup_num_phonemes(dict, "_0and"));
    } else if hundreds > 0 && tensunits > 0 && grammar.hundreds.conjunction_before_simple_remainder
    {
        // Only when the remainder is a single word (round ten, or a number with
        // a direct dictionary entry) — a compound remainder already has its own
        // conjunction, so a second one would be wrong ("honderd vier-en-dertig").
        let simple =
            tensunits % 10 == 0 || !lookup_num_phonemes(dict, &num_key(tensunits)).is_empty();
        if simple {
            hundreds_part.append_lookup_suffix(&lookup_num_phonemes(dict, "_0and"));
        }
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
        NumberToken::Cardinal(digits) => {
            Some(cardinal_pronunciation(digits, dict, grammar)?.finish())
        }
        NumberToken::Decimal {
            integer,
            fractional,
        } => {
            // An empty integer is a continued decimal ("1.2.3" → "…point three"):
            // render just the point and the fractional digits, no leading cardinal.
            let mut pronunciation = if integer.is_empty() {
                Pronunciation::default()
            } else {
                cardinal_pronunciation(integer, dict, grammar)?
            };
            let decimal_point = lookup_num_phonemes(dict, "_dpt");
            // C's `TranslateNumber_1`: `_dpt`, then the fraction in the same
            // word unless it starts one.
            if let Some(flags) = grammar.flags {
                pronunciation.push_lookup_word(&decimal_point);
                for b in cardinal::fraction(fractional, dict, flags) {
                    if b != PHON_END_WORD || pronunciation.bytes.last() != Some(&PHON_END_WORD) {
                        pronunciation.bytes.push(b);
                    }
                }
                return Some(pronunciation.finish());
            }
            if !decimal_point.is_empty() {
                pronunciation.push_lookup_word(&decimal_point);
            }
            if grammar.fraction_suffix
                && push_fraction(&mut pronunciation, fractional, dict, grammar)
            {
                return Some(pronunciation.finish());
            }
            if grammar.fraction_digits_as_number > 0
                && push_fraction_as_number(&mut pronunciation, fractional, dict, grammar)
            {
                return Some(pronunciation.finish());
            }
            for digit in fractional.bytes() {
                pronunciation.push_lookup_word(&lookup_num_phonemes(dict, &num_key(digit - b'0')));
            }
            Some(pronunciation.finish())
        }
        NumberToken::Ordinal(_) => None,
    }
}

/// Read a decimal fraction as a fraction: the digits as one number, followed by
/// the scale word for their count — `ru "0,25"` → "двадцать пять **сотых**".
///
/// Mirrors upstream `NUM_DFRACTION_5` (`numbers.c`), and is entirely data-driven:
/// the suffix comes from the language's own `_0Z<n>` (plural) / `_0Z<n>s`
/// (singular) dictionary entries, so a language without them falls back to
/// reading the digits one by one.  With `fraction_feminine` (Russian) the
/// numerator's final 1/2 take the feminine `_1f`/`_2f` forms and a numerator
/// ending in 1 — but not 11 — takes the singular suffix ("двадцать одна сотая").
///
/// Returns `false` if the language has no suffix entry for this digit count, so
/// the caller can fall back to digit-by-digit.
fn push_fraction(
    pronunciation: &mut Pronunciation,
    fractional: &str,
    dict: &Dictionary,
    grammar: &NumberGrammar,
) -> bool {
    let count = fractional.len();
    if count == 0 || !fractional.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let Ok(value) = fractional.parse::<u64>() else {
        return false;
    };

    let feminine_units =
        grammar.fraction_feminine && matches!(value % 10, 1 | 2) && !matches!(value % 100, 11 | 12);
    let singular = grammar.fraction_feminine && value % 10 == 1 && value % 100 != 11;

    let suffix = {
        let plural = lookup_num_phonemes(dict, &format!("_0Z{count}"));
        let chosen = if singular {
            let s = lookup_num_phonemes(dict, &format!("_0Z{count}s"));
            if s.is_empty() {
                plural
            } else {
                s
            }
        } else {
            plural
        };
        if chosen.is_empty() {
            return false;
        }
        chosen
    };

    // Numerator.  With a feminine final unit, speak the tens (if any) from the
    // normal cardinal machinery and append the feminine `_1f`/`_2f`.
    if feminine_units {
        let tens = value - (value % 10);
        if tens > 0 {
            let Some(p) = cardinal_pronunciation(&tens.to_string(), dict, grammar) else {
                return false;
            };
            // `push_pronunciation` drops the trailing word marker, so the
            // feminine unit below joins the tens into one numeral.
            pronunciation.push_pronunciation(&p);
        }
        let fem = lookup_num_phonemes(dict, if value % 10 == 1 { "_1f" } else { "_2f" });
        if fem.is_empty() {
            return false;
        }
        // Part of the same numeral, as every other tens+units pair is.
        if tens > 0 {
            pronunciation.append_lookup_suffix(&fem);
        } else {
            pronunciation.push_lookup_word(&fem);
        }
    } else {
        let Some(p) = cardinal_pronunciation(&value.to_string(), dict, grammar) else {
            return false;
        };
        pronunciation.push_pronunciation(&p);
    }

    // C `strcat(ph_buf, buf1)`: the scale word joins the numerator *inside* one
    // word, which is what makes `ru "1,2"` one unit — `V#d;'in_ _:'i_:
    // dv;'e_d;Is;'AtYx`, not two words with a gap between them.
    pronunciation.append_lookup_suffix(&suffix);
    true
}

/// Read the fractional digits as a single number (upstream `NUM_DFRACTION_2` /
/// `NUM_DFRACTION_4`): "3,14" is "three point **fourteen**" in Polish, Czech,
/// Finnish, Turkish, Portuguese, Spanish, French and friends — not "one four".
///
/// Leading zeros are still spoken individually ("0,05" → "nula päť"), and a
/// fraction longer than the language's limit (2 or 5 digits) falls back to
/// digit-by-digit, exactly as C does.
///
/// Returns `false` when the fallback should be used.
fn push_fraction_as_number(
    pronunciation: &mut Pronunciation,
    fractional: &str,
    dict: &Dictionary,
    grammar: &NumberGrammar,
) -> bool {
    if fractional.is_empty() || !fractional.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let zeros = fractional.bytes().take_while(|&b| b == b'0').count();
    let rest = &fractional[zeros..];
    if rest.is_empty() || rest.len() > grammar.fraction_digits_as_number as usize {
        return false;
    }
    let Some(number) = cardinal_pronunciation(rest, dict, grammar) else {
        return false;
    };
    for _ in 0..zeros {
        pronunciation.push_lookup_word(&lookup_num_phonemes(dict, &num_key(0)));
    }
    pronunciation.push_lookup_word(&number.finish());
    true
}

fn cardinal_pronunciation(
    digits: &str,
    dict: &Dictionary,
    grammar: &NumberGrammar,
) -> Option<Pronunciation> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }

    // A leading zero marks a code, not a quantity (C `TranslateNumber`): a short
    // one (≤3 digits) speaks its leading zeros then the number ("007" → "zero
    // zero seven"); a longer one is spoken digit-by-digit ("0800" → "zero eight
    // zero zero").  A lone "0" and decimals (a separate token) are unaffected.
    if digits.len() > 1 && digits.starts_with('0') {
        let mut result = Pronunciation::default();
        if digits.len() > 3 {
            for d in digits.bytes() {
                result.push_lookup_word(&lookup_num_phonemes(dict, &num_key(d - b'0')));
            }
            return Some(result);
        }
        // Speak the leading zeros (at most all but the last digit), then read the
        // remaining value with the normal cardinal machinery.
        let n_leading = digits
            .bytes()
            .take(digits.len() - 1)
            .take_while(|&b| b == b'0')
            .count();
        // C's `ph_zeros`: the zeros are one word.
        for k in 0..n_leading {
            let zero = lookup_num_phonemes(dict, "_0");
            if k == 0 {
                result.push_lookup_word(&zero);
            } else {
                result.append_lookup_suffix(&zero);
            }
        }
        let value: u128 = digits.parse().ok()?;
        // "000" → the leading zeros plus a final "zero" (an all-zero run).
        result.push_pronunciation(&num3_phonemes(dict, value as u32, false, grammar));
        return Some(result);
    }

    let value: u128 = digits.parse().ok()?;
    if value == 0 {
        let mut pronunciation = Pronunciation::default();
        pronunciation.push_lookup_word(&lookup_num_phonemes(dict, "_0"));
        return Some(pronunciation);
    }

    // C `NUM_1900`: 1900–1999 read as "nineteen hundred (and) …" — `hundreds`
    // is 19, which the three-digit path already handles (`_19` + `_0C` + the
    // usual conjunction).  Only the ten languages that set the flag do this;
    // the port used to read *every* round hundred from 1100 up this way, in
    // every language, so `en 1500` was "fifteen hundred" where upstream says
    // "one thousand five hundred", `de 1100` "elfhundert" for "ein tausend ein
    // hundert" — and `en 1984` missed the year reading entirely.
    if grammar.year_1900 && (1900..=1999).contains(&value) {
        return Some(num3_phonemes(dict, value as u32, false, grammar));
    }

    let mut result = Pronunciation::default();
    for (group, _) in cardinal_groups(value, dict, grammar) {
        result.push_pronunciation(&group);
    }
    Some(result)
}

/// The spoken groups of a cardinal, most significant first, each with whether
/// a pause follows it.
fn cardinal_groups(
    value: u128,
    dict: &Dictionary,
    grammar: &NumberGrammar,
) -> Vec<(Pronunciation, bool)> {
    let flags = grammar.flags.or(cardinal::NumberFlags::for_lang("en"));
    flags
        .and_then(|flags| cardinal::groups(&value.to_string(), dict, flags))
        .unwrap_or_default()
        .into_iter()
        .map(|group| {
            (
                Pronunciation {
                    bytes: group.phonemes,
                },
                group.pause,
            )
        })
        .collect()
}

fn has_scale_word(dict: &Dictionary, scale: u8) -> bool {
    ["_0M", "_1M", "_0MA", "_0MB"]
        .iter()
        .any(|key| !lookup_num_phonemes(dict, &format!("{key}{scale}")).is_empty())
}

/// A scale whose own name and the one below are both unavailable is read as
/// "millions" (C's `LookupThousands`).
fn skips_to_millions(dict: &Dictionary, scale: u8) -> bool {
    scale > 3 && lookup_num_phonemes(dict, &format!("_0M{}", scale - 1)).is_empty()
}

/// The scale whose words stand in for `scale`'s: itself when the language names
/// it, otherwise millions or a repeated "thousand".
fn spoken_scale(dict: &Dictionary, scale: u8) -> u8 {
    if has_scale_word(dict, scale) {
        scale
    } else if skips_to_millions(dict, scale) {
        2
    } else {
        1
    }
}

/// Read a number digit by digit (C's `FLAG_INDIVIDUAL_DIGITS`): the digits of a
/// group join into one word, and a number of 5–32 digits splits into groups the
/// way a quantity does.
fn individual_digits(
    digits: &str,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    grammar: &NumberGrammar,
) -> Vec<u8> {
    let grouped = (5..=32).contains(&digits.len());
    let mut out = Vec::new();
    let mut part = Pronunciation::default();
    let n = digits.len();
    let mut vowels = 0;
    let mut in_group = 0;
    for (ix, digit) in digits.bytes().enumerate() {
        let name = lookup_num_phonemes(dict, &num_key(digit - b'0'));
        vowels += name
            .iter()
            .filter(|&&b| {
                phdata
                    .get(b)
                    .is_some_and(|ph| ph.typ == crate::phoneme::PH_VOWEL)
            })
            .count();
        part.append_lookup_suffix(&name);
        in_group += 1;
        // C's `TranslateRules` reads the digits, with a pause after each pair.
        if in_group % 2 == 0 {
            part.bytes.push(crate::phoneme::PHON_PAUSE_NOLINK);
        }
        let left = n - ix - 1;
        let group_end = if grouped {
            left < 32 && grammar.break_numbers & (1 << left) != 0
        } else {
            // One word.  C cuts it off when its phoneme buffer is full (about
            // forty digits); every digit is spoken here, so the stress is placed
            // in stretches that fit the fixed-size vowel tables.
            vowels > 80
        };
        if group_end || left == 0 {
            vowels = 0;
            in_group = 0;
            // C puts the thousands separator after each group but the last,
            // and the rules read it with the digits (Spanish "punto").
            if grouped && left > 0 {
                // C's `thousands_sep`: `.` with `NUM_DECIMAL_COMMA`, none with
                // `NUM_THOUS_SPACE`.
                let numbers = grammar.flags.map_or(0, |f| f.numbers());
                let sep = if numbers & 0x4 != 0 {
                    None
                } else if numbers & 0x8 != 0 {
                    Some('.')
                } else {
                    Some(',')
                };
                if let Some(sep) = sep {
                    let mut buf = vec![b' ', sep as u8, b' ', b' ', 0];
                    let (mut vowels, mut stressed) = (0i32, 0i32);
                    let by_rules = crate::dictionary::rules::untraced(|| {
                        crate::dictionary::rules::translate_rules(
                            dict,
                            &buf,
                            1,
                            0,
                            0,
                            &dict.letter_bits,
                            dict.dict_condition,
                            &mut vowels,
                            &mut stressed,
                        )
                    });
                    buf.clear();
                    let name = if by_rules.phonemes.iter().any(|&b| b != 0) {
                        by_rules.phonemes
                    } else {
                        lookup_num_phonemes(dict, &sep.to_string())
                    };
                    part.append_lookup_suffix(&name);
                }
            }
            let mut phonemes = std::mem::take(&mut part).bytes;
            phonemes.push(0);
            set_word_stress(&mut phonemes, phdata, stress_opts, Some(0), -1, 0);
            out.extend_from_slice(trim_lookup(&phonemes));
            if grouped || left == 0 {
                out.push(PHON_END_WORD);
            }
        }
    }
    out.push(0);
    out
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

    // Exactly zero → the cardinal "zero"; the caller appends the ordinal suffix
    // ("zeroth").  Without this, `ordinal_sub_thousand` emits nothing for 0 and
    // only the bare suffix survives.
    if value == 0 {
        pronunciation.push_pronunciation(&num3_phonemes(dict, 0, false, grammar));
        return (pronunciation, false);
    }

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
                pronunciation.push_pronunciation(&num3_phonemes(
                    dict,
                    hundreds * 100,
                    false,
                    grammar,
                ));
            }
        } else {
            pronunciation.push_pronunciation(&num3_phonemes(dict, hundreds * 100, false, grammar));
        }
    }

    let full_ord = lookup_num_phonemes(dict, &format!("_{tensunits}o"));
    if !full_ord.is_empty() {
        pronunciation.push_lookup_word(&full_ord);
        found_ordinal = true;
    } else if tens >= 2 && units > 0 && grammar.ordinals.compound_cardinal_suffix {
        // German-style: the whole compound cardinal takes the ordinal suffix —
        // 21 → "einundzwanzig" + "ste" = "einundzwanzigste" (not "zwanzig erste").
        pronunciation.push_pronunciation(&num3_phonemes(dict, tensunits, false, grammar));
        // The ordinal suffix is the dot-marker's own suffix (`_#.`), else the
        // generic `_ord` — the same suffix the round-ten ordinals use, so a
        // compound reads consistently with them (21 → "einund" + "zwanzig(s)te").
        let ord_suffix: Vec<u8> = if !suffix_ph.is_empty() {
            suffix_ph.to_vec()
        } else {
            lookup_num_phonemes(dict, "_ord")
        };
        if !ord_suffix.is_empty() {
            pronunciation.append_lookup_suffix(&ord_suffix);
            found_ordinal = true;
        }
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

/// A word of symbols ("+$"): each one's list entry in turn, as C's rules
/// read them, or `None` when one needs English (C then switches the word).
fn glued_symbols(
    word: &str,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    options: &LangOptions,
) -> Option<Vec<u8>> {
    let ctx = LookupCtx {
        lookup_symbol: true,
        dict_condition: options.dict_condition,
        ..Default::default()
    };
    // C's `Lookup`: a plain entry, or the first word of a text one translated
    // as written — found only if that word is in the list.
    let c_lookup = |dict: &Dictionary, c: char, options: &LangOptions| -> (Vec<u8>, bool) {
        let Some(r) = lookup(dict, &c.to_string(), &ctx).filter(|r| r.flags1.found()) else {
            return (Vec::new(), false);
        };
        if r.flags1.textmode() == options.reversed_textmode {
            return (trim_lookup(&r.phonemes).to_vec(), true);
        }
        let text = String::from_utf8_lossy(trim_lookup(&r.phonemes)).into_owned();
        let first = text
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string();
        // (Upper case is not lowered: German "Euro" reads as nothing.)
        if first.chars().any(char::is_uppercase) {
            return (Vec::new(), false);
        }
        let wr = word_to_phonemes(&first, dict, phdata, stress_opts, options);
        (trim_lookup(&wr.phonemes).to_vec(), wr.found_in_list)
    };
    let mut out = Vec::new();
    for c in word.chars() {
        // (The rules read a minus, which says nothing with no digit after it.)
        if c == '-' {
            continue;
        }
        // The rules for the character, else C's `LookupLetter` for running
        // text.
        let mut buf = vec![b' '];
        let mut b = [0u8; 4];
        buf.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
        buf.extend_from_slice(b"  ");
        buf.push(0);
        let (mut vowels, mut stressed) = (0i32, 0i32);
        let r = crate::dictionary::rules::untraced(|| {
            crate::dictionary::rules::translate_rules(
                dict,
                &buf,
                1,
                0,
                0,
                &dict.letter_bits,
                options.dict_condition,
                &mut vowels,
                &mut stressed,
            )
        });
        let ruled: Vec<u8> = r.phonemes.into_iter().take_while(|&b| b != 0).collect();
        if !r.spellword && !ruled.is_empty() {
            out.extend(ruled);
            continue;
        }
        let (ph, found) = c_lookup(dict, c, options);
        if !found && trim_lookup(&lookup_num_phonemes(dict, &format!("_{c}"))).is_empty() {
            // Does English have it?  Then C switches the word.
            if primary_bcp47_subtag(&options.lang) != "en" {
                let en = EN_DICT.with(|d| {
                    d.get_or_init(|| {
                        let dir = DATA_DIR.with(|d| d.borrow().clone())?;
                        Dictionary::load("en", &dir).ok().map(std::rc::Rc::new)
                    })
                    .clone()
                });
                if let Some(en) = en {
                    let en_opts = LangOptions::for_lang("en");
                    if c_lookup(&en, c, &en_opts).1 {
                        return None;
                    }
                }
            }
        }
        out.extend(ph);
    }
    out.push(0);
    set_word_stress(&mut out, phdata, stress_opts, Some(0), -1, 0);
    Some(out)
}

/// A short all-capitals Latin word in a lower-case clause of a language that
/// doesn't use the Latin alphabet: C spells it, letter by letter.
fn is_spelled_abbreviation(tokens: &[Token], k: usize, dict: &Dictionary) -> bool {
    let Some(Token::Word(w)) = tokens.get(k) else {
        return false;
    };
    if dict.letter_bits_offset == 0
        || !(2..4).contains(&w.chars().count())
        || !w.chars().all(|c| c.is_ascii_uppercase())
        || dict_knows_word(w, dict)
    {
        return false;
    }
    let in_clause = |t: &&Token| !matches!(t, Token::ClauseBoundary(_));
    let before = tokens
        .get(..k)
        .unwrap_or_default()
        .iter()
        .rev()
        .take_while(in_clause);
    let after = tokens
        .get(k..)
        .unwrap_or_default()
        .iter()
        .take_while(in_clause);
    let (mut l, mut u) = (0usize, 0usize);
    for t in before.chain(after) {
        if let Token::Word(w) = t {
            l += w.chars().filter(|c| c.is_lowercase()).count();
            u += w.chars().filter(|c| c.is_uppercase()).count();
        }
    }
    l > 3 && u <= l
}

/// C's `TranslateRules` on a word of digits and separators: each digit is its
/// `_d` entry with a pause after every second one, and anything else is read
/// by the rules (and starts the count again).
fn rules_digit_word(
    text: &str,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut count = 0;
    for c in text.chars() {
        if c.is_ascii_digit() {
            out.extend_from_slice(trim_lookup(&lookup_num_phonemes(dict, &format!("_{c}"))));
            count += 1;
            if count >= 2 {
                out.push(crate::phoneme::PHON_PAUSE_NOLINK);
                count = 0;
            }
        } else {
            count = 0;
            let mut buf = vec![b' '];
            let mut b = [0u8; 4];
            buf.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            buf.extend_from_slice(b"  ");
            buf.push(0);
            let (mut vowels, mut stressed) = (0i32, 0i32);
            let r = crate::dictionary::rules::untraced(|| {
                crate::dictionary::rules::translate_rules(
                    dict,
                    &buf,
                    1,
                    0,
                    0,
                    &dict.letter_bits,
                    dict.dict_condition,
                    &mut vowels,
                    &mut stressed,
                )
            });
            if !r.spellword {
                out.extend(r.phonemes.into_iter().take_while(|&b| b != 0));
            }
        }
    }
    out.push(0);
    set_word_stress(&mut out, phdata, stress_opts, Some(0), -1, 0);
    out
}

/// C's `TranslateNumber_1` for an ordinal: the number read with its ordinal
/// entries (`_1o`, `_2Xo`, `_0Co`, `_ord`) and the spoken suffix (`_#st`).
fn c_ordinal_number(
    ordinal: &OrdinalNumber,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    grammar: &NumberGrammar,
) -> Option<WordResult> {
    let flags = grammar.flags?;
    let digits = ordinal.digits.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let mut ord = cardinal::Ordinal::default();
    if let OrdinalMarker::Suffix(suffix) = &ordinal.marker {
        if grammar.ordinals.indicator.as_deref() != Some(suffix.as_str()) {
            ord.suffix = trim_lookup(&lookup_num_phonemes(dict, &format!("_#{suffix}"))).to_vec();
            ord.suffix_x =
                trim_lookup(&lookup_num_phonemes(dict, &format!("_x#{suffix}"))).to_vec();
        }
    }
    let groups = cardinal::groups_with(digits, dict, flags, Some(&ord))?;
    let last = groups.len().saturating_sub(1);
    let mut phonemes = Vec::new();
    for (ix, group) in groups.into_iter().enumerate() {
        let mut part = Pronunciation {
            bytes: group.phonemes,
        };
        let ends_word = part.bytes.last() == Some(&PHON_END_WORD);
        part.bytes.truncate(part.trimmed_len());
        if ix != last && group.pause {
            if ends_word {
                part.bytes.push(PHON_END_WORD);
            }
            part.bytes.push(crate::phoneme::PHON_PAUSE_NOLINK);
        }
        let mut part = part.finish();
        set_word_stress(&mut part, phdata, stress_opts, Some(0), -1, 0);
        phonemes.extend_from_slice(trim_lookup(&part));
    }
    phonemes.push(0);
    Some(WordResult {
        skipwords: 0,
        phonemes,
        dict_flags: 0,
        found_in_list: true,
    })
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
/// For a language without C's number tables.
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
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
    let groups = split_scale_groups_folded(value.into(), dict);
    // For a zero value there is no non-zero group; fall back to the *last* group
    // (the units, scale-less) so "0th" renders "zeroth" instead of dropping the
    // number or attaching a spurious scale word.
    let last_nonzero = groups
        .iter()
        .rposition(|group| group.value != 0)
        .unwrap_or(groups.len().saturating_sub(1));

    for &group in groups.get(..last_nonzero).unwrap_or_default() {
        // These groups all precede the last non-zero group, so a lower non-zero
        // digit always follows — never an exact scale group.
        append_cardinal_group(&mut pronunciation, group, dict, grammar, false, false);
    }

    let final_group = groups[last_nonzero];
    let found_ordinal = if let Some(scale) = final_group.scale {
        append_ordinal_scale(&mut pronunciation, final_group.value, scale, dict, grammar)
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
    Some(WordResult {
        skipwords: 0,
        phonemes,
        dict_flags: 0,
        found_in_list: false,
    })
}

fn translate_number_token(
    token: &NumberToken,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    grammar: &NumberGrammar,
) -> Option<WordResult> {
    match token {
        // A language without number options: C's `TranslateRules` reads the
        // whole thing as one word.
        NumberToken::Decimal {
            integer,
            fractional,
        } if grammar.digits_only => {
            let text = format!("{integer}{}{fractional}", grammar.decimal_separator);
            Some(WordResult {
                skipwords: 0,
                phonemes: rules_digit_word(&text, dict, phdata, stress_opts),
                dict_flags: 0,
                found_in_list: false,
            })
        }
        NumberToken::Ordinal(ordinal) if grammar.flags.is_some() => {
            c_ordinal_number(ordinal, dict, phdata, stress_opts, grammar)
        }
        NumberToken::Ordinal(ordinal) => {
            try_ordinal_number(ordinal, dict, phdata, stress_opts, grammar)
                // The number words come from `_1`…`_20`/`_100` list entries, so C's
                // `TranslateInteger` sets `FLAG_FOUND` on the whole token and the
                // stress-conditioned changes don't apply to them.  (The languages
                // whose numbers this port spells out as *text* instead — Portuguese
                // — go through the rules and keep `found_in_list = false`.)
                .map(|wr| WordResult {
                    found_in_list: true,
                    ..wr
                })
        }
        // C looks a whole number up as a word first: Marathi has one for 100.
        NumberToken::Cardinal(digits)
            if digits.len() > 1
                && digits.len() <= 4
                && !lookup_num_phonemes(dict, digits).is_empty() =>
        {
            let mut phonemes = lookup_num_phonemes(dict, digits);
            set_word_stress(&mut phonemes, phdata, stress_opts, Some(0), -1, 0);
            Some(WordResult {
                skipwords: 0,
                phonemes,
                dict_flags: 0,
                found_in_list: true,
            })
        }
        // C: a long number, or one with a leading zero, is a code.
        NumberToken::Cardinal(digits)
            if digits.len() > 3
                && digits.bytes().all(|b| b.is_ascii_digit())
                && (digits.starts_with('0') || digits.len() > grammar.max_digits.min(32))
                || digits.len() > 1
                    && grammar.digits_only
                    && digits.bytes().all(|b| b.is_ascii_digit()) =>
        {
            // (C's `TranslateRules` reads them, so they aren't "found".)
            Some(WordResult {
                skipwords: 0,
                phonemes: individual_digits(digits, dict, phdata, stress_opts, grammar),
                dict_flags: 0,
                found_in_list: false,
            })
        }
        // C's `TranslateWord3` splits a number of five or more digits into one
        // word per group, so each is stressed on its own; so is a shorter one
        // written with a separator.
        NumberToken::Cardinal(digits)
            if digits.len() > 4 || digits.bytes().any(|b| !b.is_ascii_digit()) =>
        {
            let mut phonemes = Vec::new();
            let plain: String = digits.chars().filter(char::is_ascii_digit).collect();
            let groups = if plain.len() == digits.len() {
                cardinal_groups(plain.parse().ok()?, dict, grammar)
            } else {
                let flags = grammar.flags.or(cardinal::NumberFlags::for_lang("en"))?;
                cardinal::groups_split(&plain, dict, flags)?
                    .into_iter()
                    .map(|g| (Pronunciation { bytes: g.phonemes }, g.pause))
                    .collect()
            };
            let last = groups.len().saturating_sub(1);
            for (ix, (group, pause)) in groups.into_iter().enumerate() {
                let mut part = group;
                // (A group ending in a word break — Vietnamese "không trăm" —
                // keeps it before the pause: `x,o1NtS'a1m_| _!_|`.)
                let ends_word = part.bytes.last() == Some(&PHON_END_WORD);
                part.bytes.truncate(part.trimmed_len());
                // A short pause separates the groups (unless `NUM_NOPAUSE`), so
                // nothing assimilates across them.
                if ix != last && pause {
                    if ends_word {
                        part.bytes.push(PHON_END_WORD);
                    }
                    part.bytes.push(crate::phoneme::PHON_PAUSE_NOLINK);
                }
                let mut part = part.finish();
                set_word_stress(&mut part, phdata, stress_opts, Some(0), -1, 0);
                phonemes.extend_from_slice(trim_lookup(&part));
            }
            phonemes.push(0);
            Some(WordResult {
                skipwords: 0,
                phonemes,
                dict_flags: 0,
                found_in_list: true,
            })
        }
        _ => {
            let mut phonemes = number_token_to_phonemes(token, dict, grammar)?;
            set_word_stress(&mut phonemes, phdata, stress_opts, Some(0), -1, 0);
            Some(WordResult {
                skipwords: 0,
                phonemes,
                dict_flags: 0,
                found_in_list: true,
            })
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
/// Part-of-speech expectation for a word, derived from the *preceding* word (a
/// `FLAG_VERBF`/`NOUNF`/`PASTF` function word like "to"/"the"/"had").  Selects
/// the `$verb`/`$noun`/`$past` homograph pronunciation ("to lˈɪv" vs "lˈaɪv").
#[derive(Clone, Copy, Debug, Default)]
pub struct PosExpect {
    pub verb: bool,
    pub noun: bool,
    pub past: bool,
    /// A verb with an `-s` suffix is expected (C's `expect_verb_s`).
    pub verb_s: bool,
    /// This word is the last one in its clause.
    ///
    /// Dictionary entries flagged `$atend` only apply there — the English
    /// article "a" is `eI` at a clause end but `a#` in running text, and C
    /// gates it on `word_end < translator->clause_end`.
    pub at_clause_end: bool,
    /// Whether each of the next words of the clause ends it, for a
    /// multi-word entry's `$atend` (C compares the end of the words it
    /// matched with the clause's): English `(I am) %aIam $atend`.
    pub at_end_after: [bool; 8],
    /// This word starts the clause, for `$atstart` entries.
    pub at_clause_start: bool,
    /// Its clause ends a sentence, for `$sentence` entries.
    pub in_sentence: bool,
    /// The word as written begins with a capital, for `$capital` entries.
    pub first_upper: bool,
    /// The word as written is all capitals, for `$allcaps` entries.
    ///
    /// English "I" depends on this: its dictionary entry carries
    /// `$u+ $strend $verbf $allcaps $only`, so without the flag the lookup falls
    /// through to a flagless entry and the pronoun keeps primary stress.
    pub all_upper: bool,
    /// The letters of this *same* word that have already been dealt with, for a
    /// rule pre-context that reaches back past the fragment being translated.
    /// NUL-padded, like [`Self::next_word`].
    ///
    /// C translates a word by advancing a pointer through the clause buffer, so
    /// after spelling the unpronounceable letters of `s's` the leftover `'s` is
    /// still matched with the `s` in front of it — which is what picks
    /// `s) 's` → `I2z` over the bare `'s` → `z`.
    pub prev_letters: [u8; 8],
    /// The end of the previous word in the clause, NUL-padded, for a rule
    /// pre-context that reaches across the space (Indonesian `A_) i`).
    pub prev_word: [u8; 8],
    /// The first few UTF-8 bytes of the *following* word, NUL-padded.
    ///
    /// A rule's post-context may reach past the word boundary — Portuguese's
    /// `_) o (_C` reads the article "o" as `U` when the next word starts with a
    /// consonant — and C can do that because its rule matcher walks the clause
    /// buffer.  The port translates a word at a time, so the next word's opening
    /// is carried here and appended to the rules buffer.
    pub next_word: [u8; 8],
    /// A hyphen joins this word to the one before (C's `FLAG_HYPHEN`), so a
    /// rule pre-context `-` matches the space in front of it: French
    /// `-c) e (` silences the e of "est-ce".
    pub hyphen_before: bool,
    /// A hyphen joins this word to the next (`FLAG_HYPHEN_AFTER`), for a
    /// post-context `-`.
    pub hyphen_after: bool,
    /// A dot follows the word (C's `FLAG_HAS_DOT`), for `$hasdot` entries:
    /// Italian "Dr." is "dottor".
    pub has_dot: bool,
    /// The words after this one in the clause, lower case, each followed by a
    /// space and NUL-padded — C's `word2`, which a multi-word dictionary
    /// entry (`(parce que)`, `(celle là)`) compares with the rest of its key.
    pub following: [u8; 32],
}

/// A [`PosExpect`] carrying only the opening of the word that follows, so a
/// composed multi-word number gets the same cross-boundary rule contexts a typed
/// phrase does (French "vingt et unième" keeps the liaison `t` of "vingt").
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
fn expect_before(next: Option<&str>) -> PosExpect {
    let mut e = PosExpect::default();
    if let Some(nw) = next {
        let b = nw.as_bytes();
        let n = (0..=b.len().min(8))
            .rev()
            .find(|&k| nw.is_char_boundary(k))
            .unwrap_or(0);
        e.next_word[..n].copy_from_slice(b.get(..n).unwrap_or_default());
    }
    e
}

/// The part-of-speech expectation a word inherits from the preceding word.
/// English only; other languages get the default (no expectation).  Kept
/// conservative — only high-confidence verb/past triggers — since a false
/// positive mispronounces an otherwise-correct default.
/// C's `expect_verb`, `expect_verb_s`, `expect_noun` and `expect_past`: how
/// many words more a `$verbf`, `$verbsf`, `$nounf` or `$pastf` word before
/// makes the `$verb`, `$noun` and `$past` entries apply ("the pr'0dju:s",
/// Portuguese "seu consolo").  They reset at each clause.
#[derive(Debug, Default, Clone, Copy)]
struct Expecting {
    verb: u8,
    verb_s: u8,
    noun: u8,
    past: u8,
}

impl Expecting {
    /// After a word with the second flags word `flags2` (`TranslateWord3`).
    fn after_word(&mut self, flags2: u32) {
        const FLAG_VERBF: u32 = 0x1;
        const FLAG_VERBSF: u32 = 0x2;
        const FLAG_NOUNF: u32 = 0x4;
        const FLAG_PASTF: u32 = 0x8;
        const FLAG_VERB_EXT: u32 = 0x100;
        if flags2 & FLAG_PASTF != 0 {
            self.past = 3;
            self.verb = 0;
            self.noun = 0;
        } else if flags2 & FLAG_VERBF != 0 {
            self.verb = 2;
            self.verb_s = 0;
            self.noun = 0;
        } else if flags2 & FLAG_VERBSF != 0 {
            self.verb = 0;
            self.verb_s = 2;
            self.past = 0;
            self.noun = 0;
        } else if flags2 & FLAG_NOUNF != 0 {
            self.noun = 2;
            self.verb = 0;
            self.verb_s = 0;
            self.past = 0;
        }
        if flags2 & FLAG_VERB_EXT == 0 {
            self.verb = self.verb.saturating_sub(1);
            self.verb_s = self.verb_s.saturating_sub(1);
            self.noun = self.noun.saturating_sub(1);
            self.past = self.past.saturating_sub(1);
        }
    }
}

pub fn word_to_phonemes(
    word: &str,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    lang_opts: &LangOptions,
) -> WordResult {
    let r = word_to_phonemes_inner(
        word,
        dict,
        phdata,
        stress_opts,
        lang_opts,
        0,
        PosExpect::default(),
    );
    emoji_parts_fallback(
        r,
        word,
        dict,
        phdata,
        stress_opts,
        lang_opts,
        PosExpect::default(),
    )
}

/// Speak an emoji sequence that has no dictionary entry of its own as its
/// **parts**: the base emoji's name followed by each modifier's name.
///
/// Upstream 1.53.0 ("added skin tone emoji support: sequences are spoken as the
/// base name plus the modifier names") — `👍🏽` is one token so it matches a
/// multi-codepoint entry when one exists, but CLDR does not give every
/// base+tone combination a name, and without this fallback the whole token was
/// **silently dropped** (`👍🏽` produced no output at all).
///
/// Only runs when the whole-sequence lookup produced nothing, so sequences that
/// *do* have an entry (families, flags, subdivision tag flags) are unaffected.
fn emoji_parts_fallback(
    result: WordResult,
    word: &str,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    lang_opts: &LangOptions,
    expect: PosExpect,
) -> WordResult {
    // "Found but empty" counts as nothing spoken: a base+tone key can exist in
    // the dictionary with an empty body (`👍🏽` looks up to a single NUL byte).
    let spoke_something = result.phonemes.iter().any(|&b| b != 0);
    if spoke_something || !word.chars().any(is_skin_tone) {
        return result;
    }
    let base: String = word.chars().filter(|&c| !is_skin_tone(c)).collect();
    let mut parts: Vec<String> = Vec::new();
    if !base.is_empty() {
        parts.push(base);
    }
    parts.extend(word.chars().filter(|&c| is_skin_tone(c)).map(String::from));

    let mut phonemes: Vec<u8> = Vec::new();
    let mut dict_flags = 0;
    let mut found_in_list = false;
    for part in parts {
        let r = word_to_phonemes_inner(&part, dict, phdata, stress_opts, lang_opts, 0, expect);
        if !r.phonemes.iter().any(|&b| b != 0) {
            continue;
        }
        if !phonemes.is_empty() {
            phonemes.push(crate::phoneme::PHON_END_WORD);
        }
        phonemes.extend(r.phonemes);
        dict_flags |= r.dict_flags;
        found_in_list |= r.found_in_list;
    }
    if phonemes.is_empty() {
        return result;
    }
    WordResult {
        skipwords: 0,
        phonemes,
        dict_flags,
        found_in_list,
    }
}

/// Like [`word_to_phonemes`] but with a part-of-speech expectation, so a
/// homograph after a trigger word takes its `$verb`/`$noun`/`$past` form.
pub fn word_to_phonemes_pos(
    word: &str,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    lang_opts: &LangOptions,
    expect: PosExpect,
) -> WordResult {
    let r = word_to_phonemes_inner(word, dict, phdata, stress_opts, lang_opts, 0, expect);
    emoji_parts_fallback(r, word, dict, phdata, stress_opts, lang_opts, expect)
}

/// Decompose Korean Hangul syllables (U+AC00–U+D7A3) into conjoining jamo — a
/// leading consonant (U+1100+L), a vowel (U+1161+V), and an optional trailing
/// consonant (U+11A7+T) — which is the form the ko rules match.  Mirrors the
/// Hangul handling in C's `TranslateChar`.  Returns `None` (no allocation) when
/// the word contains no Hangul syllable.
fn decompose_hangul(word: &str) -> Option<String> {
    const SBASE: u32 = 0xAC00;
    const LBASE: u32 = 0x1100;
    const VBASE: u32 = 0x1161;
    const TBASE: u32 = 0x11A7;
    const VCOUNT: u32 = 21;
    const TCOUNT: u32 = 28;
    let is_hangul = |c: char| (SBASE..=0xD7A3).contains(&(c as u32));
    if !word.chars().any(is_hangul) {
        return None;
    }
    let mut out = String::with_capacity(word.len() + 6);
    for c in word.chars() {
        let cp = c as u32;
        if is_hangul(c) {
            let s = cp - SBASE;
            let (l, v, t) = (
                s / (VCOUNT * TCOUNT),
                (s % (VCOUNT * TCOUNT)) / TCOUNT,
                s % TCOUNT,
            );
            // Leading ㅇ (choseong ieung, index 11) is a silent placeholder for a
            // vowel-initial syllable — it never carries the "ng" sound (that is
            // only the *trailing* ᆼ), so omit it rather than emit the jamo's
            // letter-name pronunciation.
            if l != 11 {
                out.extend(char::from_u32(LBASE + l));
            }
            out.extend(char::from_u32(VBASE + v));
            if t != 0 {
                out.extend(char::from_u32(TBASE + t));
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Internal marker in `rule_word_flags` meaning "we are already expanding a
/// TEXTMODE replacement", to stop a replacement from recursively re-expanding.
/// Bit 30 — unused by the rules engine's word flags.
const FLAG_IN_TEXTMODE: u32 = 0x4000_0000;

thread_local! {
    /// The text being translated is a switched word of a clause that goes on
    /// after it.
    static CLAUSE_CONTINUES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// A word or number follows the switched phrase in the main clause, so
    /// none of the phrase's words is at the clause's end (`$atend`).
    static PHRASE_FOLLOWED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// A word of the main clause before the switched phrase has a stress
    /// (C's `any_stressed_words`, which `$unstressend` reads).
    static STRESSED_BEFORE_PHRASE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The second flags word of the last dictionary lookup of a whole word
    /// (`$verbf`, `$nounf`, …), for what the words after it expect.
    static WORD_FLAGS2: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// The `$alt` flags of a word whose prefix was taken off, which its stem's
    /// rules still see (Dutch "gestage $alt": `age (_$` is `a:Q@`).
    static INHERITED_ALT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// The clauses (by number) that end in an abbreviation's dot, which C
    /// speaks with no intonation (`tone_out = 4`), so no stress is added.
    static NO_TONE_CLAUSES: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
    /// A letter being spelled has more letters after it: C's `LookupLetter`
    /// then puts `RULE_SPELLING` after it for the rules to see.
    static LETTER_FOLLOWS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The switched phrase being read ends its clause (so `$strend` applies).
    static PHRASE_ENDS_CLAUSE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// A replacement contained a bracket or quote, whose pause C gives the next
    /// word of the clause (French `∞`, "symbole de l’infini").
    static BRACKET_IN_REPLACEMENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// A symbol's reading began with a word the rules sent to another
    /// language, so the symbol itself is looked up there as a character.
    static SYMBOL_SWITCHED_BY_RULES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The phoneme table C has left selected for the rest of the clause after
    /// switching to a language that could not read a letter (the 0xa700
    /// "Korean" range takes in Javanese and others): the words after it are
    /// stressed against that table, and their phonemes it lacks become schwa
    /// (English `x ꦄ y` reads "y" as `w'@`).
    static STALE_TABLE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// The data directory of the translation in progress.
    static DATA_DIR: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
    /// The number glued before a switched phrase, which its rules see as the
    /// word before (English `1_) st` in a Russian "1st").
    static SWITCH_PREV: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// The English dictionary, for C's `LookupLetter` check of whether
    /// English has a character.
    static EN_DICT: std::cell::OnceCell<Option<std::rc::Rc<Dictionary>>> = const { std::cell::OnceCell::new() };
    /// The `LOPT_REDUCE` of the clause a switched word is read for: C's
    /// stress conditions take it from the clause's translator.
    static CLAUSE_REDUCE: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
    /// Where the [`Token::Dot`]s were, before they became spaces.
    static DOTS: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
    /// How many `$textmode` replacements deep the current word is.
    static TEXTMODE_DEPTH: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// Ranges Rust's `is_alphabetic` counts as letters and C's bundled Unicode
/// data (`ucd_isalpha`, an older version) doesn't — the Shan tone marks among
/// them, which end a word in C.
const UCD_NOT_ALPHA: &[(u32, u32)] = &[
    (0x363, 0x36f),
    (0x558, 0x558),
    (0x58b, 0x58c),
    (0x5c8, 0x5c9),
    (0x870, 0x887),
    (0x889, 0x88f),
    (0x897, 0x897),
    (0x8b5, 0x8b5),
    (0x8be, 0x8c9),
    (0xc04, 0xc04),
    (0xc5c, 0xc5d),
    (0xcdc, 0xcdd),
    (0xcf3, 0xcf3),
    (0xd04, 0xd04),
    (0xd81, 0xd81),
    (0xe86, 0xe86),
    (0xe89, 0xe89),
    (0xe8c, 0xe8c),
    (0xe8e, 0xe93),
    (0xe98, 0xe98),
    (0xea0, 0xea0),
    (0xea8, 0xea9),
    (0xeac, 0xeac),
    (0xf82, 0xf83),
    (0x1063, 0x1064),
    (0x1069, 0x106d),
    (0x1087, 0x108d),
    (0x108f, 0x108f),
    (0x109a, 0x109b),
    (0x170d, 0x170d),
    (0x171f, 0x171f),
    (0x1abf, 0x1ac0),
    (0x1acc, 0x1ace),
    (0x1b4c, 0x1b4c),
    (0x1c36, 0x1c36),
    (0x1c89, 0x1c8a),
    (0x1cfa, 0x1cfa),
    (0x1dd3, 0x1de6),
    (0x208f, 0x208f),
    (0x209d, 0x209f),
    (0x2c2f, 0x2c2f),
    (0x2c5f, 0x2c5f),
    (0x31bb, 0x31bf),
    (0x4db6, 0x4dbf),
    (0x9ff0, 0x9fff),
    (0xa7ba, 0xa7dd),
    (0xa7e2, 0xa7e2),
    (0xa7f1, 0xa7f6),
    (0xa802, 0xa802),
    (0xa80b, 0xa80b),
    (0xa8ff, 0xa8ff),
    (0xa9e5, 0xa9e5),
    (0xaa7b, 0xaa7d),
    (0xab66, 0xab69),
    (0xab6c, 0xab6d),
    (0x10570, 0x1057a),
    (0x1057c, 0x1058a),
    (0x1058c, 0x10592),
    (0x10594, 0x10595),
    (0x10597, 0x105a1),
    (0x105a3, 0x105b1),
    (0x105b3, 0x105b9),
    (0x105bb, 0x105bc),
    (0x105c0, 0x105f3),
    (0x10780, 0x10785),
    (0x10787, 0x107b0),
    (0x107b2, 0x107bf),
    (0x10940, 0x10959),
    (0x10d4a, 0x10d65),
    (0x10d69, 0x10d69),
    (0x10d6f, 0x10d85),
    (0x10e80, 0x10ea9),
    (0x10eab, 0x10eac),
    (0x10eb0, 0x10eb1),
    (0x10ec2, 0x10ec7),
    (0x10ecb, 0x10ecd),
    (0x10ed9, 0x10eee),
    (0x10ef3, 0x10ef3),
    (0x10ef5, 0x10ef5),
    (0x10ef7, 0x10ef8),
    (0x10efa, 0x10efc),
    (0x10f70, 0x10f81),
    (0x10fb0, 0x10fc4),
    (0x10fe0, 0x10ff6),
    (0x11071, 0x11075),
    (0x11080, 0x11081),
    (0x110c2, 0x110c2),
    (0x11147, 0x11147),
    (0x111ce, 0x111cf),
    (0x1123f, 0x11241),
    (0x11380, 0x11389),
    (0x1138b, 0x1138b),
    (0x1138e, 0x1138e),
    (0x11390, 0x113b5),
    (0x113b7, 0x113c0),
    (0x113c2, 0x113c2),
    (0x113c5, 0x113c5),
    (0x113c7, 0x113ca),
    (0x113cc, 0x113cd),
    (0x113d1, 0x113d1),
    (0x113d3, 0x113d3),
    (0x1145f, 0x11461),
    (0x116b8, 0x116b8),
    (0x11740, 0x11746),
    (0x11900, 0x11906),
    (0x11909, 0x11909),
    (0x1190c, 0x11913),
    (0x11915, 0x11916),
    (0x11918, 0x11935),
    (0x11937, 0x11938),
    (0x1193b, 0x1193c),
    (0x1193f, 0x11942),
    (0x119a0, 0x119a7),
    (0x119aa, 0x119d7),
    (0x119da, 0x119df),
    (0x119e1, 0x119e1),
    (0x119e3, 0x119e4),
    (0x11a84, 0x11a85),
    (0x11ab0, 0x11abf),
    (0x11b0a, 0x11b0a),
    (0x11b60, 0x11b67),
    (0x11bc0, 0x11be0),
    (0x11db0, 0x11ddb),
    (0x11df0, 0x11df1),
    (0x11f00, 0x11f10),
    (0x11f12, 0x11f3a),
    (0x11f3e, 0x11f40),
    (0x11fb0, 0x11fb0),
    (0x1246f, 0x1246f),
    (0x12475, 0x1247f),
    (0x12550, 0x12686),
    (0x12f90, 0x12ff0),
    (0x1342f, 0x1342f),
    (0x13441, 0x13446),
    (0x13460, 0x143fa),
    (0x16100, 0x1612e),
    (0x16a70, 0x16abe),
    (0x16d40, 0x16d6c),
    (0x16ea0, 0x16eb8),
    (0x16ebb, 0x16ed3),
    (0x16f45, 0x16f4a),
    (0x16f4f, 0x16f4f),
    (0x16f7f, 0x16f87),
    (0x16f8f, 0x16f92),
    (0x16fe3, 0x16fe3),
    (0x16ff0, 0x16ff6),
    (0x187f2, 0x187ff),
    (0x18af3, 0x18cda),
    (0x18cff, 0x18d20),
    (0x18d80, 0x18df2),
    (0x18e00, 0x19191),
    (0x191a0, 0x191d2),
    (0x1aff0, 0x1aff3),
    (0x1aff5, 0x1affb),
    (0x1affd, 0x1affe),
    (0x1b11f, 0x1b128),
    (0x1b132, 0x1b132),
    (0x1b150, 0x1b152),
    (0x1b155, 0x1b155),
    (0x1b164, 0x1b168),
    (0x1d6a6, 0x1d6a6),
    (0x1df00, 0x1df81),
    (0x1df90, 0x1df96),
    (0x1dfcd, 0x1dfff),
    (0x1e030, 0x1e06d),
    (0x1e08f, 0x1e08f),
    (0x1e100, 0x1e12c),
    (0x1e137, 0x1e13d),
    (0x1e14e, 0x1e14e),
    (0x1e290, 0x1e2ad),
    (0x1e2c0, 0x1e2eb),
    (0x1e4d0, 0x1e4eb),
    (0x1e5d0, 0x1e5ed),
    (0x1e5f0, 0x1e5f0),
    (0x1e6c0, 0x1e6de),
    (0x1e6e0, 0x1e6f5),
    (0x1e6fe, 0x1e6ff),
    (0x1e7e0, 0x1e7e6),
    (0x1e7e8, 0x1e7eb),
    (0x1e7ed, 0x1e7ee),
    (0x1e7f0, 0x1e7fe),
    (0x1e94b, 0x1e94b),
    (0x2a6d7, 0x2a6df),
    (0x2b735, 0x2b73f),
    (0x2b81e, 0x2b81e),
    (0x2cea2, 0x2cead),
    (0x2ebf0, 0x2ee5d),
];

/// Ranges C's `ucd_isalpha` counts as letters and Rust doesn't.
const UCD_ALPHA: &[(u32, u32)] = &[(0x135f, 0x135f), (0x16b30, 0x16b36)];

/// C's `ucd_isalpha` (`iswalpha`).
fn ucd_isalpha(c: char) -> bool {
    let u = c as u32;
    let within = |t: &[(u32, u32)]| {
        t.binary_search_by(|&(a, b)| {
            if u < a {
                std::cmp::Ordering::Greater
            } else if u > b {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
    };
    if within(UCD_ALPHA) {
        return true;
    }
    c.is_alphabetic() && !within(UCD_NOT_ALPHA)
}

/// C's `IsAlpha`: a letter, or one of the marks and symbols C treats as one
/// (Indic vowel signs and viramas, Hebrew and Arabic vowel marks, combining
/// accents, Tibetan, jamo, braille, CJK, emoji).
fn c_is_alpha(c: char) -> bool {
    if ucd_isalpha(c) {
        return true;
    }
    let u = c as u32;
    if u < 0x300 {
        return false;
    }
    if (0x901..=0xdf7).contains(&u) {
        return (u & 0x7f) < 0x64 || matches!(u, 0xa70 | 0xa71) || (0xd7a..=0xd7f).contains(&u);
    }
    (0x5b0..=0x5c2).contains(&u)
        || u == 0x605
        || u == 0x670
        || (0x64b..=0x65e).contains(&u)
        || (0x300..=0x36f).contains(&u)
        || (0xf40..=0xfbc).contains(&u)
        || (0x1100..=0x11ff).contains(&u)
        || (0x2800..=0x28ff).contains(&u)
        || (u > 0x3040 && u <= 0xa700)
        || is_emoji_char(c)
}

/// C's `SegmentReplacement`: replacement text split into the words the clause
/// tokenizer would make of it.  Each ideograph is a word for the languages
/// that split them, a hyphen between letters is a break, and a non-letter
/// mark ends a word (a Thai tone mark, a Myanmar asat) and a letter after one
/// starts the next.
fn segment_replacement(text: &str, lang: &str) -> Vec<String> {
    let base = primary_bcp47_subtag(lang);
    let ideographs = matches!(base, "cmn" | "yue" | "zh" | "hak");
    let alt = alt_alphabet(lang).0;
    let within: &[char] = match base {
        "ca" => &['\'', '\u{b7}'],
        "jbo" => &['.', ',', '\'', '\u{2c8}'],
        _ => &['\''],
    };
    let ideographic =
        |c: char| alphabet_from_char(c).is_some_and(|a| a.1 == alt && a.3 & AL_IDEOGRAPHS != 0);
    let split = |c: char, prev: char| {
        (ideographs && (c as u32 > 0x3040 || prev as u32 > 0x3040))
            || (alt != 0 && (ideographic(c) || ideographic(prev)))
    };
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() * 2);
    let mut prev = '\0';
    let mut k = 0;
    while let Some(&c) = chars.get(k) {
        let mut space = false;
        if c_is_alpha(prev) {
            if c_is_alpha(c) {
                space = split(c, prev);
            } else if c == '-' && chars.get(k + 1).is_some_and(|&n| c_is_alpha(n)) {
                out.push(' ');
                prev = ' ';
                k += 1;
                continue;
            } else if !c.is_whitespace() && c as u32 >= 0x80 && !within.contains(&c) {
                space = true;
            }
        } else if prev as u32 >= 0x80
            && !prev.is_whitespace()
            && c_is_alpha(c)
            && !within.contains(&prev)
        {
            space = true;
        }
        if space {
            out.push(' ');
        }
        out.push(c);
        prev = c;
        k += 1;
    }
    out.split_whitespace().map(str::to_string).collect()
}

/// The rules meet a letter they have no rule for, in any script.
fn rules_spell(word: &str, dict: &Dictionary, lang_opts: &LangOptions) -> bool {
    if word.is_empty() || dict.rules().is_empty() {
        return false;
    }
    let mut buf = Vec::with_capacity(word.len() + 3);
    buf.push(b' ');
    buf.extend_from_slice(word.as_bytes());
    buf.push(b' ');
    buf.push(0);
    let (mut vowels, mut stressed) = (0i32, 0i32);
    crate::dictionary::rules::untraced(|| {
        crate::dictionary::rules::translate_rules(
            dict,
            &buf,
            1,
            0,
            0,
            &dict.letter_bits,
            lang_opts.dict_condition,
            &mut vowels,
            &mut stressed,
        )
    })
    .spellword
}
/// `$unstressend` (dictionary flag bit 11): the word loses its stress when it ends
/// the clause.  C's `FLAG_UNSTRESS_END`, the counterpart of `FLAG_STRESS_END`.
const FLAG_UNSTRESS_END: u32 = 1 << 11;

/// Peel a chain of stacked suffixes.
///
/// Mirrors the `while (more_suffixes)` loop in C's `TranslateWord3()`: while the
/// suffix just removed is marked `SUFX_M`, re-run the rules on the stem and, if
/// that finds another (non-prefix) suffix, remove it too.  Each newly found
/// suffix's phonemes go *before* those collected so far, since the loop walks
/// outside-in.
///
/// Returns the innermost stem still to translate, its `end_type`, the
/// accumulated suffix phonemes, and the stem phonemes of the last
/// re-translation in the chain.  Without this, `s's's` lost the stem and read
/// `s'I2zI2z` instead of `'EsI2zI2z`.
#[allow(clippy::too_many_arguments)]
fn peel_stacked_suffixes(
    word: &str,
    result: &crate::dictionary::rules::RulesResult,
    dict: &Dictionary,
    letter_bits: &[u8; 256],
    lang_opts: &LangOptions,
    phdata: &PhonemeData,
) -> PeeledSuffixes {
    /// Same bound as C's prefix loop; no real word stacks this many suffixes.
    const MAX_SUFFIXES: usize = 50;

    let mut cur_word = word.to_string();
    let mut cur_end_type = result.end_type;
    let mut end_phonemes: Vec<u8> = result.end_phonemes.to_vec();
    let mut stem_phonemes: Option<Vec<u8>> = None;

    for _ in 0..MAX_SUFFIXES {
        if cur_end_type & SUFX_M == 0 {
            break;
        }
        let Some((stem, _, stem_word_flags)) =
            remove_standard_suffix(&cur_word, cur_end_type, dict)
        else {
            break;
        };

        let mut stem_buf = Vec::with_capacity(stem.len() + 3);
        stem_buf.push(b' ');
        stem_buf.extend_from_slice(stem.as_bytes());
        stem_buf.push(b' ');
        stem_buf.push(0);
        let (mut vc, mut sc) = (0i32, 0i32);
        let stem_rules = translate_rules_phdata(
            dict,
            &stem_buf,
            1,
            stem_word_flags,
            0,
            letter_bits,
            lang_opts.dict_condition,
            &mut vc,
            &mut sc,
            Some(phdata),
        );

        if stem_rules.end_type == 0
            || stem_rules.end_type & SUFX_P != 0
            || stem_rules.suffix_start <= 1
        {
            break;
        }

        // This suffix is inside the ones already collected.
        let mut merged: Vec<u8> = stem_rules.end_phonemes.to_vec();
        // Drop the NUL terminator between the two runs, if present.
        if let Some(pos) = merged.iter().position(|&b| b == 0) {
            merged.truncate(pos);
        }
        merged.extend_from_slice(&end_phonemes);
        end_phonemes = merged;

        // What this re-translation made of the stem *without* its own ending.
        // `SUFX_Q` ("don't retranslate") on that ending means the caller must
        // keep exactly this, so it cannot be recomputed later.
        stem_phonemes = Some(
            stem_rules
                .phonemes
                .iter()
                .copied()
                .take_while(|&b| b != 0)
                .collect(),
        );
        cur_word = stem;
        cur_end_type = stem_rules.end_type;
    }

    PeeledSuffixes {
        word: cur_word,
        end_type: cur_end_type,
        end_phonemes,
        stem_phonemes,
    }
}

/// What [`peel_stacked_suffixes`] found.
struct PeeledSuffixes {
    /// The innermost stem still to translate.
    word: String,
    /// That stem's own ending type.
    end_type: u32,
    /// Every suffix's phonemes, innermost first.
    end_phonemes: Vec<u8>,
    /// The stem phonemes produced by the last re-translation in the chain, if
    /// the chain ran at all.
    stem_phonemes: Option<Vec<u8>>,
}

/// Whether `code` is a **tone phoneme** — a `phSTRESS`-type entry that carries
/// its own program, as Mandarin's `55`/`35`/`214`/`51`/`11` do.  Plain stress
/// markers are the same type but have no program and `std_length <= 4`.
fn is_tone_phoneme(code: u8, phdata: &PhonemeData) -> bool {
    matches!(phdata.get(code), Some(ph) if ph.typ == 1 && ph.program != 0)
}

/// Attach each tone phoneme to a syllable's vowel.
///
/// C keeps the tone as a *property* of the vowel (`PHONEME_LIST.tone_ph`), so a
/// syllable carries exactly one tone no matter where the letter-to-sound rules
/// emitted it.  This port keeps tones as ordinary list entries, so they are
/// moved next to the vowel they belong to and any extras are dropped:
///
/// * a tone written after the vowel (Mandarin `ni3`) stays with that vowel, and
///   is pulled back over the syllable's coda — `wen2` is `w'u@35n`, not
///   `w'u@n35`;
/// * a tone written before the vowel (Thai marks the syllable ahead of it)
///   attaches to the following vowel — `kh,a5?…`, not `kh5,a?…`;
/// * a second tone for a vowel that already has one replaces it.
///
/// No-op for languages whose tables have no tone phonemes.
fn reorder_tone_phonemes(phonemes: &mut Vec<u8>, phdata: &PhonemeData) {
    if !phonemes
        .iter()
        .take_while(|&&c| c != 0)
        .any(|&c| is_tone_phoneme(c, phdata))
    {
        return;
    }
    let is_vowel = |c: u8| matches!(phdata.get(c), Some(ph) if ph.typ == 2 /* phVOWEL */);
    let is_break = |c: u8| c == 0 || c == crate::phoneme::PHON_END_WORD;

    let mut out: Vec<u8> = Vec::with_capacity(phonemes.len());
    // The most recent vowel in `out` and whether it already carries a tone —
    // one tone per syllable, as in C where the tone is a field on the vowel.
    let mut last_vowel: Option<usize> = None;
    let mut last_vowel_toned = false;
    let mut pending_tone: Option<u8> = None;

    for &code in phonemes.iter() {
        // (C's `prev_vowel` runs on across the word breaks inside one
        // translated word: a Thai replacement's `w2` gives its tone to the
        // vowel before it.)
        if code == 0 {
            out.push(code);
            break;
        }
        if is_break(code) {
            out.push(code);
            continue;
        }
        if is_tone_phoneme(code, phdata) {
            match last_vowel {
                // Pull the tone back onto this syllable's vowel, over any coda.
                Some(v) if !last_vowel_toned => {
                    out.insert(v + 1, code);
                    last_vowel_toned = true;
                }
                // The last one counts.
                Some(v) => out.put(v + 1, code),
                // Written ahead of the syllable (Thai): hold it for the vowel.
                None => {
                    pending_tone = Some(code);
                }
            }
            continue;
        }
        // A length mark belongs with its vowel: the tone follows it, `s'i:2`.
        if code == crate::phoneme::PHON_LENGTHEN {
            if let Some(v) = last_vowel {
                if out.len() == v + 1 {
                    out.push(code);
                    last_vowel = Some(v + 1);
                    continue;
                }
                if last_vowel_toned && out.len() == v + 2 {
                    out.insert(v + 1, code);
                    last_vowel = Some(v + 1);
                    continue;
                }
            }
        }
        out.push(code);
        if is_vowel(code) {
            last_vowel = Some(out.len() - 1);
            last_vowel_toned = false;
            if let Some(tone) = pending_tone.take() {
                out.push(tone);
                last_vowel_toned = true;
            }
        }
    }
    *phonemes = out;
}

fn word_to_phonemes_inner(
    word: &str,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    lang_opts: &LangOptions,
    rule_word_flags: u32,
    expect: PosExpect,
) -> WordResult {
    // Apply the rules file's `.replace` table before dict/rule lookup, exactly
    // as C does per-character in `TranslateChar`.  Japanese uses it to fold
    // full-width katakana onto hiragana and decompose precomposed voiced kana
    // into base + combining dakuten (the form the ja rules match); without it
    // katakana and every voiced kana produce no phonemes.  Other languages use
    // it for their own normalisation.  No-op (None, no alloc) without a
    // `.replace` section; runs here so it also covers TEXTMODE re-expansion.
    let normalized = dict.apply_replacements(word);
    let word = normalized.as_deref().unwrap_or(word);

    // Korean: break each Hangul syllable into its jamo (the ko rules match the
    // decomposed leading/vowel/trailing jamo, not the precomposed syllable).
    let hangul = decompose_hangul(word);
    let word = hangul.as_deref().unwrap_or(word);

    let ctx = LookupCtx {
        // Not a symbol-name lookup: C only sets `lookup_symbol` when resolving a
        // punctuation/symbol name, and it is what disables the `$atend` gate.
        lookup_symbol: false,
        at_clause_end: expect.at_clause_end,
        at_end_after: expect.at_end_after,
        clause_terminator: if expect.in_sentence {
            crate::dictionary::lookup::CLAUSE_TYPE_SENTENCE
        } else {
            0
        },
        is_first_word: expect.at_clause_start,
        word_flags: (if expect.first_upper {
            crate::dictionary::FLAG_FIRST_UPPER
        } else {
            0
        }) | (if expect.all_upper {
            crate::dictionary::FLAG_ALL_UPPER
        } else {
            0
        }) | (if expect.has_dot {
            crate::dictionary::FLAG_HAS_DOT
        } else {
            0
        }),
        expect_verb: expect.verb,
        expect_verb_s: expect.verb_s,
        expect_noun: expect.noun,
        expect_past: expect.past,
        dict_condition: lang_opts.dict_condition,
        word2: expect.following,
        ..Default::default()
    };

    // Try dictionary first
    let dict_result = lookup(dict, word, &ctx);
    WORD_FLAGS2.with(|f| f.set(dict_result.as_ref().map_or(0, |r| r.flags2.0)));

    // Extract flags from dict even if no phonemes (FLAGS-only entries).
    // Note: FLAGS-only entries have FLAG_FOUND_ATTRIBUTES (bit 30) but NOT FLAG_FOUND (bit 31).
    const FLAG_FOUND_ATTRIBUTES: u32 = 0x4000_0000;
    let dict_flags_from_lookup = dict_result
        .as_ref()
        .filter(|r| r.flags1.0 & (FLAG_FOUND_ATTRIBUTES | 0x8000_0000) != 0)
        .map(|r| r.flags1.0)
        .unwrap_or(0);

    if let Some(ref result) = dict_result {
        if result.flags1.found() && !result.phonemes.is_empty() {
            // TEXTMODE entry: the "phonemes" are really a replacement *text*
            // (emoji-name entries like `😀`→"grinning face", `$text` rules).
            // Re-translate it as words instead of reading the bytes as phonemes.
            // `textmode` is *reversed* for languages whose dictionary is written
            // as replacement text (cmn/yue/zh), so compare against the language's
            // default rather than testing the bit directly.
            let is_textmode = result.flags1.textmode() != lang_opts.reversed_textmode;
            if is_textmode && rule_word_flags & FLAG_IN_TEXTMODE == 0 {
                let replacement = String::from_utf8_lossy(&result.phonemes);
                let replacement = replacement.trim_end_matches('\0');
                // C expands a replacement word that has a replacement of its
                // own, three levels deep: Mandarin `☃` → `雪与雪人` → `雪` →
                // `xue3`.
                let depth = TEXTMODE_DEPTH.with(|d| d.get());
                let nested = if depth + 1 < 3 { 0 } else { FLAG_IN_TEXTMODE };
                TEXTMODE_DEPTH.with(|d| d.set(depth + 1));
                struct Restore(u8);
                impl Drop for Restore {
                    fn drop(&mut self) {
                        TEXTMODE_DEPTH.with(|d| d.set(self.0));
                    }
                }
                let _restore = Restore(depth);
                // Run the replacement through the *tokenizer*, not just
                // `split_whitespace`: a script written without spaces (Thai,
                // Chinese) needs the same word segmentation as ordinary input,
                // or only its first word is spoken.
                let mut phonemes: Vec<u8> = Vec::new();
                // Set when a part is dropped because its translation is a
                // language-switch marker: the entry *was* understood, so the
                // word must come out silent rather than fall through to reading
                // the replacement text as phoneme codes.
                let mut dropped_switch = false;
                // Split as C's `SegmentReplacement` does, then read each piece
                // as one word.
                let parts = segment_replacement(replacement, &lang_opts.lang);
                let mut embedded_table: Option<String> = None;
                for (part_ix, part) in parts.iter().enumerate() {
                    if part.chars().count() == 1 && part.chars().all(is_bracket) {
                        BRACKET_IN_REPLACEMENT.with(|b| b.set(true));
                        continue;
                    }
                    // Re-translate each replacement word the way the tokenizer
                    // hands an input word over: lower-cased for the dictionary
                    // and the letter rules, with the capitalisation carried in
                    // the flags instead.  German names every symbol with a
                    // capitalised noun (`∑` → "Summenzeichen", `😀` →
                    // "grinsendes Gesicht"); without this the rules match
                    // nothing, the part is dropped, and a single-word reading
                    // falls through to being read as raw phoneme *codes*
                    // (`-v de ∑` → `B'AE,EQ@,aIQqJ^Q"QE2`).
                    let lower = part.to_lowercase();
                    let letters: Vec<char> = part.chars().filter(|c| c.is_alphabetic()).collect();
                    // Clause position is deliberately *not* passed down: an
                    // `$atend` entry is chosen by the word's position in the
                    // clause buffer, and a replacement word never has one, so C
                    // reads `≤` as "less than or equal **tU**", the running-text
                    // form, even at the end of a clause.  The clause-end flags
                    // that do apply (`$u+`, `$strend`, `$unstressend`) are
                    // handled below instead.
                    let part_expect = PosExpect {
                        first_upper: part.chars().next().is_some_and(char::is_uppercase),
                        all_upper: !letters.is_empty() && letters.iter().all(|c| c.is_uppercase()),
                        ..PosExpect::default()
                    };
                    let wr = word_to_phonemes_inner(
                        &lower,
                        dict,
                        phdata,
                        stress_opts,
                        lang_opts,
                        rule_word_flags | nested,
                        part_expect,
                    );
                    // A word the rules can't read is dropped: C would spell it,
                    // but not inside a replacement (Japanese `☃`, `雪だるまと雪`,
                    // says nothing).
                    if !wr.found_in_list
                        && wr.dict_flags & crate::dictionary::FLAG_TEXTMODE == 0
                        && wr.phonemes.first() != Some(&crate::phoneme::PHON_SWITCH)
                        && rules_spell(&lower, dict, lang_opts)
                    {
                        dropped_switch = true;
                        continue;
                    }
                    // The clause-end stress attributes *do* apply, and C
                    // applies them to every word of the replacement rather than
                    // just its last — `÷` reads "divided b'aI", `es ™` "símbolo
                    // D'e marca comercial", and Hungarian's `$unstressend`
                    // words go the other way (`→` → `j,ob: _!,oldAlR2A …`).
                    // They only apply when the symbol itself ends the clause:
                    // `a 👍 b` and `👍 the` both keep "up" reduced.
                    // A piece with no letter (a Thai tone mark) is read by the
                    // rules alone, as C's `TranslateWord3` reads it.
                    let wr = if wr.phonemes.iter().all(|&b| b == 0)
                        && !lower.chars().any(char::is_alphabetic)
                    {
                        match letter_by_rules(&lower, true, dict, phdata, stress_opts, lang_opts) {
                            Some(phonemes) => WordResult { phonemes, ..wr },
                            None => wr,
                        }
                    } else {
                        wr
                    };
                    let mut owned;
                    let mut ph = wr.phonemes.strip_suffix(&[0]).unwrap_or(&wr.phonemes);
                    if expect.at_clause_end
                        && wr.dict_flags
                            & (crate::dictionary::FLAG_STRESS_END
                                | crate::dictionary::FLAG_STRESS_END2
                                | FLAG_UNSTRESS_END)
                            != 0
                    {
                        owned = ph.to_vec();
                        let to = if wr.dict_flags & FLAG_UNSTRESS_END != 0 {
                            3
                        } else {
                            4
                        };
                        change_word_stress(&mut owned, phdata, to);
                        ph = &owned[..];
                    }
                    if ph.is_empty() {
                        continue;
                    }
                    // A replacement word whose translation is a language-switch
                    // marker (German `©` → "Copyright", which the de rules send
                    // to English) can't be switched from here — the switch is
                    // applied one level up, per clause token.  Drop the part
                    // rather than emit the marker as if it were a phoneme; C
                    // produces nothing for these too.
                    if ph.at(0) == crate::phoneme::PHON_SWITCH {
                        // When the *first* word switches, C retranslates the
                        // whole symbol in that language (Mandarin `∑`, "N元求和",
                        // is read as English's `∑`, and so says nothing).
                        if part_ix == 0 && phonemes.is_empty() && word.chars().count() == 1 {
                            SYMBOL_SWITCHED_BY_RULES.with(|b| b.set(true));
                            return WordResult {
                                skipwords: result.skipwords,
                                phonemes: ph.to_vec(),
                                dict_flags: crate::dictionary::FLAG_TEXTMODE,
                                found_in_list: false,
                            };
                        }
                        // A later word that switches leaves its switch in the
                        // string, and C reads the byte after it as a phoneme
                        // table: the next word's break (15, `fa`) or the end
                        // of the string (0, `base`).  The rest is read against
                        // that table, and a phoneme it lacks is dropped with
                        // "Invalid phoneme code" — Mandarin `∞` is
                        // `w'u35(fa)_| t_| fu33_| X_|`, Portuguese `©`
                        // `… dy(base) @@`.
                        if !phonemes.is_empty() {
                            let more = part_ix + 1 < parts.len();
                            let table = match ph.get(1) {
                                Some(&b) => usize::from(b),
                                None if more => usize::from(PHON_END_WORD),
                                None => 0,
                            };
                            if let Some(name) = phdata.tables.get(table).map(|t| t.name.clone()) {
                                phonemes.push(crate::phoneme::PHON_SWITCH);
                                phonemes.push(u8::try_from(table).unwrap_or(0));
                                embedded_table = Some(name.clone());
                                STALE_TABLE.with(|s| *s.borrow_mut() = Some(name));
                                continue;
                            }
                        }
                        dropped_switch = true;
                        continue;
                    }
                    // A word that is only a tone (a Thai tone mark split off by
                    // the segmenting) gives its tone to the vowel before it,
                    // as C's tone does: `w'o3p` + `2` is `w'o2p`.
                    let only_tone = ph.iter().all(|&b| is_tone_phoneme(b, phdata));
                    if only_tone && !phonemes.is_empty() {
                        phonemes.extend_from_slice(ph);
                        reorder_tone_phonemes(&mut phonemes, phdata);
                        continue;
                    }
                    if !phonemes.is_empty() {
                        phonemes.push(crate::phoneme::PHON_END_WORD);
                    }
                    // (After a switch left in the string, against that table.)
                    match &embedded_table {
                        Some(t) => phonemes.extend(drop_invalid_codes(ph, t)),
                        None => phonemes.extend_from_slice(ph),
                    }
                }
                reorder_tone_phonemes(&mut phonemes, phdata);
                if !phonemes.is_empty() || dropped_switch {
                    return WordResult {
                        skipwords: result.skipwords,
                        phonemes,
                        // Only the TEXTMODE bit: it tells the caller the word
                        // *was* in the list, so it isn't spelled out as an
                        // unpronounceable abbreviation (`fr km` → "kilomètre").
                        dict_flags: crate::dictionary::FLAG_TEXTMODE,
                        found_in_list: false,
                    };
                }
            }
            let dict_flags = result.flags1.0;
            // `-X`: C prints `Found: 'word' [phonemes]  flags` and runs no rules.
            if crate::dictionary::rules::trace_enabled() && rule_word_flags == 0 {
                crate::dictionary::rules::trace_found(
                    word,
                    &result.phonemes,
                    [result.flags1.0, result.flags2.0],
                );
            }
            // Language-switch entry (`_^_LL`): the phonemes are `phonSWITCH`
            // followed by the target language name.  Return them verbatim —
            // stress placement must not touch the embedded language bytes; the
            // caller re-translates the word with the target language.
            if result.phonemes.at(0) == crate::phoneme::PHON_SWITCH {
                return WordResult {
                    skipwords: 0,
                    phonemes: result.phonemes.clone(),
                    dict_flags,
                    found_in_list: true,
                };
            }
            let mut phonemes = result.phonemes.clone();
            // Apply stress placement
            set_word_stress(
                &mut phonemes,
                phdata,
                stress_opts,
                Some(dict_flags as u32),
                -1,
                0,
            );
            // (No stressed-vowel upgrading here: the pronunciation came from the
            // list, and C's `isTranslationGiven` gate keeps the vowel changes off
            // a word whose phonemes were given — `tr b` is `b'e`, not `b'E`.)
            if stress_opts.alt_attributes
                && dict_flags
                    & (crate::dictionary::FLAG_ALT_TRANS | crate::dictionary::FLAG_ALT2_TRANS)
                    != 0
            {
                let alt2 = dict_flags & crate::dictionary::FLAG_ALT2_TRANS != 0;
                apply_special_attribute2(&mut phonemes, phdata, alt2);
            }
            // Word-final devoicing (e.g. German Auslautverhärtung)
            if stress_opts.word_final_devoicing {
                apply_word_final_devoicing(&mut phonemes, phdata);
            }
            return WordResult {
                skipwords: result.skipwords,
                phonemes,
                dict_flags,
                found_in_list: true,
            };
        }
    }

    // `$accent`: speak the letter as its base letter and accent names.
    if rule_word_flags == 0 {
        let accent = |r: &crate::dictionary::lookup::LookupResult| {
            r.flags2.contains(crate::dictionary::FLAG_ACCENT)
        };
        if dict_result.as_ref().is_some_and(accent) {
            if let Some(mut phonemes) = accented_letter_name(word, dict, lang_opts) {
                set_word_stress(&mut phonemes, phdata, stress_opts, Some(0), -1, 0);
                return WordResult {
                    skipwords: 0,
                    phonemes,
                    dict_flags: 0,
                    found_in_list: true,
                };
            }
        }
    }

    if rule_word_flags == 0 {
        if let Some(token) = NumberToken::parse(word, &lang_opts.number_grammar) {
            if let Some(result) =
                translate_number_token(&token, dict, phdata, stress_opts, &lang_opts.number_grammar)
            {
                return result;
            }
        }
    }

    // `-X`: announce the word before its rule matches, as C's `TranslateWord`
    // does.  Only top-level words (a recursive expansion keeps the same header).
    if crate::dictionary::rules::trace_enabled() && rule_word_flags == 0 {
        crate::dictionary::rules::trace_word(word);
    }

    // Fall back to translation rules (potentially using dict flags from FLAGS-only entry)
    // Use the dictionary's language-specific letter_bits (Cyrillic, Arabic, etc.)
    //
    // `FLAG_IN_TEXTMODE` is *our* marker, not one of C's word flags — it must not
    // reach the rules engine, where bit 30 is `FLAG_FOUND_ATTRIBUTES` and changes
    // how the word is matched.  (Symptom: a Thai emoji whose replacement text is
    // a spaceless run of words stopped translating after the first two
    // syllables.)
    let rule_word_flags = rule_word_flags & !FLAG_IN_TEXTMODE
        | if expect.hyphen_before {
            crate::dictionary::FLAG_HYPHEN
        } else {
            0
        }
        | if expect.hyphen_after {
            crate::dictionary::FLAG_HYPHEN_AFTER
        } else {
            0
        };
    let letter_bits = &*dict.letter_bits;
    let mut vowel_count = 0i32;
    let mut stressed_count = 0i32;

    // Prepare word buffer with leading space (for rule pre-context), and with
    // the start of the *next* word after it so a post-context can reach across
    // the boundary (`_) o (_C`).  The translation loop itself stops at the
    // space, so the extra text is only ever read by a context.
    let mut word_buf = Vec::with_capacity(word.len() + 20);
    word_buf.push(b' ');
    // Letters of the same word that come *before* this fragment, so a rule's
    // pre-context can see them.  They are not translated: `word_start` points
    // past them, and because the test for `_`/start-of-word reads the buffer
    // byte itself, the fragment correctly stops counting as a word start.
    let before = expect.prev_word.split(|&b| b == 0).next().unwrap_or(&[]);
    if !before.is_empty() {
        word_buf.extend_from_slice(before);
        word_buf.push(b' ');
    }
    let prev = expect.prev_letters.split(|&b| b == 0).next().unwrap_or(&[]);
    word_buf.extend_from_slice(prev);
    let word_start = word_buf.len();
    word_buf.extend_from_slice(word.as_bytes());
    word_buf.push(b' ');
    let next = expect.next_word.split(|&b| b == 0).next().unwrap_or(&[]);
    if !next.is_empty() {
        word_buf.extend_from_slice(next);
        word_buf.push(b' ');
    }
    word_buf.push(0);

    // C passes the flags from the *list* lookup into `TranslateRules`
    // (`dict_flags[0]`), which is what lets a `$w_alt<n>` rule condition see a
    // flags-only entry: `en_list`'s `banana $alt3` is what selects the `a#`
    // rule, giving upstream's `ba#n'A:n@`.
    let result = translate_rules_phdata(
        dict,
        &word_buf,
        word_start, // past the leading space and any in-word pre-context
        rule_word_flags,
        dict_flags_from_lookup | INHERITED_ALT.with(std::cell::Cell::get),
        &letter_bits,
        lang_opts.dict_condition,
        &mut vowel_count,
        &mut stressed_count,
        Some(phdata),
    );

    // C's second `TranslateRules` call, with `end_phonemes == NULL`: translate the
    // whole word straight through, ignoring prefix/suffix endings.  Needed for
    // every path where the ending can't actually be removed, because the first
    // call stops as soon as an ending matches and would otherwise leave a
    // half-translated word.
    let translate_whole_word = || {
        let (mut vc, mut sc) = (0i32, 0i32);
        let r = crate::dictionary::rules::translate_rules_ext(
            dict,
            &word_buf,
            word_start,
            rule_word_flags,
            dict_flags_from_lookup,
            &letter_bits,
            lang_opts.dict_condition,
            &mut vc,
            &mut sc,
            Some(phdata),
            false,
        );
        combine_rules_result(&r)
    };
    // Only a *prefix* match cuts the rules scan short, so only then does the
    // partial result need re-translating; after a suffix match `result` still
    // holds the whole word, as it always did.
    let fallback_phonemes = |result: &crate::dictionary::rules::RulesResult| {
        if result.end_type & SUFX_P != 0 {
            translate_whole_word()
        } else {
            combine_rules_result(result)
        }
    };

    fn rules_produced_output(r: &crate::dictionary::rules::RulesResult) -> bool {
        if r.spellword || r.end_type != 0 {
            return true;
        }
        r.phonemes.iter().any(|&b| b != 0) || r.end_phonemes.iter().any(|&b| b != 0)
    }

    if rules_produced_output(&result) {
        let mut stress_dict_flags = dict_flags_from_lookup;
        // A `SUFX_T` ending is added after the stem's stress is placed (Spanish
        // `-mente`).
        let mut late_suffix: Vec<u8> = Vec::new();
        // An unstressed prefix is joined to a stem stressed on its own, and
        // the whole isn't stressed again (C's last branch): Dutch "ongeveer"
        // has no pause before its first vowel.
        let mut stressed_already = false;
        let mut phonemes = if result.end_type != 0 && (result.end_type & SUFX_P) != 0 {
            if let Some((stem, stem_word_flags)) = remove_standard_prefix(word, result.end_type) {
                let mut combined = Vec::new();
                // C's `prefix_stress`: the prefix carries a primary stress.
                let prefix_stress = result
                    .end_phonemes
                    .iter()
                    .any(|&b| b == PHON_STRESS_P || b == PHON_STRESS_P2);
                let mut prefix = result.end_phonemes.clone();
                if stress_opts.prefix_stress && (prefix_stress || stress_dict_flags & 0x7 != 0) {
                    // All but the prefix's first primary are reduced.
                    let mut seen = false;
                    for b in prefix.iter_mut().filter(|b| **b == PHON_STRESS_P) {
                        if seen {
                            *b = PHON_STRESS_3;
                        }
                        seen = true;
                    }
                }
                append_raw_phonemes(&mut combined, &prefix);
                let alt = stress_dict_flags & (0x7f << crate::dictionary::BITNUM_FLAG_ALT);
                let saved_alt = INHERITED_ALT.with(|a| a.replace(alt));
                let stem_wr = word_to_phonemes_inner(
                    &stem,
                    dict,
                    phdata,
                    stress_opts,
                    lang_opts,
                    stem_word_flags,
                    expect,
                );
                INHERITED_ALT.with(|a| a.set(saved_alt));
                // A `$1`…`$7` on the whole word places its stress, so C takes the
                // stem as the rules left it, not stressed on its own.
                let unstressed = || -> Vec<u8> {
                    stem_wr
                        .phonemes
                        .iter()
                        .copied()
                        .filter(|b| !(PHON_STRESS_U..=PHON_STRESS_P2).contains(b))
                        .collect()
                };
                let prefixed = stress_dict_flags & 0x7 != 0 || prefix_stress;
                if prefixed && !stress_opts.prefix_stress {
                    append_raw_phonemes(&mut combined, &unstressed());
                } else if prefixed {
                    // `LOPT_PREFIXES` (German, Dutch, Afrikaans, Danish): the stem
                    // keeps its stresses, but no more than secondary, before
                    // the whole word is stressed (Afrikaans "onomwonde $3" is
                    // `_|On_|,Omv'Ond,@`).
                    let mut stem = unstressed();
                    // (Its own break before a first vowel is placed again.)
                    if matches!(
                        stem.first(),
                        Some(
                            &(crate::phoneme::PHON_PAUSE_NOLINK
                                | crate::phoneme::PHON_PAUSE_VSHORT)
                        )
                    ) {
                        stem.remove(0);
                    }
                    if stem.last() != Some(&0) {
                        stem.push(0);
                    }
                    // (With the whole word's flags, as C passes them: its
                    // `$3` counts in the stem.)
                    let flags = if stress_dict_flags != 0 {
                        stress_dict_flags
                    } else {
                        stem_wr.dict_flags
                    };
                    set_word_stress(&mut stem, phdata, stress_opts, Some(flags), 3, 0);
                    append_raw_phonemes(&mut combined, &stem);
                } else {
                    append_raw_phonemes(&mut combined, &stem_wr.phonemes);
                    // (A word with flags of its own is still stressed as a
                    // whole: Dutch `onze $u`.)
                    stressed_already = stress_dict_flags == 0;
                }
                // The whole word's own flags win over its stem's (`output $1`).
                if stress_dict_flags == 0 {
                    stress_dict_flags = stem_wr.dict_flags;
                }
                combined.push(0);
                combined
            } else {
                let mut fallback = fallback_phonemes(&result);
                fallback.push(0);
                fallback
            }
        } else if result.end_type != 0 && result.suffix_start > 1 {
            // A word can stack suffixes when the outer one is marked `SUFX_M`
            // ("allow more suffixes before this suffix") — English `'s` is, so
            // `s's's` is stem `s` + `'s` + `'s`.  Peel them innermost-last,
            // accumulating each suffix's phonemes in front of the ones already
            // collected (C's `strcpy(end_phonemes2, end_phonemes); … strcat`).
            // Bounded like C's prefix loop; upstream left this loop uncapped,
            // which is how a 440-byte `s's's…` smashed its stack (#2495).
            let peeled =
                peel_stacked_suffixes(word, &result, dict, &letter_bits, lang_opts, phdata);
            let word = peeled.word.as_str();
            let result_end_type = peeled.end_type;
            let result_end_phonemes = peeled.end_phonemes;
            let peeled_stem_phonemes = peeled.stem_phonemes;
            if let Some((stem, end_flags, stem_word_flags)) =
                remove_standard_suffix(word, result_end_type, dict)
            {
                let stem_lookup = lookup(
                    dict,
                    &stem,
                    &LookupCtx {
                        // C compares the stem's end with the clause's last
                        // *byte*, so `$atend` still applies after a one-byte
                        // suffix (Romanian `îi` is the letter `î`) but not a
                        // longer one (Arabic `اه`); and Portuguese "os amigos"
                        // isn't at the end at all.
                        lookup_symbol: false,
                        at_clause_end: expect.at_clause_end && {
                            let n = usize::try_from(result_end_type & 0x3f).unwrap_or(0);
                            word.chars()
                                .rev()
                                .take(n)
                                .map(char::len_utf8)
                                .sum::<usize>()
                                == 1
                        },
                        clause_terminator: if expect.in_sentence {
                            crate::dictionary::lookup::CLAUSE_TYPE_SENTENCE
                        } else {
                            0
                        },
                        is_first_word: expect.at_clause_start,
                        end_flags,
                        expect_verb: expect.verb,
                        expect_verb_s: expect.verb_s,
                        expect_noun: expect.noun,
                        expect_past: expect.past,
                        dict_condition: lang_opts.dict_condition,
                        ..Default::default()
                    },
                );

                let mut combined = Vec::new();
                let mut used_stem = false;

                // `SUFX_Q` on the ending means "don't retranslate": when the
                // stem isn't in the dictionary, keep what the *whole word*
                // produced instead of re-running the rules on the stem.  Italian
                // "casa" matches `@C) a (_S1q`, and the retranslation loses the
                // intervocalic `A) s (A → z` (`k'asa` for `k'aza`).
                let no_retranslate = result_end_type & crate::dictionary::SUFX_Q != 0;

                if let Some(stem_lookup) = stem_lookup {
                    // A stem that belongs to another language takes the whole
                    // word with it: German "windows" is English.
                    if stem_lookup.phonemes.first() == Some(&crate::phoneme::PHON_SWITCH) {
                        return WordResult {
                            phonemes: stem_lookup.phonemes.clone(),
                            dict_flags: stem_lookup.flags1.0,
                            found_in_list: true,
                            skipwords: 0,
                        };
                    }
                    if !stem_lookup.phonemes.is_empty() {
                        combined.extend_from_slice(&stem_lookup.phonemes);
                        // The word's own flags win; the stem's apply only when
                        // it has none (English `executable $3`).
                        if stress_dict_flags == 0 {
                            stress_dict_flags = stem_lookup.flags1.0;
                        }
                        used_stem = true;
                    } else {
                        // A *flags-only* entry for the stem still applies, as it
                        // does in C ("Flags:  somm   $alt2" in its trace): the
                        // flags decide the word's stress and its alternate vowel
                        // quality even though the pronunciation comes from the
                        // rules.  Italian `somma` reads `s'omma` because of it.
                        if stress_dict_flags == 0 {
                            stress_dict_flags = stem_lookup.flags1.0;
                        }
                    }
                }

                // (With `SUFX_Q` the stem is left untranslated, so `used_stem`
                // stays false and the fallback below — the whole word's own
                // phonemes — is what gets used.)
                if !used_stem && !no_retranslate {
                    let mut stem_buf = Vec::with_capacity(stem.len() + 3);
                    stem_buf.push(b' ');
                    stem_buf.extend_from_slice(stem.as_bytes());
                    stem_buf.push(b' ');
                    stem_buf.push(0);

                    let mut stem_vc = 0i32;
                    let mut stem_sc = 0i32;
                    // (C passes the word's dictionary flags on: a `$alt` word
                    // keeps its alternative rules in the stem, Basque "alajain".)
                    let stem_rules = translate_rules_phdata(
                        dict,
                        &stem_buf,
                        1,
                        stem_word_flags,
                        stress_dict_flags,
                        &letter_bits,
                        lang_opts.dict_condition,
                        &mut stem_vc,
                        &mut stem_sc,
                        Some(phdata),
                    );
                    // A stem can itself start with a standard prefix (German
                    // `be-`, `ge-`); the rules stop at it, so hand that case to
                    // the full word translator instead of keeping the fragment.
                    let stem_phonemes = if stem_rules.end_type & SUFX_P != 0 {
                        word_to_phonemes_inner(
                            &stem,
                            dict,
                            phdata,
                            stress_opts,
                            lang_opts,
                            stem_word_flags,
                            expect,
                        )
                        .phonemes
                    } else {
                        combine_rules_result(&stem_rules)
                    };
                    if stem_phonemes.iter().any(|&b| b != 0) {
                        append_raw_phonemes(&mut combined, &stem_phonemes);
                        used_stem = true;
                    }
                }

                if used_stem {
                    if result_end_type & crate::dictionary::SUFX_T != 0 {
                        append_raw_phonemes(&mut late_suffix, &result_end_phonemes);
                    } else {
                        append_raw_phonemes(&mut combined, &result_end_phonemes);
                    }
                    combined.push(0);
                    combined
                } else if let Some(stem_ph) =
                    peeled_stem_phonemes.filter(|ph| no_retranslate && ph.iter().any(|&b| b != 0))
                {
                    // `SUFX_Q` reached through a stacked suffix: the inner
                    // ending forbids retranslating its stem, so keep what the
                    // chain's own re-translation made of it and add every
                    // suffix on top.  Falling back to the *whole word's*
                    // phonemes here dropped the inner ending — Dutch
                    // `vorige` (= `vorig` + `-e`, where `ig` is a `S2dq`
                    // ending) read `v'o:r@` instead of `v'o:r@Q@`.
                    let mut combined = stem_ph;
                    append_raw_phonemes(&mut combined, &result_end_phonemes);
                    combined.push(0);
                    combined
                } else {
                    let mut fallback = fallback_phonemes(&result);
                    fallback.push(0);
                    fallback
                }
            } else {
                let mut fallback = fallback_phonemes(&result);
                fallback.push(0);
                fallback
            }
        } else if result.end_type != 0 {
            // An ending matched but neither branch could use it (e.g. a prefix
            // whose letters can't be removed): translate the word in full rather
            // than keep the partial result the scan stopped at.
            let mut combined = fallback_phonemes(&result);
            combined.push(0);
            combined
        } else {
            let mut combined = combine_rules_result(&result);
            combined.push(0);
            combined
        };

        // Apply stress placement; use dict flags from FLAGS-only entry if available
        let flags_for_stress = if stress_dict_flags != 0 {
            Some(stress_dict_flags as u32)
        } else {
            Some(0) // non-NULL mirrors C's behavior of passing non-NULL dictionary_flags
        };
        let control = if late_suffix.is_empty() { 0 } else { 2 };
        if !stressed_already {
            set_word_stress(
                &mut phonemes,
                phdata,
                stress_opts,
                flags_for_stress,
                -1,
                control,
            );
        }
        if !late_suffix.is_empty() {
            phonemes.truncate(trim_lookup(&phonemes).len());
            phonemes.extend_from_slice(&late_suffix);
            phonemes.push(0);
        }
        // Stressed-vowel upgrading (e.g. Turkish e→E, o→O under primary stress)
        if stress_opts.alt_stress_upgrade {
            apply_alt_stress_upgrade(&mut phonemes, phdata);
        }
        if stress_opts.alt_attributes {
            let flags = stress_dict_flags;
            if flags & (crate::dictionary::FLAG_ALT_TRANS | crate::dictionary::FLAG_ALT2_TRANS) != 0
            {
                let alt2 = flags & crate::dictionary::FLAG_ALT2_TRANS != 0;
                apply_special_attribute2(&mut phonemes, phdata, alt2);
            }
        }
        // Word-final devoicing (e.g. German Auslautverhärtung)
        if stress_opts.word_final_devoicing {
            apply_word_final_devoicing(&mut phonemes, phdata);
        }
        // Tone phonemes belong to their syllable's vowel, not to the end of the
        // syllable where the tone digit was written (no-op for atonal languages).
        // (Inside a replacement the whole of it is reordered at once, as C
        // places tones over the word it builds: a Thai `w2` gives its tone to
        // the vowel of the word before.)
        if TEXTMODE_DEPTH.with(std::cell::Cell::get) == 0 {
            reorder_tone_phonemes(&mut phonemes, phdata);
        }
        // Pronounced by the rules: only the *flags* came from the list, so
        // this is not `FLAG_FOUND` and the stress-conditioned changes still apply.
        return WordResult {
            skipwords: 0,
            phonemes,
            dict_flags: stress_dict_flags,
            found_in_list: false,
        };
    }

    // Could not translate (unknown word)
    WordResult {
        skipwords: 0,
        phonemes: Vec::new(),
        dict_flags: dict_flags_from_lookup,
        found_in_list: false,
    }
}

// ---------------------------------------------------------------------------
// Translator
// ---------------------------------------------------------------------------

/// One sentence / clause unit for dictionary + stress promotion (`text_to_ipa` and
/// `translate_to_codes` share this pipeline — GitHub #4 and related parity).
#[derive(Clone, PartialEq)]
pub(crate) enum TranslateEntryKind {
    Word,
    ClauseBoundary,
    Other,
    /// A `phonSWITCH` language-switch marker (`_^_LL`): `phonemes` is
    /// `[phonSWITCH, <target-language bytes…>]`.  Excluded from word-level
    /// stress promotion; resolved at render time.
    LangSwitch,
    /// A character nothing else reads, said as C's `TranslateLetter` says it:
    /// parts in this language and in others, resolved at render time.
    /// `phonemes` holds the parts in this language, for the passes that only
    /// ask whether the entry speaks.
    Letter(Vec<LetterPart>),
}

/// A piece of a [`TranslateEntryKind::Letter`].
#[derive(Clone, PartialEq)]
pub(crate) enum LetterPart {
    /// Phonemes of this language.
    Local(Vec<u8>),
    /// A dictionary key in another language (an alphabet's name, `_zh`),
    /// after a pause; nothing if that language has no entry.
    Key { lang: String, key: String },
    /// The letter's name in another language, or `otherwise` here if it has
    /// none there.
    Letter {
        lang: String,
        letter: char,
        otherwise: Vec<u8>,
    },
    /// The letter's phonemes in another language, written in this one's table:
    /// C switches back before them (French `ა` is `_:(en)dZ'O@dZ@n(fr)'a`).
    Raw { lang: String, letter: char },
    /// A word boundary: the letters of a spelled word are words of their own.
    Break,
}

pub(crate) struct TranslateEntry {
    pub(crate) phonemes: Vec<u8>,
    pub(crate) dict_flags: u32,
    /// The word's phonemes came from the dictionary *list* (C's `FLAG_FOUND`),
    /// which suppresses the stress-conditioned phoneme changes.
    pub(crate) found_in_list: bool,
    pub(crate) kind: TranslateEntryKind,
    pub(crate) word_lower: Option<String>,
    /// Suppress the language's inter-word gap after this word.
    ///
    /// Mandarin's third-tone sandhi *joins* the pair it applies to: 你好 reads
    /// as `ni35X'Au214_|` — one gap, at the end — not as two separate words.
    pub(crate) no_word_gap: bool,
}

/// Parse the content of a `[[ … ]]` inline-phoneme span into phoneme codes.
///
/// Mirrors espeak's `EncodePhonemes` / `LookupPhonemeString`: the mnemonic
/// string is scanned left-to-right, greedily matching the **longest** phoneme
/// mnemonic (1–4 characters) present in the currently-selected phoneme table.
/// Whitespace separates mnemonics; unrecognised characters are skipped.
///
/// Stress marks such as `'` and `,` are ordinary phonemes in the table, so
/// they are matched here and rendered as stress by the IPA stage.
pub(crate) fn parse_inline_phonemes(content: &str, phdata: &PhonemeData) -> Vec<u8> {
    let bytes = content.as_bytes();
    let mut codes = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes.at(i).is_ascii_whitespace() {
            // A space inside `[[…]]` is a word boundary, exactly as in normal
            // text: without the separator `[[h@l'oU w'3:ld]]` ran together as
            // one word (`h@l'oUw'3:ld`) in `-x` and in the audio.
            if !codes.is_empty() && codes.last() != Some(&crate::phoneme::PHON_END_WORD) {
                codes.push(crate::phoneme::PHON_END_WORD);
            }
            i += 1;
            continue;
        }
        let maxlen = 4.min(bytes.len() - i);
        let mut matched = false;
        for len in (1..=maxlen).rev() {
            let Some(slice) = content.get(i..i + len) else {
                continue;
            };
            let code = phdata.lookup_phoneme(slice);
            if code != 0 {
                codes.push(code);
                i += len;
                matched = true;
                if code == crate::phoneme::PHON_SWITCH {
                    // `phonSWITCH` is followed by a language name, which C
                    // lowercases and drops entirely when it is the default voice
                    // (`_^_EN` in de_list is just "switch to the default").
                    let start = i;
                    while i < bytes.len() && !bytes.at(i).is_ascii_whitespace() {
                        codes.push(bytes.at(i).to_ascii_lowercase());
                        i += 1;
                    }
                    let name_at = codes.len() - (i - start);
                    if codes.get(name_at..).unwrap_or_default() == b"en" {
                        codes.truncate(name_at);
                    }
                }
                break;
            }
        }
        if !matched {
            i += 1; // skip an unrecognised character
        }
    }
    codes
}

/// Apply a segment's SSML [`SayAs`](ssml::SayAs) interpretation to its text,
/// given the language it will be translated in.  `Ordinal` appends the English
/// ordinal suffix to integer tokens ("3" → "3rd"), which the number path then
/// reads as an ordinal; other languages/modes pass through unchanged.
/// Process SSML into the plain text the synthesizer should speak, for the audio
/// path: strips tags, decodes entities, applies text-level `<say-as>` spelling
/// (`characters`/`digits`/`telephone`) **and** the language-aware interpret modes
/// (`ordinal`/`date`/`time`) — the latter were previously only applied on the
/// `--ipa`/`-x` path.  `<voice>` language spans are read in `lang` (audio voice
/// switching is not yet wired).
pub fn ssml_to_speech_text(text: &str, lang: &str) -> String {
    ssml_to_speech_text_opts(text, lang, 100)
}

/// [`ssml_to_speech_text`] with the `--ssml-break` percentage
/// (`espeakSSML_BREAK_MUL`) applied to every `<break time=…>`.
pub fn ssml_to_speech_text_opts(text: &str, lang: &str, ssml_break: u32) -> String {
    ssml::process_markup_opts(text, ssml_break)
        .0
        .into_iter()
        .map(|s| interpret_segment_text(&s.text, s.interpret, lang))
        .collect()
}

fn interpret_segment_text(text: &str, interpret: ssml::SayAs, lang: &str) -> String {
    // These renderings are English-only (best-effort); other languages pass
    // through and read the raw content.
    if interpret == ssml::SayAs::Normal || primary_bcp47_subtag(lang) != "en" {
        return text.to_string();
    }
    match interpret {
        ssml::SayAs::Ordinal => ordinalize_english(text),
        ssml::SayAs::Date => dateize_english(text),
        ssml::SayAs::Time => timeize_english(text),
        ssml::SayAs::Normal => unreachable!(),
    }
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Render an ISO `YYYY-MM-DD` (or `YYYY/MM/DD`) date as English words:
/// "2024-01-15" → "January 15th 2024" (read as "January fifteenth …").
/// Unrecognised input is returned unchanged (read normally).
fn dateize_english(text: &str) -> String {
    let parts: Vec<&str> = text.trim().split(['-', '/']).collect();
    if parts.len() == 3 && parts.at(0).len() == 4 {
        if let (Ok(y), Ok(m), Ok(d)) = (
            parts.at(0).parse::<u32>(),
            parts.at(1).parse::<u32>(),
            parts.at(2).parse::<u32>(),
        ) {
            if (1..=12).contains(&m) && (1..=31).contains(&d) {
                let ds = d.to_string();
                return format!(
                    "{} {d}{} {y}",
                    MONTHS.at((m - 1) as usize),
                    english_ordinal_suffix(&ds)
                );
            }
        }
    }
    text.to_string()
}

/// Render a `HH:MM` (or `HH:MM:SS`) time as English words: "14:30" → "14 30"
/// ("fourteen thirty"), "9:05" → "9 oh 5", "14:00" → "14 o'clock".
/// Unrecognised input is returned unchanged.
fn timeize_english(text: &str) -> String {
    let t = text.trim();
    if let Some((h, rest)) = t.split_once(':') {
        let m = rest.split(':').next().unwrap_or("");
        if let (Ok(hh), Ok(mm)) = (h.parse::<u32>(), m.parse::<u32>()) {
            if hh < 24 && mm < 60 {
                return match mm {
                    0 => format!("{hh} o'clock"),
                    1..=9 => format!("{hh} oh {mm}"),
                    _ => format!("{hh} {mm}"),
                };
            }
        }
    }
    text.to_string()
}

/// Append the English ordinal suffix to every integer token in `text`.
fn ordinalize_english(text: &str) -> String {
    text.split_whitespace()
        .map(|tok| {
            if !tok.is_empty() && tok.bytes().all(|b| b.is_ascii_digit()) {
                format!("{tok}{}", english_ordinal_suffix(tok))
            } else {
                tok.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The English ordinal suffix for a run of decimal digits (`st`/`nd`/`rd`/`th`).
fn english_ordinal_suffix(digits: &str) -> &'static str {
    let last_two = &digits[digits.len().saturating_sub(2)..];
    if matches!(last_two, "11" | "12" | "13") {
        return "th";
    }
    match digits.as_bytes().last() {
        Some(b'1') => "st",
        Some(b'2') => "nd",
        Some(b'3') => "rd",
        _ => "th",
    }
}

/// Technical symbols that eSpeak NG speaks as their name by default (i.e.
/// without `--punct`).  Everything else — `.,;:!?'"()[]{}` etc. — is treated
/// as pause/clause punctuation and produces no spoken output.
///
/// Determined empirically against the C reference (`x <sym> y`).
/// A word token that is exactly one alphabetic character — an acronym letter
/// ("U", "e").  A leading Lojban stress mark (`ˈ`) is ignored so caps-mark-stress
/// languages still qualify.
fn is_single_letter_word(w: &str) -> bool {
    let mut chars = w.chars().filter(|&c| c != '\u{02c8}');
    matches!(chars.next(), Some(c) if c.is_alphabetic()) && chars.next().is_none()
}

fn is_spoken_symbol(c: char) -> bool {
    matches!(
        c,
        '.' | '#'
            | '¼'
            | '½'
            | '¾'
            | '⅓'
            | '⅔'
            | '$'
            | '%'
            | '&'
            | '*'
            | '+'
            | '/'
            | '='
            | '@'
            | '~'
            | '×'
            | '÷'
            | '°'
            | '±'
            | '−'
            | '§'
            | '№'
            | '∞'
            | '√'
            | '∑'
            | '≈'
            | '≠'
            | '≤'
            | '≥'
            | '℃'
            | '℉'
            | '™'
            | '®'
            | '©'
            | '‰'
            | '‱'
    )
}

/// Whether the characters after a leading `-` begin a number, so the hyphen is a
/// unary minus (`-5`).  Takes a clone of the tokenizer iterator positioned just
/// past the `-`.
fn minus_precedes_number<I>(chars: &std::iter::Peekable<I>) -> bool
where
    I: Iterator<Item = char> + Clone,
{
    // (Not before a currency sign: C reads "-$5" as "dollar five".)
    chars.clone().next().is_some_and(|d| d.is_ascii_digit())
}

/// Merge space-grouped thousands into a single number (`1 234 567` →
/// `1234567`) for languages that use the SI/ISO space separator
/// (`grammar.space_group`).  A run is a 1–3 digit leading group followed by one
/// or more ` <3 digits>` groups; anything that isn't exactly three digits ends
/// the run, so ordinary adjacent numbers are left alone.
fn apply_space_grouping(tokens: &mut Vec<Token>, grammar: &NumberGrammar) {
    if !grammar.space_group {
        return;
    }
    let is_digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if let Some(Token::Number(NumberToken::Cardinal(first))) = tokens.get(i) {
            if first.len() <= 3 && is_digits(first) {
                // Greedily consume `<space> <exactly-3-digit cardinal>` groups.
                let mut merged = first.clone();
                let mut j = i + 1;
                let mut groups = 0;
                while matches!(tokens.get(j), Some(Token::Space | Token::WordJoin)) {
                    if let Some(Token::Number(NumberToken::Cardinal(g))) = tokens.get(j + 1) {
                        if g.len() == 3 && is_digits(g) {
                            merged.push_str(g);
                            j += 2;
                            groups += 1;
                            continue;
                        }
                    }
                    break;
                }
                // The final group may be a decimal (`1 234,56`): merge its
                // 3-digit integer part and keep the fraction on the whole number.
                let mut fraction: Option<String> = None;
                if matches!(tokens.get(j), Some(Token::Space | Token::WordJoin)) {
                    if let Some(Token::Number(NumberToken::Decimal {
                        integer,
                        fractional,
                    })) = tokens.get(j + 1)
                    {
                        if integer.len() == 3 && is_digits(integer) {
                            merged.push_str(integer);
                            fraction = Some(fractional.clone());
                            j += 2;
                            groups += 1;
                        }
                    }
                }
                if groups >= 1 {
                    let token = match fraction {
                        Some(fractional) => NumberToken::Decimal {
                            integer: merged,
                            fractional,
                        },
                        None => NumberToken::Cardinal(merged),
                    };
                    out.push(Token::Number(token));
                    i = j;
                    continue;
                }
            }
        }
        out.extend(tokens.get(i).cloned());
        i += 1;
    }
    *tokens = out;
}

/// A writing system the port can route to a dedicated voice.
///
/// Mirrors the `alphabets[]` table in upstream `tr_languages.c`: a run of text
/// in a script the current voice does not read is handed to a voice that does
/// (upstream's `translator2` / "split a word where the script changes to a
/// foreign one"), instead of being dropped.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Script {
    Latin,
    Cyrillic,
    Greek,
    Arabic,
    Hebrew,
    Armenian,
    Georgian,
    Devanagari,
    Bengali,
    Gurmukhi,
    Gujarati,
    Oriya,
    Tamil,
    Telugu,
    Kannada,
    Malayalam,
    Sinhala,
    Thai,
    Myanmar,
    Ethiopic,
    Cherokee,
    Hangul,
    Kana,
    Han,
}

/// The script of a single character, or `None` for digits, punctuation, emoji
/// and scripts no shipped voice reads.
fn char_script(c: char) -> Option<Script> {
    let cp = c as u32;
    Some(match cp {
        0x0041..=0x005A | 0x0061..=0x007A => Script::Latin,
        // Latin-1 supplement letters, minus the two maths signs (× ÷) that sit
        // inside the letter ranges.
        0x00C0..=0x00FF if cp != 0x00D7 && cp != 0x00F7 => Script::Latin,
        0x0100..=0x024F | 0x1E00..=0x1EFF => Script::Latin, // Latin Extended-A/B, Additional
        0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
        0x0400..=0x052F => Script::Cyrillic,
        0x0530..=0x058F => Script::Armenian,
        0x0590..=0x05FF => Script::Hebrew,
        0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Script::Arabic,
        0x0900..=0x097F => Script::Devanagari,
        0x0980..=0x09FF => Script::Bengali,
        0x0A00..=0x0A7F => Script::Gurmukhi,
        0x0A80..=0x0AFF => Script::Gujarati,
        0x0B00..=0x0B7F => Script::Oriya,
        0x0B80..=0x0BFF => Script::Tamil,
        0x0C00..=0x0C7F => Script::Telugu,
        0x0C80..=0x0CFF => Script::Kannada,
        0x0D00..=0x0D7F => Script::Malayalam,
        0x0D80..=0x0DFF => Script::Sinhala,
        0x0E00..=0x0E7F => Script::Thai,
        0x1000..=0x109F => Script::Myanmar,
        0x10A0..=0x10FF | 0x2D00..=0x2D2F => Script::Georgian,
        0x1200..=0x137F => Script::Ethiopic,
        0x13A0..=0x13FF | 0xAB70..=0xABBF => Script::Cherokee,
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => Script::Hangul,
        0x3040..=0x30FF | 0x31F0..=0x31FF => Script::Kana,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => Script::Han,
        0x20000..=0x2FA1F => Script::Han, // CJK Extension B-H + compatibility supplement
        _ => return None,
    })
}

/// Does upstream's `alphabets[]` table set `AL_WORDS` for this script — "use the
/// language to speak words"?
///
/// Only these scripts switch translator for a whole word.  Everything else
/// (Cyrillic, Hebrew, Thai, Han, Kana, …) is **spelled** letter by letter in the
/// current language: `en "хорошо"` reads out the Cyrillic letter names rather
/// than switching to Russian.  Taken from `tr_languages.c`, minus Greek: its row
/// also carries `AL_DONT_NAME | AL_NOT_LETTERS`, and upstream is observed to
/// spell Greek in an English voice (`αβγ` → `'alf@_b'i:t@_g'am@_`) rather than
/// switch, so it belongs with the spelled scripts here.
fn script_uses_words(script: Script) -> bool {
    matches!(
        script,
        Script::Armenian
            | Script::Arabic
            | Script::Devanagari
            | Script::Bengali
            | Script::Gurmukhi
            | Script::Gujarati
            | Script::Tamil
            | Script::Kannada
            | Script::Malayalam
            | Script::Sinhala
            | Script::Georgian
            | Script::Hangul
    )
}

/// The default voice for a script — upstream picks the alternate translator from
/// the `alphabets[]` table's `language` field the same way.  (Upstream #2479:
/// the automatic switch target is hardcoded there too; here it can be overridden
/// per language by [`alt_alphabet_voice`].)
fn script_voice(script: Script) -> &'static str {
    match script {
        Script::Latin => "en",
        Script::Cyrillic => "ru",
        Script::Greek => "el",
        Script::Arabic => "ar",
        Script::Hebrew => "he",
        Script::Armenian => "hy",
        Script::Georgian => "ka",
        Script::Devanagari => "hi",
        Script::Bengali => "bn",
        Script::Gurmukhi => "pa",
        Script::Gujarati => "gu",
        Script::Oriya => "or",
        Script::Tamil => "ta",
        Script::Telugu => "te",
        Script::Kannada => "kn",
        Script::Malayalam => "ml",
        Script::Sinhala => "si",
        Script::Thai => "th",
        Script::Myanmar => "my",
        Script::Ethiopic => "am",
        Script::Cherokee => "chr",
        Script::Hangul => "ko",
        Script::Kana => "ja",
        Script::Han => "cmn",
    }
}

/// The scripts a voice reads natively.  Anything not listed reads Latin, which
/// matches the shipped voices (the large majority are Latin-script languages).
///
/// Japanese reads both kana and kanji; Korean reads hangul and hanja — hence a
/// slice rather than a single script.
fn native_scripts(lang: &str) -> &'static [Script] {
    match primary_bcp47_subtag(lang) {
        "ru" | "uk" | "bg" | "be" | "mk" | "mn" | "kk" | "ky" | "tt" | "ba" | "cv" | "tg"
        | "os" | "cu" | "ab" | "nog" => &[Script::Cyrillic],
        // Serbo-Croatian is digraphic, and all three voices share one
        // dictionary (C: `strcpy(tr->dictionary_name, "hbs")`) and declare
        // Cyrillic as `langopts.our_alphabet` — so `sr "Beograd"` ==
        // `sr "Београд"`, and `bs`/`hr` read the Cyrillic pangram as
        // Serbo-Croatian instead of spelling it out or switching to Russian.
        "sr" | "bs" | "hr" => &[Script::Cyrillic, Script::Latin],
        "el" | "grc" => &[Script::Greek],
        "ar" | "fa" | "ur" | "sd" | "ps" | "ug" => &[Script::Arabic],
        "he" => &[Script::Hebrew],
        "hy" | "hyw" => &[Script::Armenian],
        "ka" => &[Script::Georgian],
        "hi" | "mr" | "ne" | "sa" | "kok" | "mai" => &[Script::Devanagari],
        "bn" | "as" | "bpy" => &[Script::Bengali],
        "pa" => &[Script::Gurmukhi],
        "gu" => &[Script::Gujarati],
        "or" => &[Script::Oriya],
        "ta" => &[Script::Tamil],
        "te" => &[Script::Telugu],
        "kn" => &[Script::Kannada],
        "ml" => &[Script::Malayalam],
        "si" => &[Script::Sinhala],
        "th" => &[Script::Thai],
        "my" | "shn" => &[Script::Myanmar],
        "am" | "ti" => &[Script::Ethiopic],
        "chr" => &[Script::Cherokee],
        "ko" => &[Script::Hangul, Script::Han],
        "ja" => &[Script::Kana, Script::Han],
        "cmn" | "yue" | "hak" | "zh" => &[Script::Han],
        _ => &[Script::Latin],
    }
}

/// Per-language override of the fallback voice for a script — upstream's
/// `langopts.alt_alphabet` / `alt_alphabet_lang` (#2511).  Arabic nominates
/// Mandarin for Han ideographs so they are read rather than spelled out.
fn alt_alphabet_voice(lang: &str, script: Script) -> Option<&'static str> {
    match (primary_bcp47_subtag(lang), script) {
        ("ar", Script::Han) => Some("cmn"),
        ("ka", Script::Cyrillic) => Some("ru"),
        _ => None,
    }
}

/// Whether the active dictionary has a spoken entry for `word` (used to keep a
/// listed word in the current language instead of switching by script).
fn dict_knows_word(word: &str, dict: &Dictionary) -> bool {
    let ctx = LookupCtx {
        lookup_symbol: true,
        ..Default::default()
    };
    matches!(lookup(dict, &word.to_lowercase(), &ctx), Some(r) if r.flags1.found())
}

/// The name of a single foreign letter's alphabet, if this language's list has
/// one (`_ar`, `_he`, …).  Greek and the scripts this voice reads are not named.
fn alphabet_name(word: &str, target: &str, dict: &Dictionary) -> Option<Vec<u8>> {
    let key = alphabet_key(word, target)?;
    let name = lookup_num_phonemes(dict, &key);
    (!name.is_empty() && name.at(0) != crate::phoneme::PHON_SWITCH).then_some(name)
}

/// The dictionary key of a single foreign letter's alphabet name.
fn alphabet_key(word: &str, target: &str) -> Option<String> {
    let mut chars = word.chars();
    let (c, None) = (chars.next()?, chars.next()) else {
        return None;
    };
    let key = match char_script(c)? {
        Script::Arabic
        | Script::Hebrew
        | Script::Devanagari
        | Script::Bengali
        | Script::Gujarati
        | Script::Tamil
        | Script::Telugu
        | Script::Kannada
        | Script::Malayalam
        | Script::Sinhala
        | Script::Armenian
        | Script::Georgian => format!("_{target}"),
        Script::Gurmukhi => "_gur".to_string(),
        // The jamo; a syllable is read by the Korean voice without one.
        Script::Hangul if ('\u{1100}'..='\u{11ff}').contains(&c) => "_ko".to_string(),
        _ => return None,
    };
    Some(key)
}

/// The rules meet a letter they have no rule for (C's `FLAG_SPELLWORD`).
fn rules_cannot_read(word: &str, dict: &Dictionary, lang_opts: &LangOptions) -> bool {
    if word.is_empty() || dict.rules().is_empty() {
        return false;
    }
    // (Not Korean, whose syllables are decomposed into jamo first.)
    if primary_bcp47_subtag(&lang_opts.lang) == "ko" {
        return false;
    }
    // (After the rules' `.replace` table, as the word itself is read.)
    let normalized = dict.apply_replacements(word);
    let word = normalized.as_deref().unwrap_or(word);
    let mut buf = Vec::with_capacity(word.len() + 3);
    buf.push(b' ');
    buf.extend_from_slice(word.as_bytes());
    buf.push(b' ');
    buf.push(0);
    let (mut vowels, mut stressed) = (0i32, 0i32);
    crate::dictionary::rules::untraced(|| {
        crate::dictionary::rules::translate_rules(
            dict,
            &buf,
            1,
            0,
            0,
            &dict.letter_bits,
            lang_opts.dict_condition,
            &mut vowels,
            &mut stressed,
        )
    })
    .spellword
}

/// Each letter of `word` read as a word: by the rules, or by its name where
/// the rules have none (Mixe "bardem" is `b'e 'a r d'e 'e 'm`).
fn spell_letter_words(
    word: &str,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    options: &LangOptions,
    expect: PosExpect,
) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    for c in word.chars() {
        let letter = c.to_string();
        // (Each piece has the whole word's place in the clause: C retranslates
        // it with the word's own flags, so French "aᵇ" ends the clause on `'a`.)
        let phonemes = if rules_cannot_read(&letter, dict, options) {
            letter_name(&letter, false, dict, phdata, stress_opts, options).unwrap_or_default()
        } else {
            let wr = word_to_phonemes_pos(&letter, dict, phdata, stress_opts, options, expect);
            let mut ph = wr.phonemes;
            promote_strend_stress(&mut ph, phdata, wr.dict_flags, expect.at_clause_end, false);
            ph
        };
        let phonemes: Vec<u8> = phonemes.into_iter().take_while(|&b| b != 0).collect();
        if phonemes.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(crate::phoneme::PHON_END_WORD);
        }
        out.extend(phonemes);
    }
    if !out.is_empty() {
        out.push(0);
    }
    out
}

/// Whether the language's **own letter rules** can read this word.
///
/// C never switches translator by script — its `alphabets[]` table nominates a
/// language only for *naming* an unknown letter — and several voices carry
/// rules for a second script: Serbo-Croatian is digraphic, Macedonian and Uzbek
/// have Latin transliterations (and Uzbek a Cyrillic one), Lingua Franca Nova a
/// Cyrillic orthography.  A word those rules cover must therefore stay in the
/// language, instead of being routed to whatever voice owns the script:
/// `mk "Dzidarski"` is `dz'id&Rsk,I`, not `d'@ z'@ I_:…`, and `uz "Ўзбекча"` is
/// `o:zb'ektS&`, not Russian.
fn rules_know_word(word: &str, dict: &Dictionary, lang_opts: &LangOptions) -> bool {
    if word.is_empty() || dict.rules().is_empty() {
        return false;
    }
    let mut buf = Vec::with_capacity(word.len() + 3);
    buf.push(b' ');
    buf.extend_from_slice(word.as_bytes());
    buf.push(b' ');
    buf.push(0);
    let (mut vowels, mut stressed) = (0i32, 0i32);
    let r = crate::dictionary::rules::translate_rules(
        dict,
        &buf,
        1,
        0,
        0,
        &dict.letter_bits,
        lang_opts.dict_condition,
        &mut vowels,
        &mut stressed,
    );
    // `spellword` means a letter had no rule at all — that is the case where the
    // word really is foreign to this language.
    !r.spellword && r.phonemes.iter().any(|&b| b != 0)
}

/// If `word` is written in a script the `lang` voice doesn't read (a Cyrillic or
/// Greek word inside an English clause, a Latin word inside a Russian one, Han
/// ideographs inside Arabic, …), return a voice that does, so the word is
/// re-translated instead of dropped.
fn foreign_script_voice(word: &str, lang: &str) -> Option<&'static str> {
    // The alphabet a language nominates another translator for is C's range,
    // not a script: Arabic's Mandarin takes Bopomofo `ㄅ` as well as Han.
    let (alt_offset, alt_lang) = alt_alphabet(lang);
    if alt_offset != 0
        && word
            .chars()
            .next()
            .and_then(alphabet_from_char)
            .is_some_and(|a| a.1 == alt_offset)
    {
        return Some(alt_lang);
    }
    let script = word.chars().find_map(char_script)?;
    if native_scripts(lang).contains(&script) {
        return None; // the current voice already reads this script
    }
    // A language explicitly nominated by `alt_alphabet` wins over the AL_WORDS
    // rule (upstream checks `langopts.alt_alphabet` first).
    if let Some(alt) = alt_alphabet_voice(lang, script) {
        return Some(alt);
    }
    // Otherwise only an `AL_WORDS` script switches; the rest get spelled.
    script_uses_words(script).then(|| script_voice(script))
}

/// C's `Unpronouncable`: is the first vowel too far into the word?
///
/// A word with more than `max_initial_consonants` consonants before its first
/// vowel (or none at all) is read letter by letter — that is how "BBC" becomes
/// `b,i:b,i:s'i:`.
fn is_unpronounceable(rest: &str, dict: &Dictionary, lang_opts: &LangOptions) -> bool {
    // (After the `.replace` table, which C applies as it reads the text:
    // English `ﬁsh` is "fish".)
    let replaced = dict.apply_replacements(rest);
    let rest = replaced.as_deref().unwrap_or(rest);
    // `LOPT_UNPRONOUNCABLE == 1` turns the test off for the language.
    if lang_opts.unpronouncable == 1 {
        return false;
    }
    // C refuses the test for a Latin letter in a language with another
    // alphabet ("so we can re-translate the word as English") and for a letter
    // of an alphabet not the language's own; a letter of no alphabet in the
    // table (runic `ᚠ`) takes it.
    let Some(first) = rest.chars().next() else {
        return false;
    };
    if dict.letter_bits_offset > 0 && (first as u32) < 0x241 {
        return false;
    }
    if alphabet_from_char(first).is_some_and(|a| a.1 != dict.letter_bits_offset) {
        return false;
    }
    // The test is made on Latin text, where a generic vowel test is reliable,
    // and on letters of no alphabet C knows (runic `ᚠᚡ`), which have no vowels
    // for it.  (Elsewhere the language's own vowel groups would be needed.)
    if dict.letter_bits_offset > 0 && (first as u32) >= 0x241 && alphabet_from_char(first).is_some()
    {
        return false;
    }
    let unlisted = |c: char| {
        ucd_isalpha(c)
            && (c as u32) >= 0x250
            && alphabet_from_char(c).is_none()
            && !matches!(c as u32, 0x1e00..=0x1eff)
    };
    let is_latin_letter = |c: char| (c.is_alphabetic() && (c as u32) < 0x250) || unlisted(c);
    let is_vowel = |c: char| {
        let base = crate::dictionary::rules::fold_accent(c);
        matches!(base, 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
            || crate::dictionary::rules::is_letter_wc(
                &dict.letter_bits,
                c as u32,
                dict.letter_bits_offset,
                7, // LETTERGP_VOWEL2
            )
    };
    if !is_latin_letter(first) {
        return false;
    }
    let mut count = 0usize;
    let mut vowel_posn = 9usize;
    for c in rest.chars() {
        if c == ' ' {
            break;
        }
        if !is_latin_letter(c) && c != '\'' {
            return false; // not our alphabet
        }
        if c == '\'' && count > 1 {
            break;
        }
        count += 1;
        if is_vowel(c) {
            vowel_posn = count;
            break;
        }
    }

    // `LOPT_UNPRONOUNCABLE == 2` (English, German, Spanish, …): C hands the
    // decision to the language's own rules — a word no start-of-word rule can
    // read is spelled.  The port asks the narrower question "is there a vowel at
    // all?", which agrees with upstream on the words that matter ("BBC" and
    // "FBI" are spelled; "brown", "street" and German "schnell" are not) without
    // depending on every anchored rule matching exactly.
    if lang_opts.unpronouncable == 2 && vowel_posn > 2 {
        // C's `Unpronouncable2`: no rule that starts the word.
        let mut buf = vec![b' '];
        buf.extend_from_slice(rest.split(' ').next().unwrap_or_default().as_bytes());
        buf.extend_from_slice(b"  ");
        buf.push(0);
        let (mut vowels, mut stressed) = (0i32, 0i32);
        let r = crate::dictionary::rules::untraced(|| {
            crate::dictionary::rules::translate_rules(
                dict,
                &buf,
                1,
                crate::dictionary::FLAG_UNPRON_TEST,
                0,
                &dict.letter_bits,
                lang_opts.dict_condition,
                &mut vowels,
                &mut stressed,
            )
        });
        return r.end_type == 0 || r.end_type & crate::dictionary::SUFX_UNPRON != 0;
    }

    if lang_opts.unpronouncable > 3 && first as u32 == lang_opts.unpronouncable {
        vowel_posn = vowel_posn.saturating_sub(1);
    }
    vowel_posn > lang_opts.max_initial_consonants + 1
}

/// The base letter and dictionary accent name of an accented Latin letter
/// (C's `letter_accents_0e0`).
fn letter_accent(c: char) -> Option<(char, &'static str)> {
    const ACCENTS: &[(&str, &str)] = &[
        ("_acu", "áéíóúýćĺńŕśź"),
        ("_brv", "ăĕğĭŏŭ"),
        ("_hac", "čďěľňřšťž"),
        ("_ced", "çģķļņŗşţ"),
        ("_cir", "âêîôûĉĝĥĵŝŵŷ"),
        ("_dia", "äëïöüÿ"),
        ("_ac2", "őű"),
        ("_dot", "ċėġż"),
        ("_grv", "àèìòù"),
        ("_mcn", "āēīōū"),
        ("_ogo", "ąęįų"),
        ("_rng", "åů"),
        ("_stk", "øđħłŧ"),
        ("_tld", "ãñõĩũ"),
    ];
    let name = ACCENTS.iter().find(|(_, letters)| letters.contains(c))?.0;
    let base = match c {
        'ø' => 'o',
        'đ' => 'd',
        'ħ' => 'h',
        'ł' => 'l',
        'ŧ' => 't',
        _ => crate::dictionary::rules::fold_accent(c),
    };
    (base != c).then_some((base, name))
}

/// Port of C's `LookupAccentedLetter`, before stress is placed.
fn accented_letter_name(word: &str, dict: &Dictionary, options: &LangOptions) -> Option<Vec<u8>> {
    let mut chars = word.trim_start_matches('_').chars();
    let (c, None) = (chars.next()?, chars.next()) else {
        return None;
    };
    let (base, name) = letter_accent(c)?;
    let ctx = LookupCtx {
        lookup_symbol: true,
        dict_condition: dict.dict_condition,
        ..Default::default()
    };
    let accent = lookup(dict, name, &ctx).filter(|r| !r.phonemes.is_empty())?;
    let letter = [format!("_{base}"), base.to_string()]
        .iter()
        .filter_map(|key| lookup(dict, key, &ctx))
        .find(|r| r.phonemes.first().is_some_and(|&b| b != 0))
        .map(|r| r.phonemes)?;
    const FLAG_ACCENT_BEFORE: u32 = 0x1000;
    let pause = crate::phoneme::PHON_PAUSE_VSHORT;
    // `langopts.accents & 1`: the accent's name comes first.
    let before = matches!(
        primary_bcp47_subtag(&options.lang),
        "af" | "bs" | "hr" | "sr" | "sv"
    ) || accent.flags1.0 & FLAG_ACCENT_BEFORE != 0;
    let mut out = Vec::new();
    if before {
        out.extend_from_slice(trim_lookup(&accent.phonemes));
        out.push(pause);
        out.push(PHON_STRESS_P);
        out.extend_from_slice(trim_lookup(&letter));
    } else {
        out.push(PHON_STRESS_2);
        out.extend_from_slice(trim_lookup(&letter));
        out.push(pause);
        out.extend_from_slice(trim_lookup(&accent.phonemes));
        out.push(pause);
    }
    out.push(0);
    Some(out)
}

/// A letter's name, stressed on its own (C's `LookupLetter`): the `_x` entry,
/// the plain one — looked up as a symbol, so French "a" is the letter and not
/// the verb — or whatever the rules make of it.
fn letter_name(
    lower: &str,
    non_initial: bool,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    options: &LangOptions,
) -> Option<Vec<u8>> {
    let ctx = LookupCtx {
        lookup_symbol: true,
        dict_condition: dict.dict_condition,
        ..Default::default()
    };
    let entry = [format!("_{lower}"), lower.to_string()]
        .iter()
        .filter_map(|key| lookup(dict, key, &ctx))
        .find(|r| {
            r.flags1.found()
                && r.flags1.textmode() == options.reversed_textmode
                && r.phonemes.first().is_some_and(|&b| b != 0)
        });
    let phonemes = match entry {
        Some(r) if r.phonemes.at(0) != crate::phoneme::PHON_SWITCH => {
            let mut phonemes = r.phonemes;
            let control = i32::from(non_initial);
            set_word_stress(&mut phonemes, phdata, stress_opts, Some(0), -1, control);
            phonemes
        }
        Some(_) => word_to_phonemes(lower, dict, phdata, stress_opts, options).phonemes,
        // C reads the letter with the rules alone, so a flag on its dictionary
        // entry (Aragonese `a $u+`) doesn't unstress it.
        None => letter_by_rules(lower, non_initial, dict, phdata, stress_opts, options)
            .unwrap_or_else(|| {
                word_to_phonemes(lower, dict, phdata, stress_opts, options).phonemes
            }),
    };
    phonemes.iter().any(|&b| b != 0).then_some(phonemes)
}

/// A letter's name from the rules, stressed on its own (`LookupLetter`).
/// [`letter_by_rules`] with the word before it in the buffer, for a rule's
/// pre-context.
fn letter_by_rules_after(
    prev: &[u8],
    lower: &str,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    options: &LangOptions,
) -> Option<Vec<u8>> {
    if dict.rules().is_empty() {
        return None;
    }
    let mut buf = vec![b' '];
    buf.extend_from_slice(prev);
    buf.push(b' ');
    let start = buf.len();
    buf.extend_from_slice(lower.as_bytes());
    buf.extend_from_slice(b"  ");
    buf.push(0);
    let (mut vowels, mut stressed) = (0i32, 0i32);
    let r = crate::dictionary::rules::untraced(|| {
        crate::dictionary::rules::translate_rules(
            dict,
            &buf,
            start,
            0,
            0,
            &dict.letter_bits,
            options.dict_condition,
            &mut vowels,
            &mut stressed,
        )
    });
    if r.spellword || !r.phonemes.iter().any(|&b| b != 0) {
        return None;
    }
    let mut phonemes = r.phonemes;
    if phonemes.last() != Some(&0) {
        phonemes.push(0);
    }
    set_word_stress(&mut phonemes, phdata, stress_opts, Some(0), -1, 0);
    Some(phonemes)
}

fn letter_by_rules(
    lower: &str,
    non_initial: bool,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    options: &LangOptions,
) -> Option<Vec<u8>> {
    if dict.rules().is_empty() {
        return None;
    }
    let mut buf = Vec::with_capacity(lower.len() + 4);
    buf.push(b' ');
    buf.extend_from_slice(lower.as_bytes());
    buf.push(b' ');
    buf.push(if LETTER_FOLLOWS.with(std::cell::Cell::get) {
        crate::dictionary::RULE_SPELLING
    } else {
        b' '
    });
    buf.push(0);
    let (mut vowels, mut stressed) = (0i32, 0i32);
    let r = crate::dictionary::rules::untraced(|| {
        crate::dictionary::rules::translate_rules(
            dict,
            &buf,
            1,
            0,
            0,
            &dict.letter_bits,
            options.dict_condition,
            &mut vowels,
            &mut stressed,
        )
    });
    if r.spellword || !r.phonemes.iter().any(|&b| b != 0) {
        return None;
    }
    let mut phonemes = r.phonemes;
    if phonemes.last() != Some(&0) {
        phonemes.push(0);
    }
    set_word_stress(
        &mut phonemes,
        phdata,
        stress_opts,
        Some(0),
        -1,
        i32::from(non_initial),
    );
    Some(phonemes)
}

/// `$combine` (C's `FLAG_COMBINE`): the word joins the next one.
const FLAG_COMBINE: u32 = 0x0080_0000;

/// `CombineFlag`: the word read together with the one after it, stressed as one
/// word, or `None` where C keeps them apart.
fn combine_with_next(
    first: &WordResult,
    tokens: &[Token],
    token_ix: usize,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    options: &LangOptions,
) -> Option<WordResult> {
    // `LOPT_COMBINE_WORDS`: the most syllables the result may have (0 = any),
    // and bit 8: only if the second word is marked `$alt`.
    let limit: u32 = match primary_bcp47_subtag(&options.lang) {
        "cs" => 4,
        "en" => 2,
        "pl" => 4 | 0x100,
        _ => 0,
    };
    let next = match (tokens.get(token_ix + 1), tokens.get(token_ix + 2)) {
        (Some(Token::Space | Token::WordJoin), Some(Token::Word(next))) => next,
        _ => return None,
    };
    let Some(c) = next.chars().next().filter(|c| c.is_alphabetic()) else {
        return None;
    };
    // A word of another alphabet is read in another language: a switch, which
    // C doesn't combine (Bulgarian "със A" is `s@s (en)'eI(bg)`).
    if dict.letter_bits_offset > 0 && (c as u32) < 0x241
        || alphabet_from_char(c).is_some_and(|a| a.1 != dict.letter_bits_offset)
    {
        return None;
    }
    let lower = lowercase_for_lang(next, &options.lang);
    let second = word_to_phonemes(&lower, dict, phdata, stress_opts, options);
    const FLAG_ALT_TRANS: u32 = 0x8000;
    if second.phonemes.first() == Some(&crate::phoneme::PHON_SWITCH)
        || limit & 0x100 != 0 && second.dict_flags & FLAG_ALT_TRANS == 0
    {
        return None;
    }
    // C translates the two as one word.  When the first has no pronunciation
    // of its own, the rules read the whole: Polish "nie wiem" is one word.
    let syllables_in = |ph: &[u8]| {
        ph.iter()
            .filter(|&&c| {
                phdata
                    .get(c)
                    .is_some_and(|ph| ph.typ == crate::phoneme::PH_VOWEL)
            })
            .count()
    };
    if !first.found_in_list {
        let first_word = match token_ix.checked_sub(0).and_then(|k| tokens.get(k)) {
            Some(Token::Word(w)) => lowercase_for_lang(w, &options.lang),
            _ => return None,
        };
        let whole = word_to_phonemes(
            &format!("{first_word}-{lower}"),
            dict,
            phdata,
            stress_opts,
            options,
        );
        if whole.phonemes.first() == Some(&crate::phoneme::PHON_SWITCH)
            || limit & 0x1f != 0 && syllables_in(&whole.phonemes) > (limit & 0x1f) as usize
        {
            return None;
        }
        // One word has one primary stress.
        let mut whole = whole;
        let mut seen = false;
        for c in &mut whole.phonemes {
            if *c == PHON_STRESS_P {
                if seen {
                    *c = PHON_STRESS_3;
                }
                seen = true;
            }
        }
        return Some(WordResult {
            dict_flags: if whole.dict_flags == 0 {
                second.dict_flags
            } else {
                whole.dict_flags
            },
            skipwords: 0,
            ..whole
        });
    }
    let unstressed = |ph: &[u8]| -> Vec<u8> {
        ph.iter()
            .copied()
            .take_while(|&c| c != 0)
            .filter(|c| !(2..=7).contains(c) && *c != crate::phoneme::PHON_END_WORD)
            .collect()
    };
    let mut phonemes = unstressed(&first.phonemes);
    phonemes.extend(unstressed(&second.phonemes));
    let syllables = phonemes
        .iter()
        .filter(|&&c| {
            phdata
                .get(c)
                .is_some_and(|ph| ph.typ == crate::phoneme::PH_VOWEL)
        })
        .count();
    if limit & 0x1f != 0 && syllables > (limit & 0x1f) as usize {
        return None;
    }
    phonemes.push(0);
    set_word_stress(&mut phonemes, phdata, stress_opts, Some(0), -1, 0);
    Some(WordResult {
        phonemes,
        dict_flags: second.dict_flags,
        found_in_list: second.found_in_list,
        skipwords: 0,
    })
}

/// Whether `SetSpellingStress` reduces the `count`-th of `n_stress` letter names:
/// `langopts.spelling_stress` keeps the first primary, other languages the last.
fn spelling_stress_reduced(count: usize, n_stress: usize, options: &LangOptions) -> bool {
    let initial = matches!(
        primary_bcp47_subtag(&options.lang),
        "bs" | "cs"
            | "fi"
            | "hr"
            | "hu"
            | "kn"
            | "lv"
            | "ml"
            | "si"
            | "sk"
            | "smj"
            | "sr"
            | "ta"
            | "te"
    );
    if initial {
        count > 1
    } else {
        count != n_stress && (count % 3 != 0 || count + 1 == n_stress)
    }
}

/// Spell the leading letters of an unpronounceable word, returning them and the
/// remainder still to translate — the `Unpronouncable` loop of `TranslateWord3`.
fn spell_unpronounceable<'a>(
    word: &'a str,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    options: &LangOptions,
) -> Option<(Vec<u8>, &'a str)> {
    if word.chars().count() < 2 {
        return None;
    }
    let mut out: Vec<u8> = Vec::new();
    let mut rest = word;
    let mut posn = 0usize;
    let mut length = 999usize;
    while (length < 3 && length > 0)
        || (word.chars().count() > 1 && is_unpronounceable(rest, dict, lang_opts_ref(options)))
    {
        let Some(c) = rest.chars().next() else { break };
        if c == '\'' {
            break;
        }
        let lower = c.to_lowercase().to_string();
        // A letter with no name is said by `TranslateLetter` ("letter 16a0"
        // for runic), when that is all in this language.
        let follows = rest.chars().nth(1).is_some_and(|n| n != ' ');
        let saved = LETTER_FOLLOWS.with(|f| f.replace(follows));
        let named = letter_name(&lower, posn > 0, dict, phdata, stress_opts, options);
        LETTER_FOLLOWS.with(|f| f.set(saved));
        let name = match named {
            Some(name) => name,
            None => {
                let parts = letter_parts(c, dict, phdata, stress_opts, options);
                let mut local = Vec::new();
                for part in parts {
                    match part {
                        LetterPart::Local(ph) => local.extend(ph),
                        // (English names no letter of a script outside C's
                        // alphabet table.)
                        LetterPart::Letter { otherwise, .. } if alphabet_from_char(c).is_none() => {
                            local.extend(otherwise);
                        }
                        _ => return None,
                    }
                }
                local
            }
        };
        out.extend(name.iter().copied().filter(|&b| b != 0));
        rest = &rest[c.len_utf8()..];
        posn += 1;
        length = rest.chars().count();
        if posn > 16 {
            break;
        }
    }
    if posn == 0 {
        return None;
    }
    let n_stress = out.iter().filter(|&&b| b == PHON_STRESS_P).count();
    let mut count = 0;
    for b in out.iter_mut() {
        if *b == PHON_STRESS_P {
            count += 1;
            if spelling_stress_reduced(count, n_stress, options) {
                *b = PHON_STRESS_3;
            }
        }
    }
    Some((out, rest))
}

/// Identity helper so `is_unpronounceable` can take the options by reference.
fn lang_opts_ref(o: &LangOptions) -> &LangOptions {
    o
}

/// Spell a word as its letters' *names*, run together as one word — C's
/// `SpeakIndividualLetters` with `spell_word == 1`.
///
/// `LookupLetter2` asks for the name (`_a`) before the letter-as-a-word, so "a"
/// is "ay" rather than the article, and `SetSpellingStress` then demotes every
/// primary stress but the last: English `usa` reads `j,u:,Es'eI`.
fn spell_word_letters(
    word: &str,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    options: &LangOptions,
) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    let letters: Vec<char> = word.chars().filter(|c| c.is_alphanumeric()).collect();
    for (k, &c) in letters.iter().enumerate() {
        let lower = c.to_lowercase().to_string();
        let saved = LETTER_FOLLOWS.with(|f| f.replace(k + 1 < letters.len()));
        let name = letter_name(&lower, !out.is_empty(), dict, phdata, stress_opts, options);
        LETTER_FOLLOWS.with(|f| f.set(saved));
        let name = name?;
        out.extend(name.iter().copied().filter(|&b| b != 0));
    }
    if out.is_empty() {
        return None;
    }
    // `SetSpellingStress`: keep the last primary, reduce the others.
    let n_stress = out.iter().filter(|&&b| b == PHON_STRESS_P).count();
    let mut count = 0;
    for b in out.iter_mut() {
        if *b == PHON_STRESS_P {
            count += 1;
            if spelling_stress_reduced(count, n_stress, options) {
                *b = PHON_STRESS_3;
            }
        }
    }
    Some(out)
}

/// Downgrade a clause-ending period that immediately follows a title
/// abbreviation to a plain word break, so "Dr. Smith" reads without a sentence
/// pause (upstream #2340).  A period after any other word (a real sentence end,
/// "the end. Next.") is untouched.  Adjacency is implicit: a space before the
/// period would make the previous token a `Space`, not the abbreviation `Word`.
/// English-only.
fn suppress_abbreviation_periods(tokens: &mut [Token], ordinal_dot: bool, lowercase_sentence: bool) {
    // A period only ends a sentence when a capitalised word (or nothing)
    // follows — "one. two" is not a break, "one. Two" is.  Upstream's `-x`
    // shows exactly this: `w'0n t'u:` on one line versus `w'0n` / `t'u:` on
    // two.  Language-independent, like the clause tokenizer in C.
    // Scanned backwards so the word that follows is already known; a long run of
    // dots would otherwise rescan its own tail for each one.
    // (After any word: C's `FLAG_HAS_DOT`, for Roman ordinals, dotted
    // abbreviations ("e.g.") and `$hasdot` entries.)
    let roman = |t: Option<&Token>| {
        matches!(t, Some(Token::Word(w))
            if w.chars().last().is_some_and(char::is_alphabetic))
    };
    // With `NUM_ORDINAL_DOT`, a dot after a Roman number never ends the
    // clause (C's `IsRomanU` test in `ReadClause`).
    for i in 1..tokens.len() {
        let upper_roman = matches!(tokens.get(i - 1), Some(Token::Word(w))
            if !w.is_empty() && w.chars().all(|c| "IVXL".contains(c)));
        if ordinal_dot
            && upper_roman
            && matches!(tokens.get(i), Some(Token::ClauseBoundary('.')))
            && tokens.get(i + 1).is_some()
        {
            tokens.put(i, Token::Dot);
        }
    }
    // C's `ReadClause`, mark by mark.  With no letter or digit yet in the
    // clause (`any_alnum`) a mark doesn't end it: a dot becomes a space, any
    // other is read as running text (English `!` is "exclamation", once for a
    // run of them).  Otherwise a dot ends it unless the next character after
    // its spaces is a lower-case letter ("one. two" is one clause, "one. Two"
    // and `cat . . dog` two); one that doesn't stands as a word when a space
    // is before it ("cypress . made" says "dot").
    let mut any_alnum = false;
    let mut last_unended: Option<usize> = None;
    for i in 0..tokens.len() {
        match tokens.get(i).cloned() {
            Some(Token::ClauseBoundary('.')) if !any_alnum => tokens.put(i, Token::Space),
            Some(Token::ClauseBoundary('.')) => {
                let next = tokens
                    .get(i + 1..)
                    .unwrap_or_default()
                    .iter()
                    .find(|t| !matches!(t, Token::Space | Token::WordJoin | Token::Dot));
                // (A dot glued to another is no end either: `cat .. dog`.)
                let continues = matches!(tokens.get(i + 1), Some(Token::ClauseBoundary('.')))
                    || !lowercase_sentence
                        && matches!(next, Some(Token::Word(w))
                            if w.chars().next().is_some_and(char::is_lowercase));
                if continues {
                    let spaced = matches!(
                        i.checked_sub(1).and_then(|k| tokens.get(k)),
                        Some(Token::Space)
                    );
                    let before_roman = i > 0 && roman(tokens.get(i - 1));
                    tokens.put(
                        i,
                        if before_roman {
                            Token::Dot
                        } else if spaced {
                            Token::Punctuation('.')
                        } else {
                            Token::Space
                        },
                    );
                } else {
                    any_alnum = false;
                }
            }
            Some(Token::ClauseBoundary(c)) if !any_alnum => {
                // (Glued to the one before: "!!!".)
                let repeat = i > 0 && last_unended == Some(i - 1);
                last_unended = Some(i);
                tokens.put(
                    i,
                    if repeat {
                        Token::Space
                    } else {
                        Token::Punctuation(c)
                    },
                );
            }
            Some(Token::ClauseBoundary(_)) => any_alnum = false,
            Some(Token::Word(_) | Token::Number(_)) => any_alnum = true,
            _ => {}
        }
    }

    // A comma directly between two numbers is a digit separator, not a clause
    // break: `en "3,14"` reads "three fourteen" on one line upstream, where the
    // port used to add a clause break as well as the space.
    for i in 1..tokens.len().saturating_sub(1) {
        if matches!(tokens.get(i), Some(Token::ClauseBoundary(',')))
            && matches!(tokens.get(i - 1), Some(Token::Number(_)))
            && matches!(tokens.get(i + 1), Some(Token::Number(_)))
        {
            tokens.put(i, Token::Space);
        }
    }
}

/// Look up the spoken name of a standalone symbol via the dictionary's
/// character-name entries (`_%`, `_$`, …), mirroring `LookupCharName`.
/// Returns the phoneme bytes, or empty if the symbol has no name in this voice.
/// Which name a symbol character should be read by.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SymbolName {
    /// The character appears in running text (`a − b`).  The dictionary's own
    /// entry for the character wins.
    Inline,
    /// Punctuation being announced by name (`--punct`), where C calls
    /// `LookupCharName()` and reads the `_<char>` entry — `&` is "ampersand"
    /// here but "and" inline.
    Announce,
    /// An arithmetic operator: the word for the *operation* wins over the
    /// dictionary's name for the character ("minus", not "minus sign").
    Operator,
}

/// How C classifies this character as clause punctuation
/// (`clause_type_from_codepoint`): which clause type it ends, and whether it
/// ends one even when no space follows (`OPTIONAL_SPACE_AFTER` — the Devanagari
/// danda and the CJK full stop do, which is how scripts written without spaces
/// get sentence breaks at all).  `None` for everything else.
///
/// Transcribed from the `ESPEAKNG_PROPERTY_*` clause-type properties in
/// `ucd-tools/src/proplist.c`, keeping only the property combinations C's
/// `switch` maps to a clause type: U+2048/U+2049 (`⁈`/`⁉`) carry both
/// QUESTION_MARK and EXCLAMATION_MARK, hit no case, and so *are* spoken by name.
fn clause_punctuation_class(c: char) -> Option<(ClauseTerminator, bool)> {
    use ClauseTerminator::{Comma, Exclamation, Period, Question};
    let code = c as u32;
    let class = match code {
        0x002C
        | 0x003A..=0x003B
        | 0x0387
        | 0x055D
        | 0x060C
        | 0x061B
        | 0x0702
        | 0x0706..=0x0708
        | 0x07F8
        | 0x0F14
        | 0x1363..=0x1366
        | 0x1802
        | 0x1804
        | 0x1808
        | 0x2013..=0x2014
        | 0x204F
        | 0x2982
        | 0x2E32
        | 0x2E34..=0x2E35
        | 0x2E3A..=0x2E3B
        | 0x2E41
        | 0xA4FE
        | 0xA60D
        | 0xA6F4..=0xA6F6
        | 0xFE10..=0xFE11
        | 0xFE13..=0xFE14
        | 0xFE31..=0xFE32
        | 0xFE50..=0xFE51
        | 0xFE54..=0xFE55
        | 0xFF64
        | 0x1144D
        | 0x12471..=0x12472
        | 0x1DA87
        | 0x1DA89..=0x1DA8A
        | 0x1F101..=0x1F10A
        | 0xE002C
        | 0xE003A..=0xE003B => (Comma, false),
        0x00A1
        | 0x00BF
        | 0x0EAF
        | 0x1801
        | 0x2026
        | 0x22EE..=0x22F1
        | 0x3001
        | 0xFE19
        | 0xFF0C
        | 0xFF1A..=0xFF1B => (Comma, true),
        0x0021
        | 0x055B..=0x055C
        | 0x0703
        | 0x07F9
        | 0x1944
        | 0x203C
        | 0x2755
        | 0x2757
        | 0x2762..=0x2763
        | 0xFE15
        | 0xFE57
        | 0x1E95E
        | 0xE0021 => (Exclamation, false),
        0xFF01 => (Exclamation, true),
        0x002E
        | 0x06D4
        | 0x0700..=0x0701
        | 0x0704
        | 0x0965
        | 0x0F0E
        | 0x10FB
        | 0x1362
        | 0x1368
        | 0x166E
        | 0x1803
        | 0x1809
        | 0x2488..=0x249B
        | 0x2CF9
        | 0x2CFE
        | 0x2E33
        | 0x2E3C
        | 0xA4FF
        | 0xA60E
        | 0xA6F3
        | 0xFE12
        | 0xFE52
        | 0xFF61
        | 0x16AF5
        | 0x1BC9F
        | 0x1DA88
        | 0x1F100
        | 0xE002E => (Period, false),
        0x0589 | 0x0964 | 0x0DF4 | 0x0F0D | 0x3002 | 0xFF0E => (Period, true),
        0x003F
        | 0x037E
        | 0x055E
        | 0x061F
        | 0x0709
        | 0x1367
        | 0x1945
        | 0x2047
        | 0x2753..=0x2754
        | 0x2CFA..=0x2CFB
        | 0xA60F
        | 0xA6F7
        | 0xFE16
        | 0xFE56
        | 0x11143
        | 0x1E95F
        | 0xE003F => (Question, false),
        0xFF1F => (Question, true),
        _ => return None,
    };
    Some(class)
}

/// Whether the character is clause punctuation at all — it breaks the clause
/// instead of being spoken, so it never reaches the dictionary for a name.
fn is_clause_punctuation(c: char) -> bool {
    clause_punctuation_class(c).is_some()
}

/// A bracket after the clause's last word: nothing but spaces and brackets
/// follow it in the clause.  It starts no word of its own (C finds no source
/// character for it, so its `sourceix` is 0), and a name it is spoken by runs
/// into the word before: Ukrainian `"x"` ends `'Eks(uk)podv'i:jna l'apka`.
fn is_trailing_bracket(tokens: &[Token], k: usize) -> bool {
    matches!(tokens.get(k), Some(Token::Punctuation(b)) if is_bracket(*b))
        && tokens
            .get(k + 1..)
            .unwrap_or_default()
            .iter()
            .take_while(|t| !matches!(t, Token::ClauseBoundary(_)))
            .all(|t| matches!(t, Token::Space) || matches!(t, Token::Punctuation(b) if is_bracket(*b)))
}

/// C's `IsBracket()`: a bracket or quote, which pauses rather than speaks.
fn is_bracket(c: char) -> bool {
    matches!(c,
        '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '"' | '\'' | '`'
        | '\u{ab}' | '\u{bb}'          // « »
        | '\u{300a}' | '\u{300b}')     // 《 》
        || matches!(c as u32, 0x2014..=0x201F) // dashes and the curly quotes
}

/// Does the dictionary give this character a name of its own — a bare
/// single-character list entry such as `#` → "hash" (`en_list`) or `−` →
/// "mínusz" (`hu_emoji`)?
///
/// This is the test C effectively applies to a symbol that is not clause
/// punctuation: the character reaches `TranslateWord()` like any other token, so
/// a list entry for it is spoken and anything else falls through.  The `_<char>`
/// *character-name* entries (`_(` → "left paren") are deliberately **not**
/// consulted here — C speaks those only when spelling or announcing
/// punctuation, which is why `a ( b` is silent while `a − b` is not.
fn dict_names_symbol(dict: &Dictionary, c: char, dict_condition: u32) -> bool {
    dict_symbol_flags(dict, c, dict_condition).is_some()
}

/// The dictionary flags of a bare single-character entry, when it has one.
/// `$max3` lives here: a symbol entry carrying it is spoken at most three times
/// in a row (C's `phonemes_repeat_count`), which is what keeps `~ ~ ~ ~ ~ ~`
/// to three tildes.
fn dict_symbol_flags(dict: &Dictionary, c: char, dict_condition: u32) -> Option<u32> {
    // Clause punctuation goes to the clause path, not the dictionary: Hindi
    // `।`, Ethiopic `።` and CJK `。` all have list entries naming them, and C
    // speaks none of them (`a ❓ b` → "a b").
    if is_clause_punctuation(c) {
        return None;
    }
    // The entry may be behind a `?n` condition — Portuguese names `§` twice,
    // once per variety (`?1 §` / `?2 §`) — so the language's own conditions have
    // to be in the context or neither matches.
    let ctx = LookupCtx {
        lookup_symbol: true,
        dict_condition,
        ..Default::default()
    };
    lookup(dict, &c.to_string(), &ctx)
        .filter(|r| r.flags1.found() && !r.phonemes.is_empty())
        .map(|r| r.flags1.0)
}

/// A symbol's reading, and whether it came from the dictionary **as phonemes**
/// (C's `SFLAG_DICTIONARY`, set when `FLAG_FOUND && !FLAG_TEXTMODE`).  The
/// difference is visible: `pl @` has phonemes in `pl_list`, so the
/// stress-conditioned changes don't touch it (`m'awpa`), while `tr −` has
/// *replacement text* in `tr_emoji`, which is re-translated by the rules and so
/// does take them (`eksi` → `Eks'I`).
fn lookup_symbol_name(
    c: char,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    options: &LangOptions,
    mode: SymbolName,
    expect: PosExpect,
) -> (Vec<u8>, bool) {
    // The dictionary's own entry for the character comes first.  Every shipped
    // language carries the symbol block of its `*_emoji` data (86 of them name
    // U+2212 MINUS SIGN alone) and those are the readings C speaks, including
    // the ones the table below cannot express: the pause markers around `=`
    // (`_:'i:kw@Lz_:`), "sum sign" rather than "sum" for `∑`, and every
    // currency, arrow, fraction and sign the table never listed.
    // An operator is read by translating the character as a word — through the
    // language's list *and* its letter rules, which is where C finds it (`fi`
    // `__) - (  mi:nus`, `el`'s `- m'ion` entry).  The table below is only the
    // fallback for languages whose data has nothing.
    if mode == SymbolName::Operator {
        let wr = word_to_phonemes_pos(&c.to_string(), dict, phdata, stress_opts, options, expect);
        if wr.phonemes.iter().any(|&b| b != 0) {
            return (wr.phonemes, wr.found_in_list);
        }
    }
    if mode != SymbolName::Operator
        && mode != SymbolName::Announce
        && dict_names_symbol(dict, c, options.dict_condition)
    {
        // (A text entry that reads as nothing — kanji the Japanese rules can't
        // read, German "Copyright" sent to English — leaves the symbol silent.)
        let wr = word_to_phonemes_pos(&c.to_string(), dict, phdata, stress_opts, options, expect);
        return (wr.phonemes, wr.found_in_list);
    }
    // A dot in running text (not after a number, which reads its `_.`): C's
    // `LookupLetter` takes a plain `.` entry as a word, and says nothing for
    // a `_.` one (German "cat.dog" has no "Punkt").
    if mode == SymbolName::Inline && c == '.' {
        if let Some(ph) = letter_by_rules(".", false, dict, phdata, stress_opts, options) {
            return (ph, false);
        }
        let ctx = LookupCtx {
            lookup_symbol: true,
            dict_condition: options.dict_condition,
            ..Default::default()
        };
        if let Some(r) = lookup(dict, ".", &ctx).filter(|r| r.flags1.found()) {
            let mut ph = trim_lookup(&r.phonemes).to_vec();
            if !ph.is_empty() {
                ph.push(0);
                set_word_stress(&mut ph, phdata, stress_opts, Some(r.flags1.0), -1, 0);
                return (ph, true);
            }
        }
        // (Still a word break.)
        return (vec![PHON_END_WORD, 0], false);
    }
    if mode == SymbolName::Inline {
        // The language's rules come first (Swedish reads `/` by them).
        let prev: Vec<u8> = expect
            .prev_word
            .iter()
            .copied()
            .take_while(|&b| b != 0)
            .collect();
        if let Some(ph) = if prev.is_empty() {
            letter_by_rules(&c.to_string(), false, dict, phdata, stress_opts, options)
        } else {
            letter_by_rules_after(&prev, &c.to_string(), dict, phdata, stress_opts, options)
        } {
            return (ph, false);
        }
        // (Clause punctuation read as a word: its plain entry, Czech `!`.)
        if is_clause_punctuation(c) {
            let ctx = LookupCtx {
                lookup_symbol: true,
                dict_condition: options.dict_condition,
                ..Default::default()
            };
            if let Some(r) = lookup(dict, &c.to_string(), &ctx).filter(|r| r.flags1.found()) {
                if r.flags1.textmode() != options.reversed_textmode {
                    let text = String::from_utf8_lossy(trim_lookup(&r.phonemes)).into_owned();
                    let mut out = Vec::new();
                    for w in text.split_whitespace() {
                        let wr =
                            word_to_phonemes(&w.to_lowercase(), dict, phdata, stress_opts, options);
                        if !out.is_empty() {
                            out.push(PHON_END_WORD);
                        }
                        out.extend_from_slice(trim_lookup(&wr.phonemes));
                    }
                    out.push(0);
                    return (out, false);
                }
                let mut ph = trim_lookup(&r.phonemes).to_vec();
                if !ph.is_empty() {
                    ph.push(0);
                    set_word_stress(&mut ph, phdata, stress_opts, Some(r.flags1.0), -1, 0);
                    return (ph, true);
                }
            }
        }
        // Then C's `LookupLetter` for running text: a character given only as
        // `_c` is "specified as _* so ignore it when speaking normal text"
        // (French `#`, `/`, `=`), and otherwise English is asked for a plain
        // entry.
        // (A point or comma is named by the number code instead: German
        // `3.14` is "drei Punkt vierzehn".)
        if !trim_lookup(&lookup_num_phonemes(dict, &format!("_{c}"))).is_empty() {
            if c == '.' {
                // (As the list gives it, unstressed: `pʊŋkt`.)
                let mut ph = trim_lookup(&lookup_num_phonemes(dict, &format!("_{c}"))).to_vec();
                ph.push(0);
                return (ph, true);
            }
            return (Vec::new(), false);
        }
        if primary_bcp47_subtag(&options.lang) != "en" {
            let mut switch = vec![crate::phoneme::PHON_SWITCH];
            switch.extend_from_slice(b"en");
            return (switch, false);
        }
        return (Vec::new(), false);
    }
    // An operator stops here.  C reads it by translating the character (above),
    // so when the language's data has no reading there is nothing to say — it
    // does *not* fall back to the character's spell-out name, which would make
    // `-5` "hyphen five" (fr "tiret", pl "myślnik"), nor to another language.
    // The table above is this port's own addition for the languages whose data
    // is silent; past it, silence is the right answer.
    if mode == SymbolName::Operator {
        return (Vec::new(), false);
    }
    // Otherwise use the dictionary character-name entry (`_%`, `_$`, …).
    for key in [format!("_{c}"), c.to_string()] {
        let wr = word_to_phonemes(&key, dict, phdata, stress_opts, options);
        // A result of nothing but terminators is not a reading: it used to be
        // returned as one, which silenced the fallback below (`fi -5` lost its
        // "miinus" because `_−` came back as a lone NUL).
        if wr.phonemes.iter().any(|&b| b != 0) {
            return (wr.phonemes, wr.found_in_list);
        }
    }
    // Still nothing: C's `LookupCharName` names the character in the **default
    // voice** instead of dropping it (`language = L('e','n')` when the
    // character's alphabet nominates nothing), wrapping the result in a
    // language switch — `sv #` reads `(en)h'aS(sv)`.  Returning the marker
    // hands that to the switch machinery, which re-translates the character
    // itself in the target language; if English has no name either, nothing is
    // spoken, as in C.
    if primary_bcp47_subtag(&options.lang) != "en" {
        let mut switch = vec![crate::phoneme::PHON_SWITCH];
        switch.extend_from_slice(b"en");
        return (switch, false);
    }
    (Vec::new(), false)
}

/// Lower-case a word the way the language does — C's `towlower2`.
///
/// Two differences from Rust's `char::to_lowercase`, which applies the *full*
/// Unicode mapping: `İ` (U+0130) lower-cases to a bare `i` rather than `i` plus
/// a combining dot (the dot broke the following rule match, so `tr İstanbul`
/// lost its `s` and read `itanb'UL`), and in Turkish and Azerbaijani `I` is the
/// *dotless* `ı` — C's `langopts.dotless_i`, which only those two set.  So `tr
/// IŞIK` is `@S'@k`, not `iS'Ik`.
fn lowercase_for_lang(word: &str, lang: &str) -> String {
    let dotless_i = matches!(primary_bcp47_subtag(lang), "tr" | "az");
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        match c {
            'I' if dotless_i => out.push('ı'),
            'İ' => out.push('i'), // the simple mapping, as `ucd_tolower` gives
            _ => out.extend(c.to_lowercase()),
        }
    }
    out
}

/// A symbol token that stands as a word for multi-word dictionary entries.
fn symbol_word(t: &Token) -> bool {
    matches!(t, Token::Punctuation(c) | Token::Operator(c)
        if !c.is_alphanumeric() && !is_clause_punctuation(*c) && !is_bracket(*c) && !matches!(*c, '\'' | '"' | '-'))
}

/// The first few UTF-8 bytes of the next word in the clause, for the rule
/// post-contexts that reach across a word boundary.  A *number* counts: the
/// hyphen of `-5` is read "minus" by a rule whose post-context is "boundary then
/// a digit" (`__) - (_D`), so the digits have to be visible.
/// The words from `tokens[from..]` up to anything but a word, a space or a
/// joining hyphen, lower case and each followed by a space — the clause
/// text C's multi-word dictionary entries compare with (`word2`).  A hyphen
/// counts as a space, as it does in C's clause buffer, so "celle-là" finds
/// `(celle là)`.
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
fn following_words(tokens: &[Token], from: usize, lang: &str) -> [u8; 32] {
    let mut text = String::new();
    for t in tokens.get(from.min(tokens.len())..).unwrap_or_default() {
        match t {
            Token::Space | Token::WordJoin => {}
            Token::Word(w) => {
                text.push_str(&lowercase_for_lang(w, lang));
                text.push(' ');
                if text.len() >= 32 {
                    break;
                }
            }
            // A symbol is a word of the clause too: English `(a &)`, `(at & t)`.
            t if symbol_word(t) => {
                if let Token::Punctuation(c) | Token::Operator(c) = t {
                    text.push(*c);
                    text.push(' ');
                }
            }
            _ => break,
        }
    }
    let mut out = [0u8; 32];
    let b = text.as_bytes();
    let n = (0..=b.len().min(32))
        .rev()
        .find(|&k| text.is_char_boundary(k))
        .unwrap_or(0);
    out[..n].copy_from_slice(b.get(..n).unwrap_or_default());
    out
}

#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
fn next_word_opening(tokens: &[Token], from: usize, lang: &str) -> [u8; 8] {
    let mut out = [0u8; 8];
    let Some(next) = tokens
        .get(from..)
        .unwrap_or_default()
        .iter()
        .take_while(|t| !matches!(t, Token::ClauseBoundary(_)))
        .find_map(|t| match t {
            Token::Word(w) => Some(lowercase_for_lang(w, lang)),
            Token::Number(n) => Some(n.surface().to_string()),
            _ => None,
        })
    else {
        return out;
    };
    // C's clause buffer has an extra space between a word and a number after
    // it ("to distinguish 'a 2' from 'a2'"), so `_) a (_D` doesn't match.
    let after_word = matches!(
        from.checked_sub(1).and_then(|k| tokens.get(k)),
        Some(Token::Word(w)) if w.chars().last().is_some_and(char::is_alphabetic)
    );
    let number_next = tokens
        .get(from..)
        .unwrap_or_default()
        .iter()
        .find(|t| matches!(t, Token::Word(_) | Token::Number(_)))
        .is_some_and(|t| matches!(t, Token::Number(_)));
    let next = if after_word && matches!(tokens.get(from), Some(Token::Space)) && number_next {
        format!(" {next}")
    } else {
        next
    };
    let b = next.as_bytes();
    let n = (0..=b.len().min(8))
        .rev()
        .find(|&k| next.is_char_boundary(k))
        .unwrap_or(0);
    out[..n].copy_from_slice(b.get(..n).unwrap_or_default());
    out
}

#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
pub(crate) fn build_translate_entries(
    tokens: &[Token],
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    options: &LangOptions,
) -> Vec<TranslateEntry> {
    let mut entries: Vec<TranslateEntry> = Vec::with_capacity(tokens.len());
    // `$max3` bookkeeping: which symbol is repeating, and how many times so far.
    let mut repeat_of_symbol: Option<char> = None;
    let mut repeat_count = 0u32;
    // Clause position per token, for the `$atend` / `$atstart` dictionary
    // entries: a word is "at the clause end" when no further word follows before
    // the next clause boundary (C: `word_end < translator->clause_end`).
    let is_word = |t: &Token| matches!(t, Token::Word(_) | Token::Number(_));
    let mut at_end = vec![false; tokens.len()];
    let mut at_start = vec![false; tokens.len()];
    // The clause a token is in ends a sentence (`. ? !` or the end of the
    // text), for `$sentence` entries.
    let mut in_sentence = vec![true; tokens.len()];
    {
        let mut sentence = true;
        for (i, t) in tokens.iter().enumerate().rev() {
            if let Token::ClauseBoundary(c) = t {
                sentence = ClauseTerminator::from_char(*c).is_sentence();
            }
            in_sentence.put(i, sentence);
        }
    }
    let mut seen_word_in_clause = false;
    // A symbol that is read by name is a word of the clause like any other, so
    // it both *takes* a clause position (`👍` alone is clause-final and keeps
    // "up" stressed; `a 👍 b` reduces it) and *occupies* one, which stops the
    // word before it from being clause-final (`a ÷` reads "a#", not "'eI").
    // Punctuation that is not spoken does neither.
    let word_like: Vec<bool> = tokens
        .iter()
        .map(|t| match t {
            Token::Operator(_) => true,
            Token::Punctuation(c) => {
                is_spoken_symbol(*c)
                    || punct_covers(options, *c)
                    || dict_names_symbol(dict, *c, options.dict_condition)
            }
            t => is_word(t),
        })
        .collect();
    // A bracket or quote after the last word is a 'word' of C's clause too,
    // and takes `FLAG_LAST_WORD` from it: `"go to"` reads `tU`, not `tu:`.
    // (Not what C makes a space of: an apostrophe, `_`, a spaced or double
    // dash.)
    let holds_end = |k: usize| match tokens.get(k) {
        Some(Token::Punctuation('-')) => {
            // Dashes pair up as pauses from the first; an odd one left over
            // is a word.
            let dash = |j: usize| matches!(tokens.get(j), Some(Token::Punctuation('-')));
            let first = (0..=k).rev().take_while(|&j| dash(j)).last().unwrap_or(k);
            let last = (k..tokens.len()).take_while(|&j| dash(j)).last().unwrap_or(k);
            let run = last - first + 1;
            run % 2 == 1
                && k == last
                && !(run == 1
                    && matches!(k.checked_sub(1).and_then(|j| tokens.get(j)), Some(Token::Space)))
        }
        Some(Token::Punctuation(c)) => !matches!(c, '\'' | '\u{2019}' | '_'),
        _ => false,
    };
    for (i, t) in tokens.iter().enumerate() {
        if matches!(t, Token::ClauseBoundary(_)) {
            seen_word_in_clause = false;
        } else if word_like.at(i) {
            at_start.put(i, !seen_word_in_clause);
            seen_word_in_clause = true;
            at_end[i] = !tokens
                .get(i + 1..)
                .unwrap_or_default()
                .iter()
                .enumerate()
                .take_while(|(_, t)| !matches!(t, Token::ClauseBoundary(_)))
                .any(|(k, _)| word_like.at(i + 1 + k) || holds_end(i + 1 + k));
        }
    }

    // A word read for another language's clause is at that clause's end only
    // if the clause ends there (C's `clause_end` is the main clause's): `a ☺`
    // in Hebrew reads the English "a" as `a#`, not the letter.
    if CLAUSE_CONTINUES.with(std::cell::Cell::get) || PHRASE_FOLLOWED.with(std::cell::Cell::get) {
        at_end.iter_mut().for_each(|e| *e = false);
    }

    // Per-clause letter-case counts, for C's abbreviation rule: "an upper case
    // word in a lower case clause could be an abbreviation".
    let (clause_lower, clause_upper) = {
        let mut lower = vec![0usize; tokens.len()];
        let mut upper = vec![0usize; tokens.len()];
        let mut start = 0usize;
        for i in 0..=tokens.len() {
            if i == tokens.len() || matches!(tokens.get(i), Some(Token::ClauseBoundary(_))) {
                let (mut l, mut u) = (0usize, 0usize);
                for t in tokens.get(start..i).unwrap_or_default() {
                    if let Token::Word(w) = t {
                        l += w.chars().filter(|c| c.is_lowercase()).count();
                        u += w.chars().filter(|c| c.is_uppercase()).count();
                    }
                }
                lower[start..i].fill(l);
                upper[start..i].fill(u);
                start = i + 1;
            }
        }
        (lower, upper)
    };

    // Words already spoken by an earlier multi-word dictionary entry.
    let mut skip_words = 0usize;
    // A dot already read as part of the number before it.
    let mut skip_dot: Option<usize> = None;
    // Dots a dotted abbreviation took in ("a . b").
    let mut skip_dots: Vec<usize> = Vec::new();
    let mut clause_count = 0usize;
    let mut clause_entries = 0usize;
    // What the words so far lead the next to be (C's `expect_verb` & co.).
    let mut expecting = Expecting::default();
    for (token_ix, token) in tokens.iter().enumerate() {
        match token {
            Token::ClauseBoundary(_) => expecting = Expecting::default(),
            Token::Number(_) => expecting.after_word(0),
            _ => {}
        }
        if skip_dot == Some(token_ix) || skip_dots.contains(&token_ix) {
            // (Entries stay one per token.)
            entries.push(TranslateEntry {
                phonemes: Vec::new(),
                dict_flags: 0,
                kind: TranslateEntryKind::Other,
                word_lower: None,
                no_word_gap: false,
                found_in_list: true,
            });
            continue;
        }
        match token {
            t if skip_words > 0 && (matches!(t, Token::Word(_)) || symbol_word(t)) => {
                skip_words -= 1;
                // Not a word of its own any more, so the clause-final stress
                // (`$u+`) goes to the entry that spoke it: C tests the *last*
                // word the entry took in for `FLAG_LAST_WORD`.
                entries.push(TranslateEntry {
                    phonemes: Vec::new(),
                    dict_flags: 0,
                    kind: TranslateEntryKind::Other,
                    word_lower: None,
                    no_word_gap: false,
                    found_in_list: true,
                });
            }
            // A letter another language spelled for this one (`_x`, in a
            // switched phrase): its name here, as C's `LookupLetter` gives it.
            Token::Word(word)
                if CLAUSE_REDUCE.with(std::cell::Cell::get).is_some()
                    && word.starts_with('_')
                    && word.chars().count() > 2 =>
            {
                let letters: String = word.chars().skip(1).collect();
                let phonemes = spell_word_letters(&letters, dict, phdata, stress_opts, options)
                    .map(|mut p| {
                        p.push(0);
                        p
                    })
                    .unwrap_or_default();
                entries.push(TranslateEntry {
                    phonemes,
                    dict_flags: 0,
                    kind: TranslateEntryKind::Word,
                    word_lower: Some(letters),
                    no_word_gap: false,
                    found_in_list: true,
                });
            }
            Token::Word(word)
                if CLAUSE_REDUCE.with(std::cell::Cell::get).is_some()
                    && word.starts_with('_')
                    && word.chars().count() == 2 =>
            {
                let letter: String = word.chars().skip(1).collect();
                let key = trim_lookup(&lookup_num_phonemes(dict, word)).to_vec();
                let phonemes = if key.is_empty() {
                    letter_name(&letter, false, dict, phdata, stress_opts, options)
                        .unwrap_or_default()
                } else {
                    let mut ph = key;
                    ph.push(0);
                    set_word_stress(&mut ph, phdata, stress_opts, Some(0), -1, 0);
                    ph
                };
                entries.push(TranslateEntry {
                    phonemes,
                    dict_flags: 0,
                    kind: TranslateEntryKind::Word,
                    word_lower: Some(letter),
                    no_word_gap: false,
                    found_in_list: true,
                });
            }
            Token::Word(word) if word == ":" || word == "-" => {
                // A colon between numbers, read by the rules with the numbers
                // round it (German `_L05) :` is "Uhr"), or not at all.
                // (English `D_) - (_D` is "dash", `_) - (_D` after a word a
                // pause, `__) - (_D` at the start "minus".)
                let number = |t: Option<&Token>| match t {
                    Some(Token::Number(n)) => n.surface().to_string(),
                    Some(Token::Word(w)) => lowercase_for_lang(w, &options.lang),
                    _ => String::new(),
                };
                let before = number(token_ix.checked_sub(1).and_then(|k| tokens.get(k)));
                let after = number(tokens.get(token_ix + 1));
                let mut buf = vec![b' '];
                buf.extend_from_slice(before.as_bytes());
                buf.push(b' ');
                let start = buf.len();
                buf.extend_from_slice(word.as_bytes());
                buf.push(b' ');
                buf.extend_from_slice(after.as_bytes());
                buf.extend_from_slice(b"  ");
                buf.push(0);
                let (mut vowels, mut stressed) = (0i32, 0i32);
                let r = crate::dictionary::rules::untraced(|| {
                    crate::dictionary::rules::translate_rules(
                        dict,
                        &buf,
                        start,
                        0,
                        0,
                        &dict.letter_bits,
                        options.dict_condition,
                        &mut vowels,
                        &mut stressed,
                    )
                });
                let mut phonemes: Vec<u8> = if r.spellword {
                    Vec::new()
                } else {
                    r.phonemes.into_iter().take_while(|&b| b != 0).collect()
                };
                // No rule: C's `LookupLetter` for running text, a plain entry
                // (Ukrainian ":" is "двокрапка"), not a `_:` one.
                if phonemes.is_empty() {
                    let ctx = LookupCtx {
                        lookup_symbol: true,
                        dict_condition: options.dict_condition,
                        ..Default::default()
                    };
                    if let Some(r) = lookup(dict, word, &ctx).filter(|r| {
                        r.flags1.found() && r.flags1.textmode() == options.reversed_textmode
                    }) {
                        phonemes = trim_lookup(&r.phonemes).to_vec();
                    }
                }
                if !phonemes.is_empty() {
                    phonemes.push(0);
                    set_word_stress(&mut phonemes, phdata, stress_opts, Some(0), -1, 0);
                }
                entries.push(TranslateEntry {
                    phonemes,
                    dict_flags: 0,
                    kind: TranslateEntryKind::Word,
                    word_lower: Some(word.clone()),
                    no_word_gap: false,
                    found_in_list: false,
                });
            }
            Token::Word(word) => {
                // A word in a script the voice can't read (Cyrillic/Greek inside a
                // Latin clause) switches to a voice for that script — the same
                // `phonSWITCH` re-translation used for `_^_LL` dict entries — so it
                // is spoken instead of dropped.
                // …unless this language's own dictionary knows the word: a
                // listed loanword or a pinyin reading belongs to the current
                // voice, and upstream likewise consults the dictionary before
                // the alphabet check in `TranslateRules`.
                // A word in a script this voice doesn't read, and that upstream
                // does *not* mark `AL_WORDS`, is **spelled** with the current
                // language's letter names rather than switching voice — but only
                // once the normal translation has had its turn, because some
                // letters are covered by the language's own rules (English reads
                // Greek letters directly, so `αβγ` is one word, while Cyrillic
                // falls through to spelling and comes out as separate words).
                // The dictionary's `.replace` table runs before anything else —
                // C applies it per character in `TranslateClause` — and it is
                // how a digraphic language reads its second script:
                // `mk "Dzidarski"` → `ѕидарски`, `lfn "Тота"` → `tota`,
                // `uz "Ўзбекча"` → `ózbekça`.  The script tests below have to
                // look at the *replaced* text, or the word is declared foreign
                // and spelled out letter by letter.
                // The table is keyed on lower case, as C's `SubstituteChar` is
                // (it folds the character before the lookup), so `Dzidarski`
                // transliterates as well as `dzidarski` does.
                let replaced = dict.apply_replacements(word);
                // Only the *script* tests below look at the replaced text; the
                // translation itself keeps the original, since
                // `word_to_phonemes` applies the table again (Icelandic's
                // `x → xz` would double up).
                let word_script = replaced.as_deref().unwrap_or(word);
                let foreign = word_script
                    .chars()
                    .find_map(char_script)
                    .filter(|sc| !native_scripts(&options.lang).contains(sc));
                if let Some(sc) = foreign {
                    // (A Latin word in a language with another alphabet is read
                    // in English: `TranslateRules` switches it.)
                    if !script_uses_words(sc)
                        && !(sc == Script::Latin && dict.letter_bits_offset > 0)
                        && alt_alphabet_voice(&options.lang, sc).is_none()
                        && !dict_knows_word(word, dict)
                        // …and only when this language's own rules can't read
                        // it: Macedonian and Uzbek have Latin transliterations,
                        // Uzbek and Lingua Franca Nova Cyrillic ones, so those
                        // words are translated, not spelled out.
                        && !rules_know_word(word_script, dict, options)
                    {
                        // C retranslates it as one word per letter
                        // (`FLAG_SPELLWORD`), each named if it can be and
                        // otherwise said by `TranslateLetter`.
                        let letters: Vec<char> = word
                            .chars()
                            .filter(|c| !matches!(*c as u32, 0x0300..=0x036F))
                            .collect();
                        let mut parts: Vec<LetterPart> = Vec::new();
                        for (k, c) in letters.into_iter().enumerate() {
                            if !parts.is_empty() {
                                parts.push(LetterPart::Break);
                            }
                            let lower = c.to_lowercase().to_string();
                            // A letter listed as a word is read as one (English
                            // `п`); one with only a name (`_ש`) is said by
                            // `TranslateLetter`, after its alphabet's name.
                            let name = lookup_num_phonemes(dict, &format!("_{lower}"));
                            if dict_knows_word(&lower, dict) {
                                let wr =
                                    word_to_phonemes(&lower, dict, phdata, stress_opts, options);
                                parts.push(LetterPart::Local(trim_lookup(&wr.phonemes).to_vec()));
                            } else if !trim_lookup(&name).is_empty() {
                                // (`LookupLetter` stresses all but the first.)
                                let mut name = trim_lookup(&name).to_vec();
                                name.push(0);
                                set_word_stress(
                                    &mut name,
                                    phdata,
                                    stress_opts,
                                    Some(0),
                                    -1,
                                    i32::from(k > 0),
                                );
                                parts.extend(alphabet_name_part(c, dict, &options.lang));
                                parts.push(LetterPart::Local(trim_lookup(&name).to_vec()));
                            } else {
                                parts.extend(letter_parts(c, dict, phdata, stress_opts, options));
                            }
                        }
                        entries.push(letter_entry_of(parts, word.to_lowercase()));
                        continue;
                    }
                }
                let switch_target = foreign_script_voice(word_script, &options.lang)
                    .or_else(|| foreign.map(script_voice));
                if let Some(target) = switch_target.filter(|_| {
                    !dict_knows_word(word, dict) && !rules_know_word(word_script, dict, options)
                }) {
                    let mut phonemes = vec![crate::phoneme::PHON_SWITCH];
                    phonemes.extend_from_slice(target.as_bytes());
                    // A letter on its own is introduced by its alphabet's name,
                    // in this language: English "د" is "Arabic (ar)dal".  The
                    // name follows the target, after a NUL.
                    let alphabet = alphabet_name(word, target, dict);
                    // No name for the alphabet here: the English one, then the
                    // letter, as C writes them (in this language's table).
                    let key = alphabet_key(word, target).filter(|_| alphabet.is_none());
                    if let (Some(key), Some(c)) =
                        (key, word.chars().next().filter(|_| options.lang != "en"))
                    {
                        let parts = vec![
                            LetterPart::Key {
                                lang: "en".to_string(),
                                key,
                            },
                            LetterPart::Raw {
                                lang: target.to_string(),
                                letter: c,
                            },
                        ];
                        entries.push(letter_entry_of(parts, word.to_lowercase()));
                        continue;
                    }
                    if let Some(name) = alphabet {
                        let key = format!("_{word}");
                        let letter = lookup_num_phonemes(dict, &key);
                        // (A script C reads as words is switched first, and named by
                        // the voice it switches to.)
                        // Arabic-script letters of other languages (Urdu "ڈ") are
                        // unknown to the Arabic voice, so they keep the name here.
                        let first = word.chars().next().unwrap_or(' ');
                        let spelled = char_script(first).is_some_and(|sc| {
                            !script_uses_words(sc)
                                || sc == Script::Arabic && !('\u{621}'..='\u{64a}').contains(&first)
                        });
                        if spelled && !letter.is_empty() {
                            let mut named = trim_lookup(&name).to_vec();
                            let letter =
                                letter_name(word, true, dict, phdata, stress_opts, options);
                            if let Some(letter) = letter {
                                named.extend_from_slice(trim_lookup(&letter));
                                entries.push(TranslateEntry {
                                    phonemes: named,
                                    dict_flags: 0,
                                    kind: TranslateEntryKind::Word,
                                    word_lower: Some(word.clone()),
                                    no_word_gap: false,
                                    found_in_list: true,
                                });
                                continue;
                            }
                        }
                        phonemes.push(0);
                        // (Said with the other language's word, in this one:
                        // `[phonPAUSE, phonSWITCH, …]`, `_:dZ'O@dZ@n(ka)'a(en)`.)
                        phonemes.push(crate::phoneme::PHON_PAUSE);
                        phonemes.extend_from_slice(trim_lookup(&name));
                    }
                    entries.push(TranslateEntry {
                        phonemes,
                        dict_flags: 0,
                        kind: TranslateEntryKind::Word, // retagged to LangSwitch below
                        word_lower: Some(word.clone()),
                        no_word_gap: false,
                        found_in_list: false,
                    });
                    continue;
                }
                let lower = lowercase_for_lang(word, &options.lang);
                let letters = word.chars().filter(|c| c.is_alphabetic());
                // The opening of the next word in the same clause, for the
                // post-contexts that reach across the boundary.
                let next_word = next_word_opening(tokens, token_ix + 1, &options.lang);
                let expect = PosExpect {
                    at_clause_end: at_end.at(token_ix),
                    at_end_after: {
                        let mut after = [false; 8];
                        let words = (token_ix + 1..tokens.len())
                            .take_while(|&k| !matches!(tokens.get(k), Some(Token::ClauseBoundary(_))))
                            .filter(|&k| word_like.at(k));
                        for (slot, k) in after.iter_mut().zip(words) {
                            *slot = at_end.at(k);
                        }
                        after
                    },
                    at_clause_start: at_start.at(token_ix),
                    in_sentence: in_sentence.at(token_ix),
                    first_upper: word.chars().next().is_some_and(char::is_uppercase),
                    all_upper: letters.clone().next().is_some()
                        && letters.clone().all(char::is_uppercase),
                    next_word,
                    hyphen_before: matches!(
                        token_ix.checked_sub(1).and_then(|k| tokens.get(k)),
                        Some(Token::WordJoin)
                    ),
                    hyphen_after: matches!(tokens.get(token_ix + 1), Some(Token::WordJoin)),
                    // (A dot that ends the clause too: C's
                    // `CLAUSE_DOT_AFTER_LAST_WORD`.)
                    has_dot: matches!(tokens.get(token_ix + 1), Some(Token::ClauseBoundary('.')))
                        || DOTS.with(|d| d.borrow().contains(&(token_ix + 1))),
                    following: following_words(tokens, token_ix + 1, &options.lang),
                    prev_word: previous_word_end(tokens, token_ix, &options.lang),
                    verb: expecting.verb > 0,
                    verb_s: expecting.verb_s > 0,
                    noun: expecting.noun > 0,
                    past: expecting.past > 0,
                    ..PosExpect::default()
                };
                WORD_FLAGS2.with(|f| f.set(0));
                let mut wr =
                    word_to_phonemes_pos(&lower, dict, phdata, stress_opts, options, expect);
                expecting.after_word(WORD_FLAGS2.with(std::cell::Cell::get));
                // `$dot`/`$hasdot`: C removes a dot after the word even with a
                // space before it (Portuguese "mrs . hawking").
                {
                    const FLAG_ALLOW_DOT: u32 = 0x0100_0000;
                    if wr.dict_flags & (FLAG_ALLOW_DOT | crate::dictionary::FLAG_NEEDS_DOT) != 0
                        && matches!(tokens.get(token_ix + 1), Some(Token::Space))
                        && matches!(tokens.get(token_ix + 2), Some(Token::Punctuation('.')))
                    {
                        skip_dots.push(token_ix + 2);
                    }
                }
                // Single letters joined by dots ("e.g.", "u.s.a."): C's clause
                // buffer has them as `e . g .`, which a dictionary entry
                // `(e . g .)` can match, and otherwise `CheckDottedAbbrev`
                // spells them as one word.
                if is_single_letter_word(word) && wr.skipwords == 0 {
                    let mut chain = vec![lower.clone()];
                    let mut dots: Vec<usize> = Vec::new();
                    let mut k = token_ix + 1;
                    let single = |t: Option<&Token>| matches!(t, Some(Token::Word(w)) if is_single_letter_word(w));
                    loop {
                        if single(tokens.get(k)) {
                            if let Some(Token::Word(w)) = tokens.get(k) {
                                chain.push(lowercase_for_lang(w, &options.lang));
                            }
                            k += 1;
                            continue;
                        }
                        // (Or with spaces round the dot: "a . b".)
                        let mut j = k;
                        if matches!(tokens.get(j), Some(Token::Space)) {
                            j += 1;
                        }
                        if !matches!(tokens.get(j), Some(Token::Punctuation('.'))) {
                            break;
                        }
                        let dot = j;
                        j += 1;
                        if matches!(tokens.get(j), Some(Token::Space)) {
                            j += 1;
                        }
                        if !single(tokens.get(j)) {
                            break;
                        }
                        if let Some(Token::Word(w)) = tokens.get(j) {
                            chain.push(lowercase_for_lang(w, &options.lang));
                        }
                        dots.push(dot);
                        k = j + 1;
                    }
                    let trailing_dot = DOTS.with(|d| d.borrow().contains(&k));
                    if chain.len() > 1 {
                        let mut text = String::new();
                        for l in chain.iter().skip(1) {
                            text.push_str(". ");
                            text.push_str(l);
                            text.push(' ');
                        }
                        if trailing_dot {
                            text.push_str(". ");
                        }
                        let rest = following_words(tokens, k, &options.lang);
                        let rest_len = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
                        let mut bytes = text.into_bytes();
                        bytes.extend_from_slice(rest.get(..rest_len).unwrap_or_default());
                        let mut following = [0u8; 32];
                        let n = bytes.len().min(32);
                        following[..n].copy_from_slice(bytes.get(..n).unwrap_or_default());
                        let dotted = word_to_phonemes_pos(
                            &lower,
                            dict,
                            phdata,
                            stress_opts,
                            options,
                            PosExpect {
                                following,
                                ..expect
                            },
                        );
                        let words: Vec<&[u8]> = bytes
                            .split(|&b| b == b' ')
                            .filter(|w| !w.is_empty())
                            .take(dotted.skipwords)
                            .collect();
                        if dotted.found_in_list && dotted.skipwords > 0 {
                            skip_words = words.iter().filter(|w| **w != b".").count();
                            entries.push(TranslateEntry {
                                phonemes: dotted.phonemes,
                                dict_flags: dotted.dict_flags,
                                kind: TranslateEntryKind::Word,
                                word_lower: Some(lower.clone()),
                                no_word_gap: false,
                                found_in_list: true,
                            });
                            continue;
                        }
                        // With no dot left after it, C looks the series up
                        // as one word too: Czech `e.g` is "například".
                        if !trailing_dot {
                            let word = chain.join(".");
                            let listed =
                                word_to_phonemes_pos(&word, dict, phdata, stress_opts, options, expect);
                            if listed.found_in_list && !listed.phonemes.is_empty() {
                                skip_words = chain.len() - 1;
                                skip_dots.extend(dots.iter().copied());
                                entries.push(TranslateEntry {
                                    phonemes: listed.phonemes,
                                    dict_flags: listed.dict_flags,
                                    kind: TranslateEntryKind::Word,
                                    word_lower: Some(word),
                                    no_word_gap: false,
                                    found_in_list: true,
                                });
                                continue;
                            }
                        }
                        // A dot kept after the last letter (a glued one's, which
                        // C leaves in the clause) lets the series take in a
                        // single letter after it: German `a.b. c` is one word.
                        if trailing_dot {
                            let mut j = k + 1;
                            while matches!(tokens.get(j), Some(Token::Space)) {
                                j += 1;
                            }
                            if let Some(Token::Word(w)) = tokens.get(j) {
                                if is_single_letter_word(w)
                                    && !matches!(tokens.get(j + 1), Some(Token::Word(_) | Token::Number(_) | Token::Punctuation('.')))
                                {
                                    chain.push(lowercase_for_lang(w, &options.lang));
                                }
                            }
                        }
                        let joined: String = chain.concat();
                        if let Some(phonemes) =
                            spell_word_letters(&joined, dict, phdata, stress_opts, options)
                        {
                            skip_words = chain.len() - 1;
                            skip_dots.extend(dots.iter().copied());
                            entries.push(TranslateEntry {
                                phonemes,
                                dict_flags: 0,
                                kind: TranslateEntryKind::Word,
                                word_lower: Some(joined),
                                no_word_gap: false,
                                found_in_list: false,
                            });
                            continue;
                        }
                    }
                }
                // C's `TranslateRoman`, for a word the list doesn't have.
                if !wr.found_in_list && wr.dict_flags & crate::dictionary::FLAG_TEXTMODE == 0 {
                    let prev_digit = matches!(
                        (
                            token_ix.checked_sub(1).and_then(|k| tokens.get(k)),
                            token_ix.checked_sub(2).and_then(|k| tokens.get(k))
                        ),
                        (Some(Token::Space), Some(Token::Number(_)))
                    );
                    let next = tokens.get(token_ix + 1);
                    let mid_dot = DOTS.with(|d| d.borrow().contains(&(token_ix + 1)));
                    let has_dot = mid_dot || matches!(next, Some(Token::ClauseBoundary('.')));
                    let glued = matches!(next, Some(Token::Word(_) | Token::Number(_)));
                    let next_digit = matches!(
                        (next, tokens.get(token_ix + 2)),
                        (Some(Token::Space), Some(Token::Number(_)))
                    );
                    if !glued && !next_digit {
                        if let Some(phonemes) = translate_roman(
                            word,
                            prev_digit,
                            has_dot,
                            mid_dot,
                            dict,
                            phdata,
                            stress_opts,
                            options,
                        ) {
                            // (Read as found: none of the spelling below.)
                            entries.push(TranslateEntry {
                                phonemes,
                                dict_flags: crate::dictionary::FLAG_ABBREV,
                                kind: TranslateEntryKind::Word,
                                word_lower: Some(lower.clone()),
                                no_word_gap: false,
                                found_in_list: false,
                            });
                            continue;
                        }
                    }
                }
                if let Some(stale) = STALE_TABLE.with(|t| t.borrow().clone()) {
                    let flags = wr.dict_flags;
                    // (A listed word goes in as the list gives it, explicit
                    // marks and all: Spanish `y %i`.)
                    if wr.found_in_list {
                        let raw = trim_lookup(&lookup_num_phonemes(dict, &lower)).to_vec();
                        if !raw.is_empty() {
                            wr.phonemes = raw;
                            wr.phonemes.push(0);
                        }
                    }
                    restress_with_stale_table(
                        &mut wr.phonemes,
                        &stale,
                        flags,
                        stress_opts,
                        options,
                    );
                }
                // A character this language can't read or name (a Hakka
                // ideograph, Shan's visarga): C's `TranslateLetter` fallback.
                let unread = !wr
                    .phonemes
                    .iter()
                    .any(|&b| b > 8 && b != crate::phoneme::PHON_END_WORD);
                let single = lower.chars().next().filter(|_| lower.chars().count() == 1);
                // (Not one the dictionary gives as text that reads as nothing:
                // C found it, so says nothing — Japanese `☃`.)
                let found_text = wr.dict_flags & crate::dictionary::FLAG_TEXTMODE != 0;
                if let Some(c) = single.filter(|&c| unread && !found_text && !c.is_whitespace()) {
                    entries.push(letter_entry_of(
                        letter_parts(c, dict, phdata, stress_opts, options),
                        lower.clone(),
                    ));
                    continue;
                }
                // A letter the rules have nothing for (C's `FLAG_SPELLWORD`): the
                // word is read letter by letter, each letter a word of its own.
                // (Not a word the dictionary replaces by another, Tamil "tamil".)
                // (C's `Unpronouncable` test comes before the rules: runic
                // `ᚠᚡ` is spelled as one word, below.)
                if !wr.found_in_list
                    && wr.dict_flags & crate::dictionary::FLAG_TEXTMODE == 0
                    && !is_unpronounceable(&lower, dict, options)
                    && rules_cannot_read(&lower, dict, options)
                {
                    // Each letter is a word, and one nothing reads is said by
                    // `TranslateLetter` (Shan `းမူၺ` → "Myanmar letter 1038 m u J").
                    let mut parts: Vec<LetterPart> = Vec::new();
                    for c in lower.chars() {
                        let ph = spell_letter_words(
                            &c.to_string(),
                            dict,
                            phdata,
                            stress_opts,
                            options,
                            PosExpect {
                                next_word: [0; 8],
                                following: [0; 32],
                                ..expect
                            },
                        );
                        if !parts.is_empty() {
                            parts.push(LetterPart::Break);
                        }
                        let ph = if trim_lookup(&ph).is_empty() {
                            letter_name(&c.to_string(), false, dict, phdata, stress_opts, options)
                                .unwrap_or_default()
                        } else {
                            ph
                        };
                        if trim_lookup(&ph).is_empty() {
                            parts.extend(letter_parts(c, dict, phdata, stress_opts, options));
                        } else {
                            parts.push(LetterPart::Local(trim_lookup(&ph).to_vec()));
                        }
                    }
                    if parts.iter().any(|p| !matches!(p, LetterPart::Break)) {
                        entries.push(letter_entry_of(parts, lower.clone()));
                        continue;
                    }
                }
                // C spells a word whose list entry carries `$abbrev` but gives no
                // pronunciation ("the word has $abbrev flag, but no pronunciation
                // specified.  Speak as individual letters") — English `usa` is
                // `j,u:,Es'eI`.
                if !wr.found_in_list && wr.dict_flags & crate::dictionary::FLAG_ABBREV != 0 {
                    if let Some(spelled) =
                        spell_word_letters(&lower, dict, phdata, stress_opts, options)
                    {
                        wr.phonemes = spelled;
                    }
                }
                // A word the dictionary doesn't pronounce and whose first vowel
                // is too far in — "BBC" has none at all — is spelled letter by
                // letter, then whatever remains is translated normally.
                else if !wr.found_in_list
                    && wr.dict_flags & crate::dictionary::FLAG_TEXTMODE == 0
                    && is_unpronounceable(&lower, dict, options)
                {
                    if let Some((letters, rest)) =
                        spell_unpronounceable(&lower, dict, phdata, stress_opts, options)
                    {
                        let mut ph = letters;
                        if !rest.is_empty() {
                            // The remainder is still part of the same word, so
                            // the letters already spelled stay available as a
                            // rule pre-context (`s's` → `'Es` + `s) 's` → `I2z`,
                            // not the bare `'s` → `z`).
                            let spelled = &lower[..lower.len() - rest.len()];
                            let mut prev_letters = [0u8; 8];
                            let b = spelled.as_bytes();
                            let keep = b.len().min(prev_letters.len());
                            let from = (0..=keep)
                                .rev()
                                .find(|&k| spelled.is_char_boundary(b.len() - k))
                                .unwrap_or(0);
                            prev_letters[..from]
                                .copy_from_slice(b.get(b.len() - from..).unwrap_or_default());
                            let tail = word_to_phonemes_pos(
                                rest,
                                dict,
                                phdata,
                                stress_opts,
                                options,
                                PosExpect {
                                    prev_letters,
                                    ..PosExpect::default()
                                },
                            );
                            ph.extend(tail.phonemes.iter().copied().filter(|&b| b != 0));
                        }
                        wr.phonemes = ph;
                    }
                }
                // …and a *short* all-capitals word the dictionary doesn't
                // pronounce, in a predominantly lower-case clause, is read as an
                // abbreviation too.
                else if expect.all_upper
                    && !wr.found_in_list
                    && wr.dict_flags & crate::dictionary::FLAG_TEXTMODE == 0
                    && (2..4).contains(&word.chars().count())
                    && word.chars().next().is_some_and(char::is_alphabetic)
                    && clause_lower.at(token_ix) > 3
                    && clause_upper.at(token_ix) <= clause_lower.at(token_ix)
                {
                    if let Some(spelled) =
                        spell_word_letters(&lower, dict, phdata, stress_opts, options)
                    {
                        wr.phonemes = spelled;
                        wr.found_in_list = false;
                    }
                }
                skip_words = wr.skipwords;
                // `$combine`: a preposition and the word after it are one word.
                if skip_words == 0 && wr.dict_flags & FLAG_COMBINE != 0 {
                    if let Some(combined) =
                        combine_with_next(&wr, tokens, token_ix, dict, phdata, stress_opts, options)
                    {
                        wr = combined;
                        skip_words = 1;
                    }
                }
                // Word-final inherent-vowel (schwa) deletion for the Indo-Aryan
                // scripts (Devanagari, Bengali, Gujarati, Gurmukhi).  Dravidian
                // (ta/te/kn/ml) and Oriya keep the inherent vowel, so they're
                // excluded; `delete_final_schwa` is also a no-op where the
                // inherent-vowel phoneme isn't `V`.
                if matches!(
                    primary_bcp47_subtag(&options.lang),
                    "hi" | "mr" | "ne" | "sa" | "kok" | "bn" | "as" | "gu" | "pa"
                ) {
                    delete_final_schwa(&mut wr.phonemes, phdata);
                }
                // "remove initial pause from a word if it follows a hyphen"
                if matches!(
                    token_ix.checked_sub(1).and_then(|k| tokens.get(k)),
                    Some(Token::WordJoin)
                ) && wr.phonemes.first().is_some_and(|&b| {
                    b != crate::phoneme::PHON_SWITCH
                        && phdata
                            .get(b)
                            .is_some_and(|ph| ph.typ == crate::phoneme::PH_PAUSE)
                }) {
                    wr.phonemes.remove(0);
                }
                entries.push(TranslateEntry {
                    phonemes: wr.phonemes,
                    dict_flags: wr.dict_flags,
                    found_in_list: wr.found_in_list,
                    kind: TranslateEntryKind::Word,
                    word_lower: Some(lower),
                    no_word_gap: false,
                });
            }
            Token::Number(token) => {
                // C's `TranslateWord3` looks a number up in the list before
                // reading it, with the words after it: French `(1 er)`.
                if let NumberToken::Cardinal(digits) = token {
                    let expect = PosExpect {
                        following: following_words(tokens, token_ix + 1, &options.lang),
                        ..Default::default()
                    };
                    let wr =
                        word_to_phonemes_pos(digits, dict, phdata, stress_opts, options, expect);
                    if wr.found_in_list && wr.skipwords > 0 {
                        skip_words = wr.skipwords;
                        entries.push(TranslateEntry {
                            phonemes: wr.phonemes,
                            dict_flags: wr.dict_flags,
                            found_in_list: true,
                            kind: TranslateEntryKind::Word,
                            word_lower: Some(digits.clone()),
                            no_word_gap: false,
                        });
                        continue;
                    }
                }
                // Portuguese numbers: the pt dict lacks number keys, so build the
                // number word(s) from the value and pronounce them via the rules.
                if options.number_grammar.portuguese_cardinals {
                    if let Some(word) = portuguese_number_word(token) {
                        let mut phonemes: Vec<u8> = Vec::new();
                        // The composed entry inherits the *last* word's dict
                        // flags: clause-final stress promotion keys off
                        // `FLAG_STREND`, so dropping them left "101" as
                        // `sˈẽŋtʊ i ũŋ` where the same words typed out give
                        // `… ˈũŋ` (and upstream stresses both).
                        let mut last_flags = 0;
                        let parts: Vec<&str> = word.split_whitespace().collect();
                        for (pi, part) in parts.iter().enumerate() {
                            let wr = word_to_phonemes_pos(
                                part,
                                dict,
                                phdata,
                                stress_opts,
                                options,
                                expect_before(parts.get(pi + 1).copied()),
                            );
                            let ph = wr.phonemes.strip_suffix(&[0]).unwrap_or(&wr.phonemes);
                            if ph.is_empty() {
                                continue;
                            }
                            if !phonemes.is_empty() {
                                phonemes.push(crate::phoneme::PHON_END_WORD);
                            }
                            phonemes.extend_from_slice(ph);
                            last_flags = wr.dict_flags;
                        }
                        entries.push(TranslateEntry {
                            phonemes,
                            dict_flags: last_flags,
                            kind: TranslateEntryKind::Word,
                            word_lower: Some(word),
                            no_word_gap: false,
                            found_in_list: false,
                        });
                        continue;
                    }
                }
                // Italian irregular ordinals 1–10 (1º=primo … 10º=decimo): the it
                // dict lacks them, so the regular `-esimo` fallback produces wrong
                // words ("unesimo"); generate the correct form (11+ is left to the
                // regular path, which is right).
                // (A language with C's number tables reads them as C does.)
                if options.number_grammar.ordinals.italian && options.number_grammar.flags.is_none()
                {
                    if let NumberToken::Ordinal(ord) = token {
                        if let OrdinalMarker::Suffix(suffix) = &ord.marker {
                            if let Some(word) = italian_ordinal_word(&ord.digits, suffix) {
                                let wr =
                                    word_to_phonemes(&word, dict, phdata, stress_opts, options);
                                entries.push(TranslateEntry {
                                    phonemes: wr.phonemes,
                                    dict_flags: wr.dict_flags,
                                    found_in_list: wr.found_in_list,
                                    kind: TranslateEntryKind::Word,
                                    word_lower: Some(word),
                                    no_word_gap: false,
                                });
                                continue;
                            }
                        }
                    }
                }
                // French letter-suffix ordinals (1er, 2e, 21e…): the fr dict has
                // no `_No` ordinal words, so build the ordinal word from the value
                // and let the French rules pronounce it ("premier"/"deuxième").
                if options.number_grammar.ordinals.french && options.number_grammar.flags.is_none()
                {
                    if let NumberToken::Ordinal(ord) = token {
                        if let OrdinalMarker::Suffix(suffix) = &ord.marker {
                            if let Some(word) = french_ordinal_word(&ord.digits, suffix) {
                                // The word may be several tokens ("vingt et
                                // unième", "deux cent trente-quatrième").  Split
                                // on spaces *and* hyphens — the tokenizer treats a
                                // hyphen between letters as a word break, so
                                // "quatre-vingtième" must render the same whether
                                // generated here or typed.  Join into ONE entry
                                // with word-boundary markers.
                                let mut phonemes: Vec<u8> = Vec::new();
                                let parts: Vec<&str> = word
                                    .split(|c: char| c.is_whitespace() || c == '-')
                                    .filter(|p| !p.is_empty())
                                    .collect();
                                for (pi, part) in parts.iter().enumerate() {
                                    let wr = word_to_phonemes_pos(
                                        part,
                                        dict,
                                        phdata,
                                        stress_opts,
                                        options,
                                        expect_before(parts.get(pi + 1).copied()),
                                    );
                                    let ph = wr.phonemes.strip_suffix(&[0]).unwrap_or(&wr.phonemes);
                                    if ph.is_empty() {
                                        continue;
                                    }
                                    if !phonemes.is_empty() {
                                        phonemes.push(crate::phoneme::PHON_END_WORD);
                                    }
                                    phonemes.extend_from_slice(ph);
                                }
                                entries.push(TranslateEntry {
                                    phonemes,
                                    dict_flags: 0,
                                    kind: TranslateEntryKind::Word,
                                    word_lower: Some(word),
                                    no_word_gap: false,
                                    found_in_list: false,
                                });
                                continue;
                            }
                            // Beyond the generator's range (≥1001): the fr dict has
                            // no ordinal words, so the ordinal path is silent — read
                            // the plain cardinal so the number isn't dropped.
                            let card = NumberToken::Cardinal(ord.digits.clone());
                            if let Some(wr) = translate_number_token(
                                &card,
                                dict,
                                phdata,
                                stress_opts,
                                &options.number_grammar,
                            ) {
                                entries.push(TranslateEntry {
                                    phonemes: wr.phonemes,
                                    dict_flags: wr.dict_flags,
                                    found_in_list: wr.found_in_list,
                                    kind: TranslateEntryKind::Word,
                                    word_lower: Some(ord.digits.clone()),
                                    no_word_gap: false,
                                });
                                continue;
                            }
                        }
                    }
                }
                let wr = translate_number_token(
                    token,
                    dict,
                    phdata,
                    stress_opts,
                    &options.number_grammar,
                )
                .unwrap_or_else(|| {
                    let surface = token.surface();
                    word_to_phonemes(&surface, dict, phdata, stress_opts, options)
                });
                let mut wr = wr;
                // (Read against a table C has left selected: Portuguese
                // `©2024` says the year in schwas.)
                if let Some(stale) = STALE_TABLE.with(|t| t.borrow().clone()) {
                    if let Some(t) = table_by_name(&stale) {
                        let mut ph: Vec<u8> = wr
                            .phonemes
                            .iter()
                            .copied()
                            .take_while(|&b| b != 0)
                            .map(|b| {
                                if b > 8 && b != PHON_END_WORD && t.get(b).is_none() {
                                    13
                                } else {
                                    b
                                }
                            })
                            .collect();
                        drop_vowelless_stress_marks(&mut ph, &t);
                        ph.push(0);
                        wr.phonemes = ph;
                    }
                }
                // C's `TranslateNumber_1`: a dot after the digits that isn't
                // their decimal point is the number's `_.`, in its word.
                if matches!(tokens.get(token_ix + 1), Some(Token::Punctuation('.'))) {
                    let dot = lookup_num_phonemes(dict, "_.");
                    // (With no `_.` the dot says nothing: Portuguese `3.5`.)
                    skip_dot = Some(token_ix + 1);
                    if !trim_lookup(&dot).is_empty() {
                        // (Stressed together with the number's own phonemes.)
                        let raw = match token {
                            NumberToken::Cardinal(d) if d.len() <= 4 => {
                                number_token_to_phonemes(token, dict, &options.number_grammar)
                            }
                            _ => None,
                        };
                        let mut ph: Vec<u8> =
                            trim_lookup(raw.as_deref().unwrap_or(&wr.phonemes)).to_vec();
                        while ph.last() == Some(&PHON_END_WORD) {
                            ph.pop();
                        }
                        ph.push(PHON_END_WORD);
                        ph.extend_from_slice(trim_lookup(&dot));
                        ph.push(0);
                        set_word_stress(&mut ph, phdata, stress_opts, Some(0), -1, 0);
                        wr.phonemes = ph;
                        skip_dot = Some(token_ix + 1);
                    }
                }
                entries.push(TranslateEntry {
                    phonemes: wr.phonemes,
                    dict_flags: wr.dict_flags,
                    found_in_list: wr.found_in_list,
                    kind: TranslateEntryKind::Word,
                    word_lower: Some(token.surface().to_string().to_lowercase()),
                    no_word_gap: false,
                });
            }
            Token::InlinePhonemes(content) => {
                entries.push(TranslateEntry {
                    phonemes: parse_inline_phonemes(content, phdata),
                    dict_flags: 0,
                    kind: TranslateEntryKind::Word,
                    word_lower: None,
                    no_word_gap: false,
                    found_in_list: false,
                });
            }
            Token::ClauseBoundary(c) => {
                // "probably an abbreviation such as Mr. or B. rather than end of
                // sentence": no intonation for this clause.
                const DOT_FLAGS: u32 = 0x0100_0000 | 0x0200_0000;
                // (Within this clause only.)
                let last = entries
                    .get(clause_entries..)
                    .unwrap_or_default()
                    .iter()
                    .rev()
                    .find(|e| {
                        matches!(
                            e.kind,
                            TranslateEntryKind::Word | TranslateEntryKind::LangSwitch
                        )
                    });
                // (A word read in English has English's flags.)
                let english_dot = |w: &str| {
                    EN_DICT
                        .with(|d| {
                            d.get_or_init(|| {
                                let dir = DATA_DIR.with(|d| d.borrow().clone())?;
                                Dictionary::load("en", &dir).ok().map(std::rc::Rc::new)
                            })
                            .clone()
                        })
                        .and_then(|en| {
                            let ctx = LookupCtx {
                                lookup_symbol: true,
                                word_flags: crate::dictionary::FLAG_HAS_DOT,
                                ..Default::default()
                            };
                            lookup(&en, w, &ctx)
                        })
                        .is_some_and(|r| r.flags1.0 & DOT_FLAGS != 0)
                };
                let abbreviation = last.is_some_and(|e| match e.kind {
                    TranslateEntryKind::LangSwitch => {
                        switch_target(&e.phonemes) == "en"
                            && e.word_lower.as_deref().is_some_and(english_dot)
                    }
                    _ => e.dict_flags & DOT_FLAGS != 0,
                });
                if *c == '.' && abbreviation {
                    NO_TONE_CLAUSES.with(|n| n.borrow_mut().push(clause_count));
                }
                clause_count += 1;
                clause_entries = entries.len();
                STALE_TABLE.with(|t| *t.borrow_mut() = None);
                // A clause boundary resets the part-of-speech expectation.
                // `--punct`: announce clause punctuation by name instead of a
                // silent break.  Falls back to a break if the char has no name.
                let (name, name_from_dict) = if punct_covers(options, *c) {
                    lookup_symbol_name(
                        *c,
                        dict,
                        phdata,
                        stress_opts,
                        options,
                        SymbolName::Announce,
                        PosExpect {
                            at_clause_end: at_end.at(token_ix),
                            at_clause_start: at_start.at(token_ix),
                            ..PosExpect::default()
                        },
                    )
                } else {
                    (Vec::new(), false)
                };
                if name.is_empty() {
                    entries.push(TranslateEntry {
                        phonemes: Vec::new(),
                        dict_flags: 0,
                        kind: TranslateEntryKind::ClauseBoundary,
                        word_lower: None,
                        no_word_gap: false,
                        found_in_list: false,
                    });
                } else {
                    entries.push(TranslateEntry {
                        phonemes: name,
                        dict_flags: 0,
                        kind: TranslateEntryKind::Word,
                        word_lower: None,
                        no_word_gap: false,
                        // A dictionary pronunciation is a *translation given*
                        // (C's `SFLAG_DICTIONARY`), which suppresses the
                        // stress-conditioned phoneme changes; a name that came
                        // from replacement text was read by the rules and does
                        // take them.
                        found_in_list: name_from_dict,
                    });
                }
            }
            // A sign glued to the symbol after it is one word for C, read by
            // the rules: English "+$5" is `plˈʌsdɒlə`, German "-€10" says
            // nothing for "-€".
            Token::Punctuation(c @ ('-' | '+'))
                if matches!(tokens.get(token_ix + 1), Some(Token::Punctuation(n)) if symbol_word(&Token::Punctuation(*n)) && *n != '-' && *n != '+')
                    && matches!(
                        token_ix.checked_sub(1).and_then(|k| tokens.get(k)),
                        None | Some(Token::Space | Token::ClauseBoundary(_))
                    ) =>
            {
                let Some(Token::Punctuation(n)) = tokens.get(token_ix + 1) else {
                    continue;
                };
                let word = format!("{c}{n}");
                let (phonemes, kind) =
                    match glued_symbols(&word, dict, phdata, stress_opts, options) {
                        Some(ph) => (ph, TranslateEntryKind::Word),
                        None => {
                            let mut ph = vec![crate::phoneme::PHON_SWITCH];
                            ph.extend_from_slice(b"en");
                            (ph, TranslateEntryKind::LangSwitch)
                        }
                    };
                entries.push(TranslateEntry {
                    phonemes,
                    dict_flags: 0,
                    kind,
                    word_lower: Some(word),
                    no_word_gap: false,
                    found_in_list: false,
                });
                skip_dot = Some(token_ix + 1);
            }
            // A dash with spaces round it is a pause for C, not a word.
            Token::Punctuation('-')
                if matches!(
                    token_ix.checked_sub(1).and_then(|k| tokens.get(k)),
                    None | Some(Token::Space | Token::ClauseBoundary(_))
                ) && matches!(tokens.get(token_ix + 1), Some(Token::Space)) =>
            {
                entries.push(TranslateEntry {
                    phonemes: Vec::new(),
                    dict_flags: 0,
                    kind: TranslateEntryKind::Other,
                    word_lower: None,
                    no_word_gap: false,
                    found_in_list: false,
                });
            }
            Token::Operator(c) | Token::Punctuation(c)
                if matches!(token, Token::Operator(_))
                    || is_spoken_symbol(*c)
                    // (Clause punctuation that ended nothing, C's `any_alnum`:
                    // a word read as running text, English `:` "colon".)
                    || matches!(token, Token::Punctuation(_)) && is_clause_punctuation(*c)
                    || punct_covers(options, *c)
                    || dict_names_symbol(dict, *c, options.dict_condition) =>
            {
                // Technical symbols read as their name by default (# $ % & * + / = @ ~),
                // as does any character the dictionary names (see `dict_names_symbol`).
                let mode = if matches!(token, Token::Operator(_)) {
                    SymbolName::Operator
                } else if punct_covers(options, *c) && !is_spoken_symbol(*c) {
                    SymbolName::Announce
                } else {
                    SymbolName::Inline
                };
                // `$max3`: a symbol repeated more than three times in a row is
                // dropped after the third (C's `phonemes_repeat_count`), so
                // `~ ~ ~ ~ ~ ~` is three tildes.  Letters are *not* limited —
                // C limits them too, which upstream #2551 reports as a bug and
                // #2552 fixes; this port already reads all the repeats.
                let max3 = dict_symbol_flags(dict, *c, options.dict_condition)
                    .is_some_and(|f| f & crate::dictionary::FLAG_MAX3 != 0);
                BRACKET_IN_REPLACEMENT.with(|b| b.set(false));
                SYMBOL_SWITCHED_BY_RULES.with(|b| b.set(false));
                let (phonemes, symbol_from_dict) = lookup_symbol_name(
                    *c,
                    dict,
                    phdata,
                    stress_opts,
                    options,
                    mode,
                    PosExpect {
                        at_clause_end: at_end.at(token_ix),
                        at_clause_start: at_start.at(token_ix),
                        next_word: next_word_opening(tokens, token_ix + 1, &options.lang),
                        // (The number before it, which a rule may want:
                        // Romanian `D_) %` is "la sută".)
                        prev_word: {
                            let mut out = [0u8; 8];
                            let back =
                                |k: usize| token_ix.checked_sub(k).and_then(|j| tokens.get(j));
                            let number = match (back(1), back(2)) {
                                (Some(Token::Number(n)), _)
                                | (Some(Token::Space), Some(Token::Number(n))) => {
                                    Some(n.surface().to_string())
                                }
                                _ => None,
                            };
                            if let Some(n) = number {
                                let b = n.as_bytes();
                                let keep = b.len().min(7);
                                out[..keep]
                                    .copy_from_slice(b.get(b.len() - keep..).unwrap_or_default());
                            }
                            out
                        },
                        ..PosExpect::default()
                    },
                );
                let phonemes = if max3 && repeat_of_symbol == Some(*c) {
                    repeat_count += 1;
                    if repeat_count > 3 {
                        Vec::new()
                    } else {
                        phonemes
                    }
                } else {
                    repeat_of_symbol = max3.then_some(*c);
                    repeat_count = 1;
                    phonemes
                };
                if phonemes.is_empty() {
                    entries.push(TranslateEntry {
                        phonemes,
                        dict_flags: 0,
                        kind: TranslateEntryKind::Other,
                        word_lower: None,
                        no_word_gap: false,
                        found_in_list: false,
                    });
                } else {
                    // A bracket in its reading pauses before the next word
                    // (C's `pre_pause`, two `phonPAUSE`), when there is one.
                    let mut phonemes = phonemes;
                    let word_follows = tokens
                        .get(token_ix + 1..)
                        .unwrap_or_default()
                        .iter()
                        .take_while(|t| !matches!(t, Token::ClauseBoundary(_)))
                        .any(|t| matches!(t, Token::Word(_) | Token::Number(_)));
                    if BRACKET_IN_REPLACEMENT.with(|b| b.replace(false)) && word_follows {
                        let end = trim_lookup(&phonemes).len();
                        phonemes.truncate(end);
                        phonemes.extend([
                            crate::phoneme::PHON_PAUSE,
                            crate::phoneme::PHON_PAUSE,
                            0,
                        ]);
                    }
                    entries.push(TranslateEntry {
                        phonemes,
                        // (The entry's own flags: French `& e $u $brk` pauses
                        // before it.)
                        dict_flags: dict_symbol_flags(dict, *c, options.dict_condition)
                            .unwrap_or(0),
                        kind: TranslateEntryKind::Word,
                        // What a language switch in these phonemes looks up in
                        // the target language.  A switch that came from the
                        // dictionary *entry* re-translates the character itself
                        // (`fr ©` is a bare `phonSWITCH`, and English's own `©`
                        // says "copyright"); a switch from the character-name
                        // fallback is C's `LookupCharName`, which reads the
                        // `_<char>` entry there (`he @` → English `_@` → "at
                        // sign").  Without either the marker leaked out as `_^_`.
                        word_lower: Some(
                            if dict_names_symbol(dict, *c, options.dict_condition)
                                && !SYMBOL_SWITCHED_BY_RULES.with(|b| b.replace(false))
                            {
                                c.to_string()
                            } else {
                                format!("_{c}")
                            },
                        ),
                        no_word_gap: false,
                        // Phonemes from the dictionary are a *translation
                        // given*, so the stress-conditioned changes don't apply
                        // (`pl @` is `m'awpa` where the typed word "małpa",
                        // read by the rules, is `m'awpa#`); replacement text is
                        // re-translated and does take them (`tr −` → `Eks'I`).
                        found_in_list: symbol_from_dict,
                    });
                }
            }
            _ => {
                entries.push(TranslateEntry {
                    phonemes: Vec::new(),
                    dict_flags: 0,
                    kind: TranslateEntryKind::Other,
                    word_lower: None,
                    no_word_gap: false,
                    found_in_list: false,
                });
            }
        }
        // Any other word that speaks ends the `$max3` run, as C clears
        // `phonemes_repeat_count` for every word without the flag: the three
        // tildes of `~ a ~ ~ ~ ~` start again after the "a".
        let same_symbol = match token {
            Token::Punctuation(c) | Token::Operator(c) => repeat_of_symbol == Some(*c),
            _ => false,
        };
        if !same_symbol
            && entries
                .last()
                .is_some_and(|e| e.phonemes.iter().any(|&b| b != 0))
        {
            repeat_of_symbol = None;
            repeat_count = 0;
        }
    }
    // Retag words whose translation is a language-switch marker so that
    // stress promotion skips them (they hold language bytes, not phonemes).
    for e in &mut entries {
        if e.kind == TranslateEntryKind::Word
            && e.phonemes.first() == Some(&crate::phoneme::PHON_SWITCH)
        {
            e.kind = TranslateEntryKind::LangSwitch;
        }
    }
    entries
}

/// The mnemonic of a phoneme code as a string (`"214"`, `"55"`, …).
fn phoneme_mnemonic(code: u8, phdata: &PhonemeData) -> String {
    phdata
        .get(code)
        .map(|ph| {
            ph.mnemonic
                .to_le_bytes()
                .iter()
                .take_while(|&&b| b != 0)
                .map(|&b| b as char)
                .collect()
        })
        .unwrap_or_default()
}

/// Give every toneless syllable the default tone (`phonDEFAULTTONE`, code 17 —
/// mnemonic `1`), as `CalcPitches_Tone` does for the tone languages after the
/// phoneme list is built, so a vowel a program turned into a glide gets none.
///
/// Vietnamese additionally turns the *final stressed* syllable's missing tone
/// into the falling tone 7 at the end of a clause.
fn assign_default_tones_in_codes(
    codes: &mut Vec<PhonemeCode>,
    phdata: &PhonemeData,
    lang: &str,
    switched_vowel: &dyn Fn(&str, u8) -> bool,
) {
    const PHON_DEFAULT_TONE: u8 = 17;
    let base = primary_bcp47_subtag(lang);
    if matches!(base, "cmn" | "zh") {
        mandarin_tones_in_switched(codes, phdata, lang, switched_vowel);
        return;
    }
    if !matches!(base, "hak" | "vi" | "yue" | "shn")
        || phdata.get(PHON_DEFAULT_TONE).is_none_or(|ph| ph.typ != 1)
    {
        return;
    }
    let tone_code = |code: u8, switched: bool| PhonemeCode {
        code,
        is_boundary: false,
        clause_char: None,
        marker: switched.then_some(CodeMarker::ClauseTone),
    };
    // (Before the default tones: C's sandhi sees only the tones given.)
    if base == "hak" {
        hakka_tone_sandhi(codes, phdata);
    }
    let mut i = 0;
    let mut stressed = false;
    let mut switched: Option<String> = None;
    // The last stressed syllable of each clause, and whether it had a tone.
    let mut final_stressed: Option<(usize, bool)> = None;
    let mut finals: Vec<usize> = Vec::new();
    while i < codes.len() {
        let Some(c) = codes.get(i).cloned() else {
            break;
        };
        if c.is_boundary && c.code == 0 {
            if let Some((at, false)) = final_stressed.take() {
                finals.push(at);
            }
            stressed = false;
            i += 1;
            continue;
        }
        if let Some(CodeMarker::LangSwitch(t)) = &c.marker {
            switched = (t != lang).then(|| t.clone());
        }
        let vowel = match &switched {
            Some(t) => switched_vowel(t, c.code),
            None => matches!(phdata.get(c.code), Some(ph) if ph.typ == 2),
        };
        if c.is_boundary || c.marker.is_some() || !vowel {
            stressed |= matches!(c.code, PHON_STRESS_P | PHON_STRESS_P2 | PHON_STRESS_TONIC)
                && !c.is_boundary;
            i += 1;
            continue;
        }
        // The tone follows the vowel's length mark, as C writes them: `b'i:1`.
        let mut at = i + 1;
        while codes.get(at).is_some_and(|n| {
            !n.is_boundary && n.marker.is_none() && n.code == crate::phoneme::PHON_LENGTHEN
        }) {
            at += 1;
        }
        let toned = codes.get(at).is_some_and(|n| {
            !n.is_boundary
                && match n.marker {
                    Some(CodeMarker::ClauseTone) => true,
                    None => switched.is_none() && is_tone_phoneme(n.code, phdata),
                    Some(_) => false,
                }
        });
        if !toned {
            codes.insert(at, tone_code(PHON_DEFAULT_TONE, switched.is_some()));
        }
        if stressed {
            final_stressed = Some((at, toned));
        }
        stressed = false;
        i = at + 1;
    }
    if let Some((at, false)) = final_stressed {
        finals.push(at);
    }
    if base == "vi" {
        let falling = phdata.lookup_phoneme("7");
        if falling != 0 {
            for at in finals {
                if let Some(c) = codes.get_mut(at) {
                    c.code = falling;
                }
            }
        }
    }
}

/// Mandarin tone sandhi and default-tone assignment — a port of the Mandarin
/// branch of `CalcPitches_Tone()` (`intonation.c`), which rewrites the tone
/// phonemes *after* the phoneme list is built:
///
/// * a syllable with no tone gets tone 1 (`55`) after a pause, otherwise the
///   neutral tone 5 (`11`);
/// * `214` + `214` → the first becomes `35` (third-tone sandhi: nǐ hǎo →
///   "ní hǎo"); `214` before any other tone becomes `21`;
/// * `51` + `51` → the first becomes `53`;
/// * the neutral tone `11` takes its level from the preceding tone
///   (`55`→`22`, `53`→`33`, `214`→`44`).
///
/// Runs on the whole clause, since the rules reach across word boundaries.
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
fn apply_tone_sandhi(entries: &mut [TranslateEntry], phdata: &PhonemeData, lang: &str) {
    // Numbers come with their tones as the dictionary writes them.
    for entry in entries.iter_mut() {
        if entry.kind == TranslateEntryKind::Word {
            reorder_tone_phonemes(&mut entry.phonemes, phdata);
        }
    }
    let base = primary_bcp47_subtag(lang);
    // Upstream's tone languages (`langopts.tone_language`), the ones that run
    // `CalcPitches_Tone`.  Thai is *not* one of them — it uses ordinary
    // intonation, so it only gets the placement pass.
    if !matches!(base, "cmn" | "zh" | "yue" | "hak" | "vi" | "shn") {
        return;
    }
    if base != "cmn" && base != "zh" {
        // No sandhi table for these, but every syllable still ends up with a
        // tone: `CalcPitches_Tone` gives a toneless one `phonDEFAULTTONE`
        // ("no tone specified, use default tone 1"), which is why Vietnamese
        // "xin" is `s'i1n` and not `s'in`.
        // (The default tone is given after the phoneme programs have run, by
        // `assign_default_tones_in_codes`.)
        return;
    }
    let code_of = |m: &str| phdata.lookup_phoneme(m);

    // Position of the previous syllable's tone phoneme, so sandhi can rewrite it.
    let mut prev_slot: Option<(usize, usize)> = None;
    let mut prev_tone = String::new(); // previous tone in this word
    let mut prevw_tone = String::new(); // previous tone across word boundaries
    let mut pause = true;
    let mut promoted = false;

    for ei in 0..entries.len() {
        match entries[ei].kind {
            TranslateEntryKind::Word => {}
            TranslateEntryKind::ClauseBoundary => {
                pause = true;
                prevw_tone.clear();
                prev_tone.clear();
                continue;
            }
            _ => continue,
        }
        prev_tone.clear(); // forget across word boundaries
        let mut i = 0;
        while i < entries[ei].phonemes.len() {
            let code = entries[ei].phonemes.at(i);
            if code == 0 {
                break;
            }
            // (A replacement's words are separate words.)
            if code == crate::phoneme::PHON_END_WORD {
                prev_tone.clear();
            }
            let is_vowel = matches!(phdata.get(code), Some(ph) if ph.typ == 2);
            if !is_vowel {
                i += 1;
                continue;
            }

            // The syllable's tone sits directly after its vowel.
            let mut tone_pos = i + 1;
            let existing = entries[ei]
                .phonemes
                .get(tone_pos)
                .copied()
                .filter(|&c| is_tone_phoneme(c, phdata));
            let tone_code = match existing {
                Some(c) => {
                    promoted = false;
                    c
                }
                None => {
                    let mnem = if pause || promoted { "55" } else { "11" };
                    let c = code_of(mnem);
                    if c == 0 {
                        i += 1;
                        continue;
                    }
                    promoted = mnem == "55";
                    entries[ei].phonemes.insert(tone_pos, c);
                    c
                }
            };
            let mut tone = phoneme_mnemonic(tone_code, phdata);

            // 214 + 214 → 35 + 214;  214 + anything else → 21
            if prevw_tone == "214" {
                if let Some((pe, pp)) = prev_slot {
                    let joins = tone == "214";
                    let new = if joins { code_of("35") } else { code_of("21") };
                    if new != 0 {
                        entries[pe].phonemes.put(pp, new);
                    }
                    // (The pair is one word only when the dictionary lists it
                    // as one, `你好`: separate words keep their stress and gap.)
                }
            }
            // 51 + 51 → 53 + 51 (within the word)
            if prev_tone == "51" && tone == "51" {
                if let Some((pe, pp)) = prev_slot {
                    let new = code_of("53");
                    if new != 0 {
                        entries[pe].phonemes.put(pp, new);
                    }
                }
            }
            // The neutral tone takes its level from the preceding tone, and is
            // unstressed.  (C's packed mnemonic `0x3533` is "35".)
            if tone == "11" {
                let replacement = match prevw_tone.as_str() {
                    "55" => "22",
                    "35" => "33",
                    "214" => "44",
                    _ => "",
                };
                if !replacement.is_empty() {
                    let new = code_of(replacement);
                    if new != 0 {
                        entries[ei].phonemes.put(tone_pos, new);
                        tone = replacement.to_string();
                    }
                }
                // Drop the syllable's stress marks: back to the previous vowel
                // (or its tone) in this word.
                let phonemes = &mut entries[ei].phonemes;
                let mut k = i;
                while k > 0 {
                    let c = phonemes.at(k - 1);
                    let stop =
                        phdata.get(c).is_some_and(|ph| ph.typ == 2) || is_tone_phoneme(c, phdata);
                    if stop {
                        break;
                    }
                    if matches!(
                        c,
                        PHON_STRESS_2
                            | PHON_STRESS_3
                            | PHON_STRESS_P
                            | PHON_STRESS_P2
                            | PHON_STRESS_TONIC
                    ) {
                        phonemes.remove(k - 1);
                        tone_pos -= 1;
                    }
                    k -= 1;
                }
            }

            prev_slot = Some((ei, tone_pos));
            prev_tone = tone.clone();
            prevw_tone = tone;
            pause = false;
            i = tone_pos + 1;
        }
    }
}

#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
pub(crate) fn promote_translate_entries(
    entries: &mut [TranslateEntry],
    phdata: &PhonemeData,
    lang: &str,
) {
    const FLAG_STREND: u32 = 1 << 9;
    const FLAG_STREND2: u32 = 1 << 10;
    const PHON_STRESS_P_CODE: u8 = 6;
    const PHON_STRESS_P2_CODE: u8 = 7;

    #[expect(
        clippy::indexing_slicing,
        reason = "indices are bounded by the loops over these lists"
    )]
    fn promote_clause(entries: &mut [TranslateEntry], phdata: &PhonemeData, lang: &str) {
        let n = entries.len();
        #[allow(clippy::needless_range_loop)]
        for i in 0..n {
            if entries[i].kind != TranslateEntryKind::Word {
                continue;
            }
            let dict_flags = entries[i].dict_flags;
            if dict_flags & (FLAG_STREND | FLAG_STREND2 | FLAG_UNSTRESS_END) == 0 {
                continue;
            }

            // A word spoken in another language (`_^_en`) is still a word of
            // the clause: "le week-end" does not end on "le".
            let is_last_word = entries.get(i + 1..).unwrap_or_default().iter().all(|e| {
                !matches!(
                    e.kind,
                    TranslateEntryKind::Word | TranslateEntryKind::LangSwitch
                )
            });
            let following_all_unstressed = entries
                .get(i + 1..)
                .unwrap_or_default()
                .iter()
                .filter(|e| e.kind == TranslateEntryKind::Word)
                .all(|e| {
                    !e.phonemes
                        .iter()
                        .any(|&c| c == PHON_STRESS_P_CODE || c == PHON_STRESS_P2_CODE)
                });

            // `$unstressend`: a word that ends a clause with a stress before it
            // has only a secondary one (Hungarian "teher").
            if dict_flags & FLAG_UNSTRESS_END != 0 {
                let stressed_before = STRESSED_BEFORE_PHRASE.with(std::cell::Cell::get)
                    || entries.get(..i).unwrap_or_default().iter().any(|e| {
                        e.kind == TranslateEntryKind::Word
                            && e.phonemes
                                .iter()
                                .any(|&c| c == PHON_STRESS_P_CODE || c == PHON_STRESS_P2_CODE)
                    });
                if is_last_word && stressed_before {
                    change_word_stress(&mut entries[i].phonemes, phdata, 3);
                }
                continue;
            }

            // A composed entry (a spelled-out number: "cento‖e‖um") holds several
            // words separated by END_WORD; end-stress promotion belongs to its
            // LAST word, exactly as if they had been typed as separate words.
            // Promoting over the whole buffer found "cento"'s existing stress and
            // did nothing, so pt "101" read `sˈẽŋtʊ i ũŋ` while "cento e um" gave
            // `… ˈũŋ`.
            let phonemes = &mut entries[i].phonemes;
            match phonemes
                .iter()
                .rposition(|&c| c == crate::phoneme::PHON_END_WORD)
            {
                Some(sep) => {
                    let mut tail = phonemes.get(sep + 1..).unwrap_or_default().to_vec();
                    promote_strend_stress(
                        &mut tail,
                        phdata,
                        dict_flags,
                        is_last_word,
                        following_all_unstressed,
                    );
                    phonemes.truncate(sep + 1);
                    phonemes.extend(tail);
                }
                None => promote_strend_stress(
                    phonemes,
                    phdata,
                    dict_flags,
                    is_last_word,
                    following_all_unstressed,
                ),
            }
        }

        // (A clause with no primary stress at all gets one from
        // `promote_stress_in_unstressed_clauses`, after the phoneme programs have
        // run: C places it in the pitch pass, so Portuguese "ser" is `s'yr-`,
        // with the unstressed vowel.)
        if primary_bcp47_subtag(lang) == "en" {
            apply_en_wh_clause_initial_secondary(entries, phdata);
        }
        // Tone languages: default tones + sandhi, across the whole clause.
        apply_tone_sandhi(entries, phdata, lang);
    }

    #[expect(
        clippy::indexing_slicing,
        reason = "indices are bounded by the loops over these lists"
    )]
    fn apply_en_wh_clause_initial_secondary(entries: &mut [TranslateEntry], phdata: &PhonemeData) {
        const PHON_STRESS_2: u8 = 4;
        const PHON_STRESS_P: u8 = 6;
        const PHON_STRESS_P2: u8 = 7;
        const WH: &[&str] = &[
            "when", "where", "what", "who", "why", "how", "which", "while",
        ];

        let word_ix: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == TranslateEntryKind::Word && !e.phonemes.is_empty())
            .map(|(i, _)| i)
            .collect();
        if word_ix.len() < 2 {
            return;
        }
        let i0 = word_ix.at(0);
        let i1 = word_ix.at(1);
        let Some(w) = entries[i0].word_lower.as_deref() else {
            return;
        };
        if !WH.iter().any(|&kw| kw == w) {
            return;
        }
        if !entries[i1]
            .phonemes
            .iter()
            .any(|&c| c == PHON_STRESS_P || c == PHON_STRESS_P2)
        {
            return;
        }
        let ph = &entries[i0].phonemes;
        let Some(fv) = ph
            .iter()
            .position(|&c| phdata.get(c).map(|p| p.typ == 2).unwrap_or(false))
        else {
            return;
        };
        if ph
            .get(..fv)
            .unwrap_or_default()
            .iter()
            .any(|&c| matches!(c, 4 | 5 | 6 | 7))
        {
            return;
        }
        let mut new_ph = ph.to_vec();
        new_ph.insert(fv, PHON_STRESS_2);
        entries[i0].phonemes = new_ph;
    }

    let clause_boundaries: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.kind == TranslateEntryKind::ClauseBoundary)
        .map(|(i, _)| i)
        .collect();

    let lang_for_promo = lang;
    if clause_boundaries.is_empty() {
        promote_clause(entries, phdata, lang_for_promo);
    } else {
        let mut prev_end = 0usize;
        let mut boundaries_with_end = clause_boundaries.clone();
        boundaries_with_end.push(entries.len());
        for &bound in &boundaries_with_end {
            let slice_end = if bound < entries.len() {
                bound
            } else {
                entries.len()
            };
            if slice_end > prev_end {
                promote_clause(&mut entries[prev_end..slice_end], phdata, lang_for_promo);
            }
            prev_end = if bound < entries.len() {
                bound + 1
            } else {
                entries.len()
            };
        }
    }
}

/// English “linking ɹ” after **or** before a vowel-initial word (same condition as IPA post-pass).
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
pub(crate) fn apply_en_or_linking_r(entries: &mut [TranslateEntry], phdata: &PhonemeData) {
    let r_code = phdata.lookup_phoneme("r");
    if r_code == 0 {
        return;
    }

    fn next_word_vowel_initial(
        entries: &[TranslateEntry],
        from: usize,
        phdata: &PhonemeData,
    ) -> bool {
        entries
            .get(from + 1..)
            .unwrap_or_default()
            .iter()
            .find(|e| e.kind == TranslateEntryKind::Word && !e.phonemes.is_empty())
            .map(|e| {
                e.phonemes
                    .iter()
                    .find(|&&c| c > 8 && c != 15)
                    .and_then(|&c| phdata.get(c))
                    .map(|ph| ph.typ == 2)
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    for i in 0..entries.len() {
        if entries[i].kind != TranslateEntryKind::Word {
            continue;
        }
        if entries[i].word_lower.as_deref() != Some("or") {
            continue;
        }
        if !next_word_vowel_initial(entries, i, phdata) {
            continue;
        }
        while entries[i].phonemes.last() == Some(&0) {
            entries[i].phonemes.pop();
        }
        entries[i].phonemes.push(r_code);
        entries[i].phonemes.push(0);
    }
}

/// Top-level text translator.
///
/// Create with [`Translator::new_default`] for the most common case.
/// Use [`Translator::text_to_ipa`] or [`Translator::translate_to_codes`].
pub struct Translator {
    /// Language and speech-rate configuration.
    pub options: LangOptions,
    /// Resolved espeak-ng data directory.
    data_dir: PathBuf,
    /// The data last loaded, so that each call needn't read it again.
    cache: std::sync::Mutex<Option<std::sync::Arc<LanguageData>>>,
}

/// What the files say about a language.
struct LanguageData {
    /// What it was loaded for (`Translator::options` can be changed).
    lang: String,
    dict_condition: u32,
    dict: Dictionary,
    phdata: PhonemeData,
    stress_opts: StressOpts,
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

        // A `+variant` suffix (`en+f3`) only affects the acoustic voice; the
        // base tag selects the language data used for text→phonemes.
        let (base, _variant) = split_voice_variant(lang);
        let mut options = LangOptions::for_lang(base);
        // Honour the voice's `dictrules` directive (Brazilian Portuguese etc.).
        options.dict_condition = crate::voices::voice_dict_condition(&dir, base);
        options.bracket_pause = crate::voices::voice_bracket_pause(&dir, base);
        Ok(Translator {
            options,
            data_dir: dir,
            cache: std::sync::Mutex::new(None),
        })
    }

    /// The data of the current language, loaded on first use.
    fn data(&self) -> Result<std::sync::Arc<LanguageData>> {
        let lang = &self.options.lang;
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(data) = cache.as_ref() {
            if data.lang == *lang && data.dict_condition == self.options.dict_condition {
                return Ok(data.clone());
            }
        }
        // An unknown voice is a *voice* error, not an unimplemented feature:
        // `-v alex` used to report "not yet implemented: … dict not found",
        // which says nothing about the real problem (upstream #2469, where C
        // prints "Unknown phoneme table: ''" and then crashes).
        let dict_stem = resolve_dict_stem(&self.data_dir, lang)
            .ok_or_else(|| Error::VoiceNotFound(lang.to_string()))?;
        let dict_path = dict_path(&self.data_dir, &dict_stem)
            .ok_or_else(|| Error::VoiceNotFound(lang.to_string()))?;
        let dict_bytes = std::fs::read(&dict_path).map_err(Error::Io)?;
        let mut dict = Dictionary::from_bytes(&dict_stem, dict_bytes)?;
        dict.dict_condition = self.options.dict_condition;
        if !self.data_dir.join("phontab").exists() {
            return Err(Error::NotImplemented(
                "translate_to_codes: phontab not found",
            ));
        }
        let mut phdata = PhonemeData::load(&self.data_dir)?;
        // The table is usually named after the language; a few voices redirect
        // it (`phonemes <name>`, e.g. Norwegian Bokmål `nb` → `no`).
        select_phoneme_table(&mut phdata, &self.data_dir, lang)?;
        let data = std::sync::Arc::new(LanguageData {
            lang: lang.clone(),
            dict_condition: self.options.dict_condition,
            dict,
            phdata,
            stress_opts: StressOpts::for_lang_in(lang, &self.data_dir),
        });
        *cache = Some(data.clone());
        Ok(data)
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
                        _ => Intonation::FullStop,
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
                _ => {
                    current.push(c);
                }
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
        self.text_to_ipa_with_options(text, false, false, false, true)
    }

    /// Each source word with the phonemes it was rendered as — the Rust form of
    /// upstream's proposed `espeak_TextToWordPhonemePairs` (PR #2533).
    ///
    /// The phonemes come from the *same* rendering path as
    /// [`text_to_ipa`](Self::text_to_ipa), so joining them with single spaces
    /// reproduces that output; the alignment is taken from the pipeline rather
    /// than re-derived, which is what makes the awkward cases right — a number
    /// expands to several spoken words (`2.5` → `tˈuː pɔɪnt fˈaɪv`), an
    /// abbreviation to a different word, and a multi-word dictionary entry to
    /// one pronunciation unit.
    ///
    /// `offset`/`length` are byte offsets into `text`, located by searching
    /// forward for the token, so they are exact for text the tokenizer did not
    /// rewrite and `length == 0` where it did (a merged currency amount, a
    /// substituted abbreviation).
    ///
    /// ```no_run
    /// let tr = espeak_ng::Translator::new("en-us", None).unwrap();
    /// for w in tr.text_to_ipa_words("Version 2.5").unwrap() {
    ///     println!("{} -> {}", w.word, w.phonemes);
    /// }
    /// ```
    #[expect(
        clippy::indexing_slicing,
        reason = "indices are bounded by the loops over these lists"
    )]
    pub fn text_to_ipa_words(&self, text: &str) -> Result<Vec<WordPhonemes>> {
        let mut track: Vec<TokenCodes> = Vec::new();
        let codes = self.translate_to_codes_tracked(text, true, Some(&mut track))?;
        let data = self.data()?;
        let phdata = &data.phdata;
        let render = |slice: &[PhonemeCode]| {
            phoneme_ipa::codes_to_ipa_opts(
                slice,
                phdata,
                &self.data_dir,
                &self.options.lang,
                false,
                self.options.ipa_tie,
                self.options.ipa_separator,
            )
        };
        // The flat rendering is authoritative: where the per-word counts add up
        // to it, the words are cut straight out of it.
        let flat = render(&codes);
        let pieces: Vec<&str> = flat.split_whitespace().collect();
        let rendered: Vec<String> = track
            .iter()
            .map(|t| render(&codes[t.codes.clone()]).trim().to_string())
            .collect();
        let counts: Vec<usize> = rendered
            .iter()
            .map(|r| r.split_whitespace().count())
            .collect();
        let exact = counts.iter().sum::<usize>() == pieces.len();

        let mut out = Vec::new();
        let mut taken = 0;
        let mut search_from = 0;
        for (t, (phon, n)) in track.iter().zip(rendered.iter().zip(&counts)) {
            let phonemes = if exact {
                pieces.get(taken..taken + n).unwrap_or_default().join(" ")
            } else {
                phon.clone()
            };
            taken += n;
            if phonemes.is_empty() {
                continue; // punctuation, or a token that says nothing
            }
            let (offset, length) = find_token_span(text, &t.text, search_from);
            search_from = offset + length;
            out.push(WordPhonemes {
                word: t.text.clone(),
                offset,
                length,
                phonemes,
            });
        }
        Ok(out)
    }

    /// The phonemes this language's dictionary gives for a *character-name* key
    /// (`_@` → "at sign"), or `None` when it has no entry.
    ///
    /// C's cross-language fallback for a character the current language cannot
    /// name is `LookupCharName` in the default voice, which reads exactly this
    /// key — so `he @` is "at sign", not the short reading of `@` ("at") that
    /// re-translating the character itself would give.
    /// Phonemes of this language after its phoneme programs have run, as C's
    /// `MakePhonemeList` runs them over a switched word: English `_eth`
    /// `i:TI'0pIk` is said `i:TI;'0pIk`.
    pub(crate) fn phonemes_after_programs(&self, phonemes: &[u8]) -> Vec<PhonemeCode> {
        let mut codes: Vec<PhonemeCode> = phonemes
            .iter()
            .filter(|&&b| b != 0)
            .map(|&code| PhonemeCode {
                code,
                ..Default::default()
            })
            .collect();
        let Ok(data) = self.data() else {
            return codes;
        };
        let mut flags = vec![true; codes.len()];
        resolve_virtual_phonemes_in_codes(
            &mut codes,
            &data.phdata,
            &mut flags,
            data.stress_opts.reduce,
        );
        flags.resize(codes.len(), true);
        apply_list_editing_instructions(
            &mut codes,
            &data.phdata,
            &flags,
            data.stress_opts.reduce,
            0,
            0,
        );
        drop_vowelless_stress(&mut codes, &data.phdata);
        syllabic_after_vowel(&mut codes, &data.phdata);
        codes
    }

    /// A letter's name in this language, as C's `LookupLetter` finds it.
    /// A name given as text is read as one word, as C's `Lookup` reads it,
    /// so only its first word is said (`☺` → "smiling").
    pub(crate) fn letter_phonemes(&self, c: char) -> Option<Vec<u8>> {
        let data = self.data().ok()?;
        let lower = c.to_lowercase().to_string();
        if let Some(text) = self.replacement_text(&lower) {
            let first = text.split_whitespace().next()?.to_lowercase();
            let wr = word_to_phonemes(
                &first,
                &data.dict,
                &data.phdata,
                &data.stress_opts,
                &self.options,
            );
            return wr.phonemes.iter().any(|&b| b != 0).then_some(wr.phonemes);
        }
        letter_name(
            &lower,
            false,
            &data.dict,
            &data.phdata,
            &data.stress_opts,
            &self.options,
        )
    }

    /// The replacement text of a `$textmode` entry for `word` (or `_word`).
    fn replacement_text(&self, word: &str) -> Option<String> {
        let data = self.data().ok()?;
        let ctx = LookupCtx {
            lookup_symbol: true,
            dict_condition: data.dict.dict_condition,
            ..Default::default()
        };
        [word.to_string(), format!("_{word}")]
            .iter()
            .find_map(|key| {
                let r = lookup(&data.dict, key, &ctx)?;
                let text =
                    r.flags1.found() && r.flags1.textmode() != self.options.reversed_textmode;
                text.then(|| String::from_utf8_lossy(trim_lookup(&r.phonemes)).into_owned())
            })
    }

    /// A symbol's name in this language when another language switched to it
    /// for one, as C ends up with it: a plain entry's phonemes as they are
    /// (`=` `_:'i:kw@Lz_:`); of a text name, only the first word, and only
    /// when the list gives it (`≈` "approximately", but `∞` "infinity" and `÷`
    /// "divided by" are lost).
    pub(crate) fn switched_symbol_phonemes(&self, c: char, spelled: bool) -> Option<Vec<u8>> {
        let data = self.data().ok()?;
        // C only switches when this language has a plain entry for it
        // (`LookupLetter` asks English for `c`, not `_c`): Dutch `3.5` says
        // nothing for the point.
        if !dict_knows_word(&c.to_string(), &data.dict) {
            return None;
        }
        // A single character switched to this language is spelled when no
        // letter follows it: its `_c` name, else its entry (`@` → "at sign").
        // Before a letter it is read as a word (`a @ b` → `_at_`).
        if self.replacement_text(&format!("_{c}")).is_none() {
            let keys = if spelled {
                [format!("_{c}"), c.to_string()]
            } else {
                [c.to_string(), format!("_{c}")]
            };
            let ctx = LookupCtx {
                lookup_symbol: true,
                dict_condition: data.dict.dict_condition,
                ..Default::default()
            };
            for key in keys {
                let Some(r) = lookup(&data.dict, &key, &ctx).filter(|r| r.flags1.found()) else {
                    continue;
                };
                if r.flags1.textmode() != self.options.reversed_textmode {
                    break;
                }
                let mut ph = trim_lookup(&r.phonemes).to_vec();
                if !ph.is_empty() {
                    ph.push(0);
                    set_word_stress(
                        &mut ph,
                        &data.phdata,
                        &data.stress_opts,
                        Some(r.flags1.0),
                        -1,
                        0,
                    );
                    return Some(trim_lookup(&ph).to_vec());
                }
            }
        }
        // A name given as text is read only when its first word is in the
        // list (`≈` "approximately", `±` "plus or minus"); otherwise C loses
        // it (`∞` "infinity", `÷` "divided by").
        let text = self.replacement_text(&c.to_string())?;
        let words = segment_replacement(&text, &self.options.lang);
        if !words
            .first()
            .is_some_and(|w| dict_knows_word(&w.to_lowercase(), &data.dict))
        {
            return None;
        }
        let mut out: Vec<u8> = Vec::new();
        for word in words {
            let wr = word_to_phonemes(
                &word.to_lowercase(),
                &data.dict,
                &data.phdata,
                &data.stress_opts,
                &self.options,
            );
            let ph = trim_lookup(&wr.phonemes);
            if !out.is_empty() && !ph.is_empty() {
                out.push(PHON_END_WORD);
            }
            out.extend_from_slice(ph);
        }
        (!out.is_empty()).then_some(out)
    }

    /// The phonemes of a dictionary key (`_zh`), if it has an entry.
    pub(crate) fn key_phonemes(&self, key: &str) -> Option<Vec<u8>> {
        let data = self.data().ok()?;
        let ph = lookup_num_phonemes(&data.dict, key);
        ph.iter().any(|&b| b != 0).then_some(ph)
    }

    /// Translate free text to IPA and report how the last clause ended.
    ///
    /// The Rust form of upstream's `espeak_TextToPhonemesWithTerminator`
    /// (added in 1.53.0): the phoneme string alone doesn't say whether the text
    /// ended a sentence, asked a question, or simply ran out, which a caller
    /// driving a phonemizer or a prosody model needs.
    ///
    /// ```rust,no_run
    /// # use espeak_ng::translate::{Translator, ClauseTerminator};
    /// let t = Translator::new("en", None)?;
    /// let (ipa, end) = t.text_to_ipa_with_terminator("Hello. Are you there?")?;
    /// assert_eq!(end, ClauseTerminator::Question);
    /// assert!(end.is_sentence());
    /// # let _ = ipa;
    /// # Ok::<(), espeak_ng::Error>(())
    /// ```
    pub fn text_to_ipa_with_terminator(&self, text: &str) -> Result<(String, ClauseTerminator)> {
        let ipa = self.text_to_ipa(text)?;
        Ok((ipa, clause_terminator_of(text)))
    }

    /// Translate free text to IPA, interpreting SSML markup (`-m` mode).
    ///
    /// Equivalent to `espeak-ng -v <lang> -q --ipa -m <text>`.  See
    /// [`ssml::strip_markup`] for the supported subset.
    pub fn text_to_ipa_ssml(&self, text: &str) -> Result<String> {
        self.text_to_ipa_with_options(text, false, false, true, true)
    }

    /// `allow_switch` enables `phonSWITCH` language switching (`_^_LL` dict
    /// entries).  It is disabled for the re-translation of an already-switched
    /// word so a switch cannot recurse indefinitely.
    pub(crate) fn text_to_ipa_with_options(
        &self,
        text: &str,
        preserve_punctuation: bool,
        flatten_clauses: bool,
        markup: bool,
        allow_switch: bool,
    ) -> Result<String> {
        // SSML markup mode: parse tags/entities into language-tagged segments.
        let stripped;
        let text = if markup {
            let segments = ssml::process_markup(text);
            // `<voice xml:lang=…>` spans translate their content in another
            // language, each as its own clause.  Delegate per segment.
            if segments.iter().any(|s| s.lang.is_some()) {
                let mut voice_cache: std::collections::HashMap<String, Translator> =
                    std::collections::HashMap::new();
                let sep = if flatten_clauses { " " } else { "\n" };
                let mut parts: Vec<String> = Vec::new();
                for seg in &segments {
                    if seg.text.trim().is_empty() {
                        continue;
                    }
                    // Choose the translator: base (self) or the voice language.
                    // The `<voice>` selector may be a language tag or a voice
                    // *name* ("French (France)"); resolve it, falling back to the
                    // base voice if it doesn't match (never error).
                    let resolved = seg
                        .lang
                        .as_deref()
                        .filter(|l| *l != self.options.lang)
                        .and_then(|l| resolve_voice_lang(l, &self.data_dir))
                        .filter(|lang| *lang != self.options.lang);
                    let translator: &Translator = match resolved {
                        Some(lang) => {
                            if !voice_cache.contains_key(&lang) {
                                if let Ok(tr) = Translator::new(&lang, Some(&self.data_dir)) {
                                    voice_cache.insert(lang.clone(), tr);
                                }
                            }
                            voice_cache.get(&lang).unwrap_or(self)
                        }
                        None => self,
                    };
                    let seg_text =
                        interpret_segment_text(&seg.text, seg.interpret, &translator.options.lang);
                    let ipa = translator.text_to_ipa_with_options(
                        &seg_text,
                        preserve_punctuation,
                        flatten_clauses,
                        false,
                        allow_switch,
                    )?;
                    if !ipa.trim().is_empty() {
                        parts.push(ipa);
                    }
                }
                return Ok(parts.join(sep));
            }
            stripped = segments
                .into_iter()
                .map(|s| interpret_segment_text(&s.text, s.interpret, &self.options.lang))
                .collect::<String>();
            stripped.as_str()
        } else {
            text
        };
        // The clause's finished phoneme-code list *is* the IPA source: C's
        // `GetTranslatedPhonemeString(espeakPHONEMES_IPA)` walks exactly the
        // list `-x` prints, so the two can't disagree about which phonemes a
        // clause contains.  Rendering per word instead — which this used to do —
        // cannot see the cross-word allophones (Spanish `ðˈias`, Ukrainian's
        // voicing assimilation) or the contextual IPA names a phoneme's program
        // selects (Italian `*` is `r` between vowels, `ɾ` elsewhere).
        let codes = self.translate_to_codes_inner(text, allow_switch)?;
        let data = self.data()?;
        let phdata = &data.phdata;
        let mut ipa_out = phoneme_ipa::codes_to_ipa_opts(
            &codes,
            phdata,
            &self.data_dir,
            &self.options.lang,
            preserve_punctuation,
            self.options.ipa_tie,
            self.options.ipa_separator,
        );
        if flatten_clauses {
            ipa_out = ipa_out.replace('\n', " ");
        }
        Ok(ipa_out)
    }

    /// Translate text into a raw phoneme-code sequence for synthesis.
    ///
    /// Uses the same token → dictionary → **clause-level stress promotion** pipeline as
    /// [`Translator::text_to_ipa`] (including English clause-initial WH secondary stress and
    /// linking **/r/** after **or** before a vowel), then flattens to phoneme bytes.
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
        self.translate_to_codes_inner(text, true)
    }

    /// [`translate_to_codes`](Self::translate_to_codes) with `phonSWITCH`
    /// language switching selectable — a word reached *through* a switch is
    /// translated with `allow_switch = false`, so a switch can't recurse.
    pub(crate) fn translate_to_codes_inner(
        &self,
        text: &str,
        allow_switch: bool,
    ) -> Result<Vec<PhonemeCode>> {
        self.translate_to_codes_tracked(text, allow_switch, None)
    }

    /// [`translate_to_codes_inner`](Self::translate_to_codes_inner) with an
    /// optional record of **which source token produced which codes**, which is
    /// what the word-level API ([`text_to_ipa_words`](Self::text_to_ipa_words))
    /// needs: the tokens have been through fifteen rewriting passes by the time
    /// they are translated, so the mapping can only be taken from the pipeline
    /// itself.
    #[expect(
        clippy::indexing_slicing,
        reason = "indices are bounded by the loops over these lists"
    )]
    pub(crate) fn translate_to_codes_tracked(
        &self,
        text: &str,
        allow_switch: bool,
        track: Option<&mut Vec<TokenCodes>>,
    ) -> Result<Vec<PhonemeCode>> {
        let lang = &self.options.lang;
        let data = self.data()?;
        let (dict, phdata, stress_opts) = (&data.dict, &data.phdata, &data.stress_opts);
        let saved_dir = DATA_DIR.with(|d| d.replace(Some(self.data_dir.clone())));
        struct RestoreDir(Option<std::path::PathBuf>);
        impl Drop for RestoreDir {
            fn drop(&mut self) {
                let dir = self.0.take();
                DATA_DIR.with(|d| *d.borrow_mut() = dir);
            }
        }
        let _restore_dir = RestoreDir(saved_dir);
        STALE_TABLE.with(|t| *t.borrow_mut() = None);

        // Capital-letter indication before a capitalised word (C `option_capitals`):
        //   `-k 1` → the `phonCAPITAL` *sound* (short pause + capital phoneme),
        //   `-k 2` → the spoken "capital" announcement (dict `_cap`).
        // Empty when disabled.
        let cap_codes: Vec<u8> = match self.options.capitals {
            1 => vec![
                crate::phoneme::PHON_PAUSE_SHORT,
                crate::phoneme::PHON_CAPITAL,
            ],
            2 => word_to_phonemes("_cap", dict, phdata, stress_opts, &self.options).phonemes,
            _ => Vec::new(),
        };

        let text = normalize_apostrophes(text);
        let mut grammar = std::borrow::Cow::Borrowed(&self.options.number_grammar);
        if text.contains(is_dict_number_symbol) {
            let grammar = grammar.to_mut();
            grammar.powers_as_digits = true;
            let mut symbols: Vec<char> =
                text.chars().filter(|&c| is_dict_number_symbol(c)).collect();
            symbols.sort_unstable();
            symbols.dedup();
            symbols.retain(|c| !lookup_num_phonemes(dict, &c.to_string()).is_empty());
            grammar.dict_symbols = symbols;
        }
        // The suffixes after numbers that mark ordinals.
        let mut suffixes: Vec<String> = Vec::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if !c.is_ascii_digit() || chars.peek().is_some_and(char::is_ascii_digit) {
                continue;
            }
            let mut suffix = String::new();
            while let Some(&n) = chars.peek() {
                if n.is_alphabetic() || n == 'º' || n == 'ª' {
                    suffix.extend(n.to_lowercase());
                    chars.next();
                } else {
                    break;
                }
            }
            if !suffix.is_empty()
                && !suffixes.contains(&suffix)
                && !lookup_num_phonemes(dict, &format!("_#{suffix}")).is_empty()
            {
                suffixes.push(suffix);
            }
        }
        if !suffixes.is_empty() {
            grammar.to_mut().ordinal_suffixes = suffixes;
        }
        let mut tokens = tokenize_opts(&text, &grammar);
        suppress_abbreviation_periods(
            &mut tokens,
            self.options.number_grammar.ordinals.dot_marks_ordinal,
            // C's `lowercase_sentence` (`lowercaseSentence` in a voice file):
            // a dot ends a sentence even before a lower-case word.
            matches!(primary_bcp47_subtag(&self.options.lang), "ka" | "mto" | "xex"),
        );
        // C reads a `:` between numbers as a word, by the rules (German
        // `_L05) :` is "Uhr").
        for i in 1..tokens.len() {
            if matches!(tokens.get(i), Some(Token::Punctuation('-'))) {
                let before = tokens.get(i - 1);
                let after = tokens.get(i + 1);
                if matches!(before, Some(Token::Number(_) | Token::Word(_)))
                    && matches!(after, Some(Token::Number(_)))
                {
                    // So is a hyphen before a number (English "dash" between
                    // numbers, Portuguese "menos"; a pause after a word).
                    tokens.put(i, Token::Word("-".to_string()));
                } else if matches!(before, Some(Token::Number(_)))
                    && matches!(after, Some(Token::Word(_)))
                {
                    // After a number it joins the word that follows ("5-x").
                    tokens.put(i, Token::WordJoin);
                }
            }
            if matches!(tokens.get(i), Some(Token::Punctuation(':')))
                && matches!(tokens.get(i - 1), Some(Token::Number(_)))
                && matches!(tokens.get(i + 1), Some(Token::Number(_)))
            {
                tokens.put(i, Token::Word(":".to_string()));
                // "looks like a time 02:30, omit the leading zero"
                let two = |t: Option<&Token>| {
                    matches!(t, Some(Token::Number(NumberToken::Cardinal(d)))
                        if d.len() == 2 && d.bytes().all(|b| b.is_ascii_digit()))
                };
                // (C's clause buffer has a space after the minutes, always.)
                if two(tokens.get(i - 1)) && two(tokens.get(i + 1)) {
                    if let Some(Token::Number(NumberToken::Cardinal(d))) = tokens.get(i - 1) {
                        if let Some(rest) = d.strip_prefix('0') {
                            let hour = Token::Number(NumberToken::Cardinal(rest.to_string()));
                            tokens.put(i - 1, hour);
                        }
                    }
                }
            }
        }
        apply_space_grouping(&mut tokens, &self.options.number_grammar);
        let dots: Vec<usize> = tokens
            .iter()
            .enumerate()
            .filter(|(_, t)| matches!(t, Token::Dot))
            .map(|(i, _)| i)
            .collect();
        for &i in &dots {
            tokens.put(i, Token::Space);
        }
        let saved_dots = DOTS.with(|d| d.replace(dots));
        let saved_no_tone = NO_TONE_CLAUSES.with(|n| n.take());
        let mut entries =
            build_translate_entries(&tokens, dict, phdata, stress_opts, &self.options);
        let no_tone_clauses = NO_TONE_CLAUSES.with(|n| n.replace(saved_no_tone));
        DOTS.with(|d| d.replace(saved_dots));
        // (Not for a word read for another language's clause: its position in
        // that clause is what counts.)
        if allow_switch || PHRASE_ENDS_CLAUSE.with(std::cell::Cell::get) {
            promote_translate_entries(&mut entries, phdata, lang);
        }
        if primary_bcp47_subtag(lang) == "en" {
            apply_en_or_linking_r(&mut entries, phdata);
        }

        debug_assert_eq!(tokens.len(), entries.len());
        let mut codes: Vec<PhonemeCode> = Vec::new();
        // Parallel to `codes`: did this phoneme come from an explicit
        // translation (a dictionary entry or a number key) rather than the
        // letter-to-sound rules?  Turkish/Bashkir/Tatar suppress vowel reduction
        // then (`isTranslationGiven`).
        let mut from_dict: Vec<bool> = Vec::new();
        // `phonSWITCH` entries carry `[phonSWITCH, <lang bytes…>]` rather than
        // real phonemes.  Re-translate the word with the target language and
        // wrap its codes in switch markers, so `-x` prints `(en)h@l'oU(ru)`
        // instead of leaking the raw marker — the same thing `text_to_ipa`
        // already does.  Translators are cached per target within the clause.
        let mut switch_cache: std::collections::HashMap<String, Translator> =
            std::collections::HashMap::new();
        let mut open_switch: Option<String> = None;
        // A bracket's pause opened the clause: the next space is no break.
        let mut pause_opens_word = false;
        // C's `prepause_timeout`: words to go before a `$pause` counts again.
        let mut prepause_timeout = 0u8;

        // Parallel to `codes` when tracking: the token each code came from.
        // Filled one iteration behind — every `continue` in the loop below would
        // otherwise need its own bookkeeping.
        let mut owners: Vec<usize> = Vec::new();
        let tracking = track.is_some();
        let mut owner_now = 0usize;

        // The previous word asked for this one's first consonant to be doubled.
        let mut double_next = false;
        // Words already read with the switched word before them.
        let mut switched_through = 0usize;
        // The last entry was a letter that ended in another language, or in a
        // switched alphabet's letter written in this table.
        let mut letter_ended_switched = false;
        let mut letter_ended_raw = false;
        for (token_ix, (token, entry)) in tokens.iter().zip(entries.iter()).enumerate() {
            if tracking {
                owners.resize(codes.len(), owner_now);
                owner_now = token_ix;
            }
            if token_ix < switched_through {
                continue;
            }
            // (A clause boundary ends the doubling: Finnish "purje, ne".)
            if matches!(token, Token::ClauseBoundary(_)) {
                double_next = false;
            }
            if matches!(
                token,
                Token::Word(_)
                    | Token::Number(_)
                    | Token::InlinePhonemes(_)
                    | Token::Operator(_)
                    | Token::Punctuation(_)
            ) {
                prepause_timeout = prepause_timeout.saturating_sub(1);
            }
            // A word after punctuation starts a word of its own, as in C's
            // clause buffer (`3.º` is `3 . º`).
            if matches!(token, Token::Word(_))
                && matches!(
                    token_ix.checked_sub(1).and_then(|k| tokens.get(k)),
                    Some(Token::Punctuation(_))
                )
                && codes
                    .iter()
                    .rev()
                    .find(|c| c.marker.is_none())
                    .is_some_and(|c| {
                        !c.is_boundary
                            && c.code != PHON_END_WORD
                            && phdata
                                .get(c.code)
                                .is_some_and(|ph| ph.typ != crate::phoneme::PH_PAUSE)
                    })
            {
                codes.push(PhonemeCode {
                    code: PHON_END_WORD,
                    is_boundary: true,
                    ..Default::default()
                });
            }
            // C's `SFLAG_DICTIONARY`: the *pronunciation* came from the list,
            // not merely some flags — a flags-only `$` entry still leaves the
            // rules to pronounce the word.
            let translation_given = entry.found_in_list;

            if let TranslateEntryKind::Letter(parts) = &entry.kind {
                let own = self.options.lang.clone();
                let close = |codes: &mut Vec<PhonemeCode>| {
                    codes.push(PhonemeCode {
                        marker: Some(CodeMarker::LangSwitch(own.clone())),
                        ..Default::default()
                    });
                };
                // After a word read in another language, a pause that opens
                // this one ends that word instead, ahead of its break, as C
                // writes it: `(en)'Eks_: h'aNgul`.
                // (Only for a letter C reads with a second translator, an
                // alphabet it switches for: `ა`, `ᄀ`.)
                let words_alphabet = parts.iter().any(|p| matches!(p, LetterPart::Raw { .. }));
                let was_open = open_switch.take().is_some();
                let after_switched_word =
                    (was_open | std::mem::take(&mut letter_ended_switched)) && words_alphabet;
                if was_open {
                    close(&mut codes);
                }
                let mut first_part = true;
                let local = |codes: &mut Vec<PhonemeCode>, ph: &[u8]| {
                    codes.extend(ph.iter().filter(|&&b| b != 0).map(|&code| PhonemeCode {
                        code,
                        ..Default::default()
                    }));
                };
                let mut switched = false;
                for part in parts {
                    let is_first = std::mem::replace(&mut first_part, false);
                    let lang = match part {
                        LetterPart::Key { lang, .. }
                        | LetterPart::Letter { lang, .. }
                        | LetterPart::Raw { lang, .. } => lang,
                        _ => &own,
                    };
                    if allow_switch && *lang != own && !switch_cache.contains_key(lang) {
                        if let Ok(tr) = Translator::new(lang, Some(&self.data_dir)) {
                            switch_cache.insert(lang.clone(), tr);
                        }
                    }
                    let found = match part {
                        LetterPart::Key { lang, key } => switch_cache
                            .get(lang)
                            .filter(|_| allow_switch)
                            .and_then(|tr| tr.key_phonemes(key)),
                        LetterPart::Letter { lang, letter, .. }
                        | LetterPart::Raw { lang, letter } => switch_cache
                            .get(lang)
                            .filter(|_| allow_switch)
                            .and_then(|tr| tr.letter_phonemes(*letter))
                            .filter(|ph| ph.first() != Some(&crate::phoneme::PHON_SWITCH)),
                        _ => None,
                    };
                    match (part, found) {
                        (LetterPart::Raw { .. }, found) => {
                            if switched {
                                close(&mut codes);
                                switched = false;
                            }
                            local(&mut codes, &found.unwrap_or_default());
                        }
                        (
                            LetterPart::Key { lang, .. } | LetterPart::Letter { lang, .. },
                            Some(ph),
                        ) => {
                            // `[phonPAUSE, phonSWITCH, …, phonSWITCH]`
                            if switched {
                                close(&mut codes);
                            }
                            if is_first && after_switched_word {
                                let at = codes
                                    .iter()
                                    .rposition(|c| {
                                        !(c.is_boundary && c.code == PHON_END_WORD)
                                            && c.marker.is_none()
                                    })
                                    .map_or(0, |k| k + 1);
                                codes.insert(
                                    at,
                                    PhonemeCode {
                                        code: crate::phoneme::PHON_PAUSE,
                                        ..Default::default()
                                    },
                                );
                            } else {
                                local(&mut codes, &[crate::phoneme::PHON_PAUSE]);
                            }
                            codes.push(PhonemeCode {
                                marker: Some(CodeMarker::LangSwitch(lang.clone())),
                                ..Default::default()
                            });
                            match switch_cache.get(lang) {
                                Some(tr) => codes.extend(tr.phonemes_after_programs(&ph)),
                                None => local(&mut codes, &ph),
                            }
                            switched = true;
                        }
                        (LetterPart::Key { .. }, None) => {}
                        (LetterPart::Local(ph) | LetterPart::Letter { otherwise: ph, .. }, _) => {
                            if switched {
                                close(&mut codes);
                                switched = false;
                            }
                            local(&mut codes, ph);
                        }
                        (LetterPart::Break, _) => {
                            if switched {
                                close(&mut codes);
                                switched = false;
                            }
                            codes.push(PhonemeCode {
                                code: 15,
                                is_boundary: true,
                                ..Default::default()
                            });
                        }
                    }
                }
                letter_ended_switched = switched;
                letter_ended_raw = matches!(parts.last(), Some(LetterPart::Raw { .. }));
                if switched {
                    close(&mut codes);
                }
                if !matches!(tokens.get(token_ix + 1), Some(Token::WordJoin)) {
                    codes.push(PhonemeCode {
                        code: 15,
                        is_boundary: true,
                        ..Default::default()
                    });
                }
                from_dict.resize(codes.len(), false);
                continue;
            }
            if matches!(entry.kind, TranslateEntryKind::LangSwitch) && allow_switch {
                letter_ended_switched = false;
                let target = switch_target(&entry.phonemes);
                let word = entry.word_lower.as_deref().unwrap_or("");
                if !word.is_empty() {
                    if !switch_cache.contains_key(&target) {
                        if let Ok(tr) = Translator::new(&target, Some(&self.data_dir)) {
                            switch_cache.insert(target.clone(), tr);
                        }
                    }
                    // A `_<char>` key is a character *name* to look up, not
                    // text to translate (`_@` tokenises as two symbols).
                    // (Or a lone symbol, which C's `LookupLetter` switches for:
                    // Bulgarian "мир &" spells it, `ampersand`.)
                    let lone_symbol = {
                        let mut it = word.chars();
                        it.next()
                            .filter(|c| !c.is_alphanumeric() && *c != '_')
                            .filter(|_| it.next().is_none())
                    };
                    let char_name = word
                        .strip_prefix('_')
                        .and_then(|rest| {
                            let mut it = rest.chars();
                            it.next().filter(|_| it.next().is_none())
                        })
                        .or(lone_symbol);
                    // `translate_to_codes_inner` with switching disabled: a
                    // switched word may not switch again.
                    // A longer `_` key is a dictionary entry in the other
                    // language (`_zh`, the name of an alphabet).
                    let key =
                        word.starts_with('_') && char_name.is_none() && word.chars().count() > 2;
                    let sw = match char_name {
                        None if key => switch_cache.get(&target).and_then(|tr| {
                            tr.key_phonemes(word).map(|ph| {
                                ph.iter()
                                    .filter(|&&b| b != 0)
                                    .map(|&b| PhonemeCode {
                                        code: b,
                                        ..Default::default()
                                    })
                                    .collect::<Vec<_>>()
                            })
                        }),
                        Some(c) => switch_cache.get(&target).and_then(|tr| {
                            // C spells a one-character word unless a letter of
                            // the same alphabet follows (`TranslateWord3`).
                            let next = tokens
                                .get(token_ix + 1..)
                                .unwrap_or_default()
                                .iter()
                                .find(|t| !matches!(t, Token::Space))
                                .and_then(|t| match t {
                                    Token::Word(w) => w.chars().next(),
                                    _ => None,
                                });
                            let spelled = !next.is_some_and(|n| {
                                c_is_alpha(n)
                                    && alphabet_from_char(n).map(|a| a.1)
                                        == alphabet_from_char(c).map(|a| a.1)
                            });
                            let Some(ph) = tr.switched_symbol_phonemes(c, spelled) else {
                                // (No name for it there: the character read as
                                // a word, French `©` is "copyright".)
                                if lone_symbol.is_none() {
                                    return None;
                                }
                                let saved =
                                    CLAUSE_REDUCE.with(|r| r.replace(Some(stress_opts.reduce)));
                                let codes = tr.translate_to_codes_inner(word, false).ok();
                                CLAUSE_REDUCE.with(|r| r.set(saved));
                                return codes;
                            };
                            // A name given as text is translated again, as C
                            // does, under the clause's `LOPT_REDUCE` (Russian
                            // `½` is `(en)a hˈɑːf`).
                            let text = tr
                                .replacement_text(&c.to_string())
                                .filter(|_| tr.replacement_text(&format!("_{c}")).is_none());
                            if let Some(text) = text {
                                let saved =
                                    CLAUSE_REDUCE.with(|r| r.replace(Some(stress_opts.reduce)));
                                let codes = tr.translate_to_codes_inner(&text, false).ok();
                                CLAUSE_REDUCE.with(|r| r.set(saved));
                                if let Some(codes) = codes {
                                    return Some(codes);
                                }
                            }
                            Some(
                                ph.iter()
                                    .filter(|&&b| b != 0)
                                    .map(|&b| PhonemeCode {
                                        code: b,
                                        ..Default::default()
                                    })
                                    .collect::<Vec<_>>(),
                            )
                        }),
                        None => {
                            // The switched words that follow are read with it,
                            // as one phrase: C's second translator sees the
                            // next words (English "the end" is `DI2; 'End`).
                            // A short all-capitals word in a lower-case clause
                            // is an abbreviation for C's `TranslateWord3`,
                            // spelled before the switch: Russian "XIV" is its
                            // three letters, each named in English.
                            let as_written = |k: usize, lower: &str| match tokens.get(k) {
                                Some(Token::Word(w))
                                    if is_spelled_abbreviation(&tokens, k, dict) =>
                                {
                                    w.chars()
                                        .map(|c| format!("_{}", c.to_ascii_lowercase()))
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                }
                                _ => lower.to_string(),
                            };
                            let mut phrase = as_written(token_ix, word);
                            let mut k = token_ix + 1;
                            // (A dot this language doesn't read joins them too.)
                            let silent_dot = |k: usize| {
                                matches!(tokens.get(k), Some(Token::Punctuation('.')))
                                    && entries.get(k).is_some_and(|e| {
                                        e.phonemes.iter().all(|&b| b == 0 || b == PHON_END_WORD)
                                    })
                            };
                            // Single letters joined by dots ("e.g.") are one
                            // word for C's `CheckDottedAbbrev`, before the
                            // switch: English is given "eg".
                            let single = |t: Option<&Token>| matches!(t, Some(Token::Word(w)) if is_single_letter_word(w));
                            if single(tokens.get(token_ix)) {
                                let mut letters = phrase.clone();
                                while single(tokens.get(k))
                                    && entries.get(k).is_some_and(|e| {
                                        e.kind == TranslateEntryKind::LangSwitch
                                            && switch_target(&e.phonemes) == target
                                    })
                                {
                                    if let Some(Token::Word(w)) = tokens.get(k) {
                                        letters.push_str(&w.to_lowercase());
                                    }
                                    k += 1;
                                }
                                // (C's word count is off by the dots it
                                // skipped: from the third letter on, the
                                // letters are spelled again.)
                                phrase.clone_from(&letters);
                                let again: String = letters.chars().skip(2).collect();
                                if !again.is_empty() {
                                    phrase.push_str(" _");
                                    phrase.push_str(&again);
                                }
                            }
                            while let (Some(Token::Space), Some(next))
                            | (Some(Token::Punctuation('.')), Some(next)) = (
                                tokens.get(k).filter(|_| {
                                    matches!(tokens.get(k), Some(Token::Space)) || silent_dot(k)
                                }),
                                entries.get(k + 1),
                            ) {
                                let same = next.kind == TranslateEntryKind::LangSwitch
                                    && switch_target(&next.phonemes) == target
                                    && !has_alphabet_name(&next.phonemes);
                                // (A symbol is its own word for C, not part of
                                // the phrase: Bulgarian "a & b" is `'eI and b'i:`.)
                                match next.word_lower.as_deref() {
                                    Some(w)
                                        if same
                                            && !w.starts_with('_')
                                            && w.chars().any(char::is_alphanumeric) =>
                                    {
                                        phrase.push(' ');
                                        phrase.push_str(&as_written(k + 1, w));
                                        k += 2;
                                    }
                                    _ => break,
                                }
                            }
                            switched_through = k;
                            // Does a letter of the same alphabet follow the
                            // phrase straight away?  Then C reads its last word
                            // as a word rather than a lone letter (`a ☺` is
                            // `a#`; `a ( b` keeps the letter `'eI`).
                            let last_char = phrase.chars().last();
                            let next_char = tokens
                                .get(k..)
                                .unwrap_or_default()
                                .iter()
                                .find(|t| !matches!(t, Token::Space))
                                .and_then(|t| match t {
                                    Token::Word(w) => w.chars().next(),
                                    _ => None,
                                });
                            let continues = next_char.is_some_and(|n| {
                                c_is_alpha(n)
                                    && last_char.map(|l| alphabet_from_char(l).map(|a| a.1))
                                        == Some(alphabet_from_char(n).map(|a| a.1))
                            });
                            let saved = CLAUSE_CONTINUES.with(|c| c.replace(continues));
                            let ends = !tokens
                                .get(k..)
                                .unwrap_or_default()
                                .iter()
                                .take_while(|t| !matches!(t, Token::ClauseBoundary(_)))
                                .any(|t| matches!(t, Token::Word(_) | Token::Number(_)));
                            let saved_ends =
                                PHRASE_ENDS_CLAUSE.with(|c| c.replace(ends && !continues));
                            let clause_from = tokens
                                .get(..token_ix)
                                .unwrap_or_default()
                                .iter()
                                .rposition(|t| matches!(t, Token::ClauseBoundary(_)))
                                .map_or(0, |b| b + 1);
                            let stressed = entries.get(clause_from..token_ix).unwrap_or_default().iter().any(|e| {
                                e.kind == TranslateEntryKind::Word
                                    && e.phonemes.iter().any(|&c| {
                                        c == crate::phoneme::PHON_STRESS_P || c == crate::phoneme::PHON_STRESS_P2
                                    })
                            });
                            let saved_stressed = STRESSED_BEFORE_PHRASE.with(|c| c.replace(stressed));
                            let saved_followed = PHRASE_FOLLOWED
                                .with(|c| c.replace(!ends && phrase.chars().count() > 1));
                            // (Its stress conditions use the clause's translator:
                            // Russian's `LOPT_REDUCE` makes the English `a#`
                            // count as stressed, so it is `a`.)
                            let saved_reduce =
                                CLAUSE_REDUCE.with(|r| r.replace(Some(stress_opts.reduce)));
                            // (Not for a lone letter, which C spells.)
                            let glued = match token_ix.checked_sub(1).and_then(|k| tokens.get(k)) {
                                Some(Token::Number(n)) if phrase.chars().count() > 1 => {
                                    Some(n.surface().to_string())
                                }
                                _ => None,
                            };
                            let saved_prev = SWITCH_PREV.with(|p| p.replace(glued));
                            let sw = switch_cache
                                .get(&target)
                                .and_then(|tr| tr.translate_to_codes_inner(&phrase, false).ok());
                            CLAUSE_REDUCE.with(|r| r.set(saved_reduce));
                            SWITCH_PREV.with(|p| p.replace(saved_prev));
                            CLAUSE_CONTINUES.with(|c| c.set(saved));
                            PHRASE_ENDS_CLAUSE.with(|c| c.set(saved_ends));
                            PHRASE_FOLLOWED.with(|c| c.set(saved_followed));
                            STRESSED_BEFORE_PHRASE.with(|c| c.set(saved_stressed));
                            sw
                        }
                    };
                    // A switch whose target says nothing leaves no markers:
                    // they would print as an empty `(en)(ru)` pair.
                    let said_nothing = sw.as_ref().is_some_and(|codes| {
                        !codes.iter().any(|c| !c.is_boundary && c.marker.is_none())
                    }) && char_name.is_none();
                    let sw = sw
                        .filter(|codes| codes.iter().any(|c| !c.is_boundary && c.marker.is_none()));
                    // (Read, and nothing to say: English `1_) st` in "1st".)
                    if said_nothing {
                        continue;
                    }
                    if let Some(sw) = sw {
                        // A dash before it is a pause in this language (C's
                        // `pre_pause_add`), between the switches.
                        let back = |n: usize| token_ix.checked_sub(n).and_then(|j| tokens.get(j));
                        let dash = matches!(back(1), Some(Token::Space))
                            && matches!(back(2), Some(Token::Punctuation('-')))
                            && matches!(back(3), Some(Token::Space));
                        if dash {
                            // (Before the word break, after the last word,
                            // and before the word end a number closes with.)
                            let at = codes
                                .iter()
                                .rposition(|c| {
                                    !c.is_boundary && c.marker.is_none() && c.code != PHON_END_WORD
                                })
                                .map_or(0, |i| i + 1);
                            // (A switch back already written moves with them.)
                            let closed = codes.get(at..).unwrap_or_default().iter().any(|c| {
                                matches!(&c.marker, Some(CodeMarker::LangSwitch(t)) if *t == self.options.lang)
                            });
                            if closed {
                                let mut k = at;
                                while k < codes.len() {
                                    if matches!(&codes.get(k).and_then(|c| c.marker.clone()), Some(CodeMarker::LangSwitch(t)) if *t == self.options.lang)
                                    {
                                        codes.remove(k);
                                    } else {
                                        k += 1;
                                    }
                                }
                            }
                            let mut ins = Vec::new();
                            if open_switch.take().is_some() || closed {
                                ins.push(PhonemeCode {
                                    marker: Some(CodeMarker::LangSwitch(self.options.lang.clone())),
                                    ..Default::default()
                                });
                            }
                            for _ in 0..2 {
                                ins.push(PhonemeCode {
                                    code: crate::phoneme::PHON_PAUSE,
                                    ..Default::default()
                                });
                            }
                            codes.splice(at..at, ins);
                        }
                        // An alphabet name, spoken in this language first.
                        if let Some(k) = entry
                            .phonemes
                            .iter()
                            .position(|&b| b == 0)
                            .filter(|_| has_alphabet_name(&entry.phonemes))
                        {
                            if open_switch.take().is_some() {
                                codes.push(PhonemeCode {
                                    marker: Some(CodeMarker::LangSwitch(self.options.lang.clone())),
                                    ..Default::default()
                                });
                            }
                            let before = entry.phonemes.get(k + 1..).unwrap_or_default();
                            let before = before.split(|&b| b == 0).next().unwrap_or_default();
                            codes.extend(before.iter().map(|&code| PhonemeCode {
                                code,
                                ..Default::default()
                            }));
                        }
                        if open_switch.as_deref() != Some(target.as_str()) {
                            let marker = PhonemeCode {
                                marker: Some(CodeMarker::LangSwitch(target.clone())),
                                ..Default::default()
                            };
                            // From one switched language straight to another,
                            // the marker follows the last word, before the gap.
                            let gap = codes
                                .iter()
                                .rev()
                                .take_while(|c| c.is_boundary && c.code == 15)
                                .count();
                            if (open_switch.is_some() || std::mem::take(&mut letter_ended_raw))
                                && gap > 0
                            {
                                let at = codes.len() - gap;
                                codes.insert(at, marker);
                            } else {
                                codes.push(marker);
                            }
                            open_switch = Some(target.clone());
                        }
                        for c in sw {
                            // Drop the inner clause boundary; this is one word
                            // inside the outer clause.
                            if c.is_boundary && c.code == 0 {
                                continue;
                            }
                            codes.push(c);
                        }
                        // What this language says after it (`[switch, lang, 0,
                        // before…, 0, after…]`): Hakka "Chinese letter".
                        let after = entry.phonemes.split(|&b| b == 0).nth(2).unwrap_or_default();
                        if !after.is_empty() {
                            open_switch = None;
                            codes.push(PhonemeCode {
                                marker: Some(CodeMarker::LangSwitch(self.options.lang.clone())),
                                ..Default::default()
                            });
                            codes.extend(after.iter().map(|&code| PhonemeCode {
                                code,
                                ..Default::default()
                            }));
                        }
                        // A hyphen joins it to the next word, as it does in
                        // the base language: `fr week-end` is
                        // `(en)w'i:k'End(fr)`, one word.
                        if !matches!(tokens.get(token_ix + 1), Some(Token::WordJoin)) {
                            codes.push(PhonemeCode {
                                code: 15,
                                is_boundary: true,
                                ..Default::default()
                            });
                        }
                        from_dict.resize(codes.len(), false);
                        continue;
                    }
                    // A character name the other language doesn't give either
                    // is silent, as in C (Hakka `©`).
                    if char_name.is_some() || key {
                        from_dict.resize(codes.len(), false);
                        continue;
                    }
                }
            }
            // An open switch stays open across the separators between words, so
            // a run of switched words shares one `(en)…(ru)` group like
            // upstream's; it closes at the next entry that actually speaks, at a
            // clause boundary — C ends the clause's phoneme list there, so
            // `ru "a , b"` is `(en)'eI(ru) (en)b'i:(ru)`, two groups, not one
            // spanning the comma — and at a bracket, whose pause belongs to the
            // base language (`(en)'eI(ru)_:_: (en)b'i:(ru)`).
            let bracket_here = matches!(token, Token::Punctuation(c) if is_bracket(*c))
                && entry.phonemes.is_empty();
            if !entry.phonemes.is_empty() || entry.kind == TranslateEntryKind::ClauseBoundary {
                letter_ended_switched = false;
                letter_ended_raw = false;
            }
            if open_switch.is_some()
                && (!entry.phonemes.is_empty()
                    || bracket_here
                    || entry.kind == TranslateEntryKind::ClauseBoundary)
            {
                open_switch = None;
                codes.push(PhonemeCode {
                    marker: Some(CodeMarker::LangSwitch(self.options.lang.clone())),
                    ..Default::default()
                });
            }

            match token {
                // An embedded command becomes a marker in the code stream, so
                // the synthesizer can change rate/pitch/amplitude *from here*
                // instead of for the whole utterance.
                Token::Embedded(cmd) => {
                    codes.push(PhonemeCode {
                        marker: Some(CodeMarker::Embedded(*cmd)),
                        ..Default::default()
                    });
                    from_dict.resize(codes.len(), false);
                }
                // Inline phonemes (`[[…]]`) carry pre-parsed phoneme codes and
                // spoken symbols (`&`→"and", `%`→"percent") carry name phonemes
                // in their entry (kind `Word`); emit them like any other word so
                // the `-x` / synthesis path renders them (parity with the
                // `text_to_ipa` / `--ipa` path).  Non-spoken punctuation has an
                // empty entry, so emitting it is a harmless no-op.
                Token::Word(_)
                | Token::Number(_)
                | Token::InlinePhonemes(_)
                | Token::Operator(_)
                | Token::Punctuation(_) => {
                    if let Token::Punctuation(c) | Token::Operator(c) = token {
                        // A bracket or quote is not spoken but does pause:
                        // `langopts.param[LOPT_BRACKET_PAUSE]` is 4, and C's
                        // pre-pause loop spends that as two `phonPAUSE`
                        // (`en "a ( b"` → `a#_:_: b'i:`).  The announced-bracket
                        // variant (`--punct`) is 2, i.e. one pause.  C builds the
                        // pause as the *next* word's pre-pause, so a bracket with
                        // no word after it in the clause — `(hello)` — pauses
                        // only on the opening one.
                        let word_follows = tokens
                            .get(token_ix + 1..)
                            .unwrap_or_default()
                            .iter()
                            .take_while(|t| !matches!(t, Token::ClauseBoundary(_)))
                            .zip(entries.get(token_ix + 1..).unwrap_or_default().iter())
                            .enumerate()
                            .any(|(k, (t, e))| {
                                // (Another bracket is a word of C's too, when
                                // spaced: `a ( (` pauses before the second.)
                                e.phonemes.iter().any(|&b| b != 0)
                                    || matches!(t, Token::Punctuation(b) if is_bracket(*b) && *b != '\'')
                                        && matches!(tokens.get(token_ix + k), Some(Token::Space))
                            });
                        // A bracket glued to one before it is part of that
                        // one's word, which pauses once: `((a))`.
                        let glued_bracket = matches!(
                            token_ix.checked_sub(1).and_then(|j| tokens.get(j)),
                            Some(Token::Punctuation(b)) if is_bracket(*b)
                        );
                        // An apostrophe's pause is C's `pre_pause_add`, which
                        // `words[0].pre_pause = 0` discards at the start of a
                        // clause ("don't add extra pause at beginning of
                        // clause") — so `'hello` has no pause while the curly
                        // `‘hello`, a real bracket, keeps its own.
                        let clause_initial_quote = *c == '\''
                            && !tokens
                                .get(..token_ix)
                                .unwrap_or_default()
                                .iter()
                                .rev()
                                .take_while(|t| !matches!(t, Token::ClauseBoundary(_)))
                                .zip(entries.get(..token_ix).unwrap_or_default().iter().rev())
                                .any(|(_, e)| e.phonemes.iter().any(|&b| b != 0));
                        if is_bracket(*c)
                            && !clause_initial_quote
                            && !glued_bracket
                            && entry.phonemes.is_empty()
                            && word_follows
                        {
                            // The pause joins the run of phonemes that is already
                            // open, so take back the word boundary the separator
                            // before the bracket pushed and put it back after.
                            // Markers (a closing language switch) can sit after
                            // that boundary, so look past them, and a switched
                            // word contributes one of its own on top of the
                            // separator's — take every trailing boundary and put
                            // a single one back after the pause.
                            // Nothing said yet in the clause (only silent
                            // marks before it: `; ? ( soweit`)?  Then the pause
                            // opens the next word.
                            let said = codes
                                .iter()
                                .rev()
                                .take_while(|k| !(k.is_boundary && k.code == 0))
                                .any(|k| {
                                    k.marker.is_none()
                                        && !k.is_boundary
                                        && k.code != PHON_END_WORD
                                        && k.code != crate::phoneme::PHON_PAUSE
                                });
                            let mut closed = false;
                            while let Some(i) = codes
                                .iter()
                                .rposition(|k| k.marker.is_none())
                                // …but not past the end of a clause, which
                                // stays in front of the bracket's pause.
                                // (A number's phonemes end in a word end of their own.)
                                .filter(|&i| {
                                    codes.boundary_at(i) && codes.code_at(i) != 0
                                        || codes.code_at(i) == PHON_END_WORD
                                })
                            {
                                codes.remove(i);
                                closed = true;
                            }
                            let (plain, announced) = self.options.bracket_pause;
                            // The pre-pause loop in C spends 2 per `phonPAUSE`.
                            let mut pauses = if punct_covers(&self.options, *c) {
                                announced / 2
                            } else {
                                plain / 2
                            };
                            // A dash before the bracket is the bracket word's
                            // own pre-pause, on top (`- " kmo` is `_:_:_:_:`).
                            let back =
                                |n: usize| token_ix.checked_sub(n).and_then(|j| tokens.get(j));
                            if matches!(back(1), Some(Token::Space))
                                && matches!(back(2), Some(Token::Punctuation('-')))
                                && matches!(back(3), Some(Token::Space))
                            {
                                pauses += 2;
                            }
                            // At the start of a clause the pause is the next
                            // word's own (`_:_:b'i:t`): no break follows it.
                            let opens_clause = !said;
                            if opens_clause {
                                closed = false;
                            }
                            if opens_clause && pauses > 0 {
                                pause_opens_word = true;
                            }
                            for _ in 0..pauses {
                                codes.push(PhonemeCode {
                                    code: crate::phoneme::PHON_PAUSE,
                                    ..Default::default()
                                });
                            }
                            if closed {
                                codes.push(PhonemeCode {
                                    code: 15,
                                    is_boundary: true,
                                    ..Default::default()
                                });
                            }
                            from_dict.resize(codes.len(), false);
                        }
                        codes.push(PhonemeCode {
                            marker: Some(CodeMarker::Punctuation(*c)),
                            ..Default::default()
                        });
                    }
                    // C flags the start of every word in the phoneme list.  A
                    // spoken symbol standing tight between two words —
                    // "cat.Dog" → "cat dot dog", "3.14" → "drei punkt vierzehn"
                    // — has no `Space` token on either side, so without this the
                    // three words ran together (`k'atd'0td'0g`).
                    // A `WordJoin` separator (a hyphen) means the two words are
                    // spoken as one, so no boundary goes between them.
                    let joined = matches!(
                        token_ix.checked_sub(1).and_then(|k| tokens.get(k)),
                        Some(Token::WordJoin)
                    ) || (0..token_ix)
                        .rev()
                        .find(|&k| !matches!(tokens.get(k), Some(Token::Space | Token::WordJoin)))
                        .and_then(|k| entries.get(k))
                        .is_some_and(|e| e.no_word_gap);
                    // Does the open run end in phonemes this word should be
                    // separated from?  A run of nothing but pauses doesn't count:
                    // that is a bracket's pre-pause, which belongs to the word
                    // after it as it does in C's phoneme list (`(hello)` is
                    // `_:_:h@l'oU`, one token).  A word whose own reading merely
                    // *ends* in a pause still does (`=` is `_:'i:kw@Lz_:`).
                    let runs_on = codes
                        .iter()
                        .rev()
                        .filter(|c| c.marker.is_none())
                        .take_while(|c| !c.is_boundary && c.code != crate::phoneme::PHON_END_WORD)
                        .any(|c| c.code != crate::phoneme::PHON_PAUSE);
                    // A bracket after the clause's last word starts no word of
                    // its own (C finds no source character for it, so its
                    // `sourceix` is 0): Ukrainian `"x"` ends `'Eks(uk)podv'i:jna`.
                    let trailing_bracket = is_trailing_bracket(&tokens, token_ix);
                    // (Nor the word end a number or a switched word closes with.)
                    if trailing_bracket && entry.phonemes.iter().any(|&b| b != 0) {
                        while let Some(i) = codes.iter().rposition(|k| k.marker.is_none()).filter(|&i| {
                            codes.boundary_at(i) && codes.code_at(i) != 0 || codes.code_at(i) == PHON_END_WORD
                        }) {
                            codes.remove(i);
                        }
                        from_dict.truncate(codes.len());
                    }
                    if runs_on
                        && !joined
                        && !trailing_bracket
                        && entry.phonemes.iter().any(|&b| b != 0)
                    {
                        codes.push(PhonemeCode {
                            code: 15,
                            is_boundary: true,
                            clause_char: None,
                            marker: None,
                        });
                    }
                    // `-k`: capital indication before a capitalised word.
                    //   1 → the `phonCAPITAL` sound, which belongs to the
                    //       *word* (C puts it at the head of its phoneme list,
                    //       hence `_#X1h@l'oU` with no gap), repeated for an
                    //       all-caps multi-letter word (C: FLAG_ALL_UPPER +
                    //       IsAlpha(word[1]));
                    //   2 → the spoken announcement, a word of its own;
                    //   ≥3 → raise the pitch for this word by that many Hz
                    //       (3 means C's default 20), restored afterwards.
                    let mut capital_pitch = 0;
                    if let Token::Word(w) = token {
                        if w.chars().next().is_some_and(char::is_uppercase) {
                            if !cap_codes.is_empty() {
                                let alpha = w.chars().filter(|c| c.is_alphabetic());
                                let all_upper = alpha.clone().count() > 1
                                    && alpha.clone().all(|c| c.is_uppercase());
                                let repeats = if self.options.capitals == 1 && all_upper {
                                    2
                                } else {
                                    1
                                };
                                for _ in 0..repeats {
                                    for &b in &cap_codes {
                                        if b != 0 {
                                            codes.push(PhonemeCode {
                                                code: b,
                                                is_boundary: false,
                                                clause_char: None,
                                                marker: None,
                                            });
                                        }
                                    }
                                }
                                if self.options.capitals != 1 {
                                    codes.push(PhonemeCode {
                                        code: 15,
                                        is_boundary: true,
                                        clause_char: None,
                                        marker: None,
                                    }); // END_WORD
                                }
                            } else if self.options.capitals >= 3 {
                                capital_pitch = if self.options.capitals == 3 {
                                    20
                                } else {
                                    self.options.capitals as i32
                                };
                                codes.push(PhonemeCode {
                                    marker: Some(CodeMarker::Embedded(EmbeddedCmd {
                                        letter: 'P',
                                        value: capital_pitch,
                                        relative: 1,
                                    })),
                                    ..Default::default()
                                });
                            }
                        }
                    }
                    // `$brk` / `$pause` put a pause *before* the word (C's
                    // `pre_pause`): one `phonPAUSE_NOLINK` for `$brk`, two
                    // `phonPAUSE` for `$pause`.  Swahili's "za" is `$u $brk`,
                    // which is where `hab'aRi_! za` gets its break.
                    if !entry.phonemes.is_empty() {
                        use crate::dictionary::{FLAG_PAUSE1, FLAG_PREPAUSE};
                        let mut pre_pause: i32 = 0;
                        // C: "don't use $brk pause before the last word of a
                        // sentence".
                        let more_words = tokens
                            .get(token_ix + 1..)
                            .unwrap_or_default()
                            .iter()
                            .take_while(|t| !matches!(t, Token::ClauseBoundary(_)))
                            .any(|t| matches!(t, Token::Word(_) | Token::Number(_)));
                        if entry.dict_flags & FLAG_PAUSE1 != 0 && more_words {
                            pre_pause = 1;
                        }
                        // C additionally skips `$pause` at either end of the
                        // clause and right after its first word, so it needs at
                        // least two words before it and one after.
                        // (C's words include brackets and symbols, and a
                        // `$pause` is ignored for the two words after one.)
                        let timed_out = prepause_timeout == 0;
                        if entry.dict_flags & FLAG_PREPAUSE != 0 && timed_out {
                            let is_word = |t: &Token| {
                                matches!(
                                    t,
                                    Token::Word(_)
                                        | Token::Number(_)
                                        | Token::InlinePhonemes(_)
                                        | Token::Operator(_)
                                        | Token::Punctuation(_)
                                )
                            };
                            let before = tokens
                                .get(..token_ix)
                                .unwrap_or_default()
                                .iter()
                                .rev()
                                .take_while(|t| !matches!(t, Token::ClauseBoundary(_)))
                                .filter(|t| is_word(t))
                                .count();
                            // (Brackets after the clause's last word leave it
                            // the last: `ab díky )` has no pause.)
                            let after = tokens
                                .get(token_ix + 1..)
                                .unwrap_or_default()
                                .iter()
                                .take_while(|t| !matches!(t, Token::ClauseBoundary(_)))
                                .filter(|t| is_word(t) && !matches!(t, Token::Punctuation(c) if is_bracket(*c)))
                                .count();
                            if before >= 2 && after >= 1 {
                                pre_pause = 4;
                                prepause_timeout = 3;
                            }
                        }
                        // A dash between two spaces, or a double hyphen, is
                        // a pause before the next word (C's `pre_pause_add`).
                        let back = |k: usize| token_ix.checked_sub(k).and_then(|j| tokens.get(j));
                        let double = matches!(
                            (back(1), back(2)),
                            (Some(Token::Punctuation('-')), Some(Token::Punctuation('-')))
                        );
                        let dash = double
                            || matches!(back(1), Some(Token::Space))
                                && (matches!(
                                    (back(2), back(3)),
                                    (Some(Token::Punctuation('-')), Some(Token::Space))
                                ) || matches!(
                                    (back(2), back(3)),
                                    (Some(Token::Punctuation('-')), Some(Token::Punctuation('-')))
                                ));
                        let first_word = !tokens
                            .get(..token_ix)
                            .unwrap_or_default()
                            .iter()
                            .rev()
                            .take_while(|t| !matches!(t, Token::ClauseBoundary(_)))
                            .any(|t| {
                                // (A symbol or bracket is a word of C's too:
                                // `/ - go` is `sl'aS_:_: g'oU`.)
                                matches!(
                                    t,
                                    Token::Word(_)
                                        | Token::Number(_)
                                        | Token::Operator(_)
                                        | Token::InlinePhonemes(_)
                                ) || matches!(t, Token::Punctuation(c) if *c != '-')
                            });
                        if dash && !first_word {
                            pre_pause = pre_pause.max(4);
                        }
                        // C keeps the largest of the pre-pauses: a bracket's
                        // pauses already written count (`) - sakker` is
                        // `_:_:`, as is `( पर` with `$pause`).
                        let written = codes
                            .iter()
                            .rev()
                            .filter(|c| {
                                c.marker.is_none() && !(c.is_boundary && c.code == PHON_END_WORD)
                            })
                            .take_while(|c| !c.is_boundary && c.code == crate::phoneme::PHON_PAUSE)
                            .count();
                        pre_pause = pre_pause.saturating_sub(2 * written as i32);
                        // The pause goes *before* the word separator, so it
                        // reads as part of the previous word (`hab'aRi_! za`).
                        let mut at = codes
                            .iter()
                            .rposition(|c| {
                                !c.is_boundary
                                    && c.marker.is_none()
                                    && c.code != crate::phoneme::PHON_END_WORD
                            })
                            .map_or(0, |i| i + 1);
                        // Nothing said yet in this clause: the pause opens the
                        // word, after the clause's break and with no word
                        // break of its own (`/ - 12` is `_:_:dv'ana:st;`).
                        let clause_start = codes
                            .iter()
                            .rposition(|c| c.is_boundary && c.code == 0 && c.clause_char.is_some())
                            .map_or(0, |i| i + 1);
                        let said = codes.get(clause_start..).unwrap_or_default().iter().any(|c| {
                            c.marker.is_none()
                                && !c.is_boundary
                                && !matches!(
                                    c.code,
                                    PHON_END_WORD
                                        | crate::phoneme::PHON_PAUSE
                                        | crate::phoneme::PHON_PAUSE_NOLINK
                                )
                        });
                        if (pre_pause > 0 || written > 0) && !said {
                            while codes.len() > clause_start
                                && codes.last().is_some_and(|c| {
                                    c.marker.is_none()
                                        && (c.is_boundary && c.code != 0 || c.code == PHON_END_WORD)
                                })
                            {
                                codes.pop();
                            }
                            from_dict.truncate(codes.len());
                            at = codes.len().max(clause_start);
                        }
                        // (After the switch back from a word in another
                        // language: `(en)h@l'oU(ru)_:_: m;'ir`.)
                        while codes
                            .get(at)
                            .is_some_and(|c| matches!(c.marker, Some(CodeMarker::LangSwitch(_))))
                        {
                            at += 1;
                        }
                        if pre_pause > 0 {
                            if let Some(k) = (at..codes.len()).find(|&k| {
                                matches!(&codes.get(k).and_then(|c| c.marker.clone()), Some(CodeMarker::LangSwitch(t)) if *t == self.options.lang)
                            }) {
                                let marker = codes.remove(k);
                                codes.insert(at, marker);
                                at += 1;
                            }
                        }
                        if pre_pause > 0 && open_switch.take().is_some() {
                            codes.insert(
                                at,
                                PhonemeCode {
                                    marker: Some(CodeMarker::LangSwitch(self.options.lang.clone())),
                                    ..Default::default()
                                },
                            );
                            at += 1;
                        }
                        let mut n = 0;
                        while pre_pause > 0 {
                            let code = if pre_pause > 1 {
                                pre_pause -= 2;
                                crate::phoneme::PHON_PAUSE
                            } else {
                                pre_pause -= 1;
                                crate::phoneme::PHON_PAUSE_NOLINK
                            };
                            codes.insert(
                                at + n,
                                PhonemeCode {
                                    code,
                                    is_boundary: false,
                                    clause_char: None,
                                    marker: None,
                                },
                            );
                            n += 1;
                        }
                        if n > 0 {
                            from_dict.resize(codes.len(), false);
                        }
                        // A pause before the word makes C forget the word
                        // before it (`prev_dict_flags`), and so its `$double`:
                        // Finnish "syödä tai x" is `tai`.
                        if n > 0 || written > 0 {
                            double_next = false;
                        }
                    }
                    // `$double` or `phonX1` on the previous word doubles this
                    // one's first consonant (Finnish "purra tähän").
                    let double_first = std::mem::take(&mut double_next);
                    let first_at = codes.len();
                    double_next = entry.dict_flags & FLAG_DOUBLING != 0
                        || entry.phonemes.contains(&crate::phoneme::PHON_X1);
                    // (Straight after a pause that opens it, the word has no
                    // break before it: `_:_:f'aIv`.)
                    let after_pause = codes
                        .iter()
                        .rev()
                        .find(|c| c.marker.is_none())
                        .is_some_and(|c| !c.is_boundary && c.code == crate::phoneme::PHON_PAUSE);
                    let lead = if after_pause {
                        entry
                            .phonemes
                            .iter()
                            .take_while(|&&b| b == PHON_END_WORD)
                            .count()
                    } else {
                        0
                    };
                    let mut bytes = entry.phonemes.iter().copied().skip(lead);
                    while let Some(b) = bytes.next() {
                        // A switch left in the word (see the replacement
                        // reading): the byte after it is the table.
                        if b == crate::phoneme::PHON_SWITCH {
                            let Some(ix) = bytes.next() else { break };
                            if let Some(t) = phdata.tables.get(usize::from(ix)) {
                                codes.push(PhonemeCode {
                                    marker: Some(CodeMarker::LangSwitch(t.name.clone())),
                                    ..Default::default()
                                });
                            }
                            continue;
                        }
                        // `phonX1` is a marker, not a sound: C reads it as
                        // "double the next word's initial consonant"
                        // (`FLAG_DOUBLING`) and does *not* put it in the phoneme
                        // list.  Emitting it printed a stray `_X1` (Finnish
                        // `k'aikIlle_X1`) and spoke it.
                        if b != 0 && b != crate::phoneme::PHON_X1 {
                            codes.push(PhonemeCode {
                                code: b,
                                is_boundary: false,
                                clause_char: None,
                                marker: None,
                            });
                        }
                    }
                    if double_first {
                        let first = codes
                            .get(first_at..)
                            .unwrap_or_default()
                            .iter()
                            .position(|c| c.code > 8 && c.code != crate::phoneme::PHON_END_WORD);
                        if let Some(k) = first.map(|k| first_at + k) {
                            let consonant = phdata
                                .get(codes.code_at(k))
                                .is_some_and(|ph| !matches!(ph.typ, 0..=2));
                            if consonant {
                                codes.insert(
                                    k + 1,
                                    PhonemeCode {
                                        code: crate::phoneme::PHON_LENGTHEN,
                                        ..Default::default()
                                    },
                                );
                            }
                        }
                    }
                    // The pitch raise for a capital is undone after the word,
                    // carried — as in C — on a `phonPAUSE_SHORT`, which is why
                    // `-k 20 "Hello World"` prints `h@l'oU_ w'3:ld_`.
                    if capital_pitch > 0 {
                        codes.push(PhonemeCode {
                            marker: Some(CodeMarker::Embedded(EmbeddedCmd {
                                letter: 'P',
                                value: capital_pitch,
                                relative: -1,
                            })),
                            ..Default::default()
                        });
                        codes.push(PhonemeCode {
                            code: crate::phoneme::PHON_PAUSE_SHORT,
                            is_boundary: false,
                            clause_char: None,
                            marker: None,
                        });
                    }
                    // `-g` word gap: a pause phoneme (`_`) after each word.  (Not
                    // in a word read for another language's clause, whose own
                    // gaps are the ones that apply: Latin in Afrikaans.)
                    if self.options.word_gap > 0 && !entry.phonemes.is_empty() && allow_switch {
                        let pause = phdata.lookup_phoneme("_");
                        if pause != 0 {
                            codes.push(PhonemeCode {
                                code: pause,
                                is_boundary: false,
                                clause_char: None,
                                marker: None,
                            });
                        }
                    }
                    // (The language's own inter-word gap is inserted later, by
                    // `insert_word_gaps`, which needs the finished code list.)
                }
                // A hyphen-joined pair is spoken as one word: no boundary, no
                // space in the output.  (A number's phonemes end in one.)
                Token::WordJoin => {
                    if matches!(
                        token_ix.checked_sub(1).and_then(|k| tokens.get(k)),
                        Some(Token::Number(_))
                    ) {
                        while codes
                            .last()
                            .is_some_and(|c| c.code == PHON_END_WORD && c.marker.is_none())
                        {
                            codes.pop();
                        }
                    }
                }
                Token::Space | Token::Dot => {
                    // A sandhi-joined pair is one word, so the separator between
                    // its halves emits no boundary either.
                    let joined = token_ix
                        .checked_sub(1)
                        .and_then(|k| entries.get(k))
                        .is_some_and(|e| e.no_word_gap)
                        // (Kept past a spaced dash, which says nothing:
                        // `" - $5` is `_:_:dOl'ar`.)
                        || if matches!(tokens.get(token_ix + 1), Some(Token::Punctuation('-'))) {
                            pause_opens_word
                        } else {
                            std::mem::take(&mut pause_opens_word)
                        }
                        || (is_trailing_bracket(&tokens, token_ix + 1)
                            && entries.get(token_ix + 1).is_some_and(|e| e.phonemes.iter().any(|&b| b != 0)));
                    if !joined {
                        codes.push(PhonemeCode {
                            code: 15,
                            is_boundary: true,
                            clause_char: None,
                            marker: None,
                        }); // END_WORD
                    }
                }
                Token::ClauseBoundary(punct) => {
                    if entry.phonemes.is_empty() {
                        codes.push(PhonemeCode {
                            code: 0,
                            is_boundary: true,
                            clause_char: Some(*punct),
                            marker: None,
                        }); // pause + clause
                    } else {
                        // `--punct`: the boundary became a spoken name.  It
                        // absorbed its surrounding whitespace, so bracket it with
                        // END_WORD markers to keep the name a separate word.
                        codes.push(PhonemeCode {
                            code: 15,
                            is_boundary: true,
                            clause_char: None,
                            marker: None,
                        });
                        for &b in &entry.phonemes {
                            if b != 0 {
                                codes.push(PhonemeCode {
                                    code: b,
                                    is_boundary: false,
                                    clause_char: None,
                                    marker: None,
                                });
                            }
                        }
                        codes.push(PhonemeCode {
                            code: 15,
                            is_boundary: true,
                            clause_char: None,
                            marker: None,
                        });
                    }
                }
            }
            // Positions added during this iteration inherit the entry's flag.
            from_dict.resize(codes.len(), translation_given);
        }

        // A switch left open at the end of the text still closes, so the
        // output reads `(en)h@l'oU(ru)` rather than trailing off.
        if open_switch.take().is_some() {
            codes.push(PhonemeCode {
                marker: Some(CodeMarker::LangSwitch(self.options.lang.clone())),
                ..Default::default()
            });
            from_dict.resize(codes.len(), false);
        }

        // Which source token owns which codes, in *boundary* coordinates: the
        // post-passes below insert and delete phonemes, so an index into
        // `codes` would not survive them — the number of word/clause boundaries
        // before a code does, since no pass adds or removes one.
        let mut groups: Vec<(String, usize)> = Vec::new();
        if tracking {
            owners.resize(codes.len(), owner_now);
            let mut boundaries_before = Vec::with_capacity(codes.len() + 1);
            let mut n = 0usize;
            boundaries_before.push(0);
            for c in &codes {
                if c.is_boundary {
                    n += 1;
                }
                boundaries_before.push(n);
            }
            // The first *sounding* code: a token's own codes often open with the
            // word boundary the separator before it pushed, and counting that
            // boundary as "not yet passed" would start the span one word early.
            let mut first: Vec<Option<usize>> = vec![None; tokens.len()];
            for (i, &o) in owners.iter().enumerate() {
                if !codes.boundary_at(i) && o < tokens.len() && first[o].is_none() {
                    first.put(o, Some(i));
                }
            }
            for (ix, token) in tokens.iter().enumerate() {
                let Some(f) = first.at(ix) else { continue };
                let text = token_surface(token, &self.options.number_grammar);
                if !text.is_empty() {
                    groups.push((text, boundaries_before.at(f)));
                }
            }
        }

        // Regressive voicing assimilation runs over the whole clause, before the
        // phoneme programs — C calls it once on `ph_list2` in `MakePhonemeList`,
        // so a `;` that is about to be deleted still separates the consonants.
        if stress_opts.regressive_voicing != 0 {
            set_regressive_voicing(
                &mut codes,
                phdata,
                stress_opts.regressive_voicing,
                &self.options.lang,
            );
        }

        // The post-passes resolve codes against *this* clause's phoneme table, so
        // they must skip anything inside a `phonSWITCH` region — those codes
        // belong to the switched language's table, and rewriting them with this
        // one turned Arabic `H` into `d`.
        // The voice's own `replace` substitutions run before the phoneme
        // programs (C's `SubstitutePhonemes` in `MakePhonemeList`): Russian
        // `replace 03 a a#` is why an unstressed word ends in `a#`.
        let replacements: Vec<(u8, u8, u8)> =
            crate::voices::voice_replacements(&self.data_dir, lang)
                .into_iter()
                .filter_map(|(f, old, new)| {
                    let o = phdata.lookup_phoneme(&old);
                    let n = if new == "NULL" {
                        0
                    } else {
                        phdata.lookup_phoneme(&new)
                    };
                    (o != 0).then_some((f, o, n))
                })
                .collect();
        if allow_switch {
            collapse_language_switches(&mut codes, &self.options.lang);
        }
        let mut out: Vec<PhonemeCode> = Vec::with_capacity(codes.len());
        let mut own: Vec<PhonemeCode> = Vec::new();
        let mut own_flags: Vec<bool> = Vec::new();
        let mut in_switch = false;
        let finish =
            |own: &mut Vec<PhonemeCode>, own_flags: &mut Vec<bool>, out: &mut Vec<PhonemeCode>| {
                if own.is_empty() {
                    return;
                }
                own_flags.resize(own.len(), false);
                // (A word read for another language's clause is in that
                // voice, whose own `replace` lines are the ones that apply.)
                let replacements: &[(u8, u8, u8)] = if allow_switch { &replacements } else { &[] };
                for i in apply_voice_replacements(own, phdata, replacements)
                    .into_iter()
                    .rev()
                {
                    if i < own_flags.len() {
                        own_flags.remove(i);
                    }
                }
                // Before the phoneme programs run, as in C's `MakePhonemeList`.
                double_lengthened_consonants(own, phdata);
                // (A word read for another clause uses that clause's option.)
                let reduce = if allow_switch {
                    stress_opts.reduce
                } else {
                    CLAUSE_REDUCE
                        .with(std::cell::Cell::get)
                        .unwrap_or(stress_opts.reduce)
                };
                if stress_opts.stress_flags & 0x08 != 0 {
                    diminish_next_unstressed(own, phdata);
                    own_flags.resize(own.len(), false);
                }
                resolve_virtual_phonemes_in_codes(own, phdata, own_flags, reduce);
                apply_list_editing_instructions(
                    own,
                    phdata,
                    own_flags,
                    reduce,
                    if self.options.word_gap == 0 {
                        stress_opts.vowel_pause
                    } else {
                        0
                    },
                    if self.options.word_gap == 0 {
                        stress_opts.word_gap
                    } else {
                        0
                    },
                );
                drop_vowelless_stress(own, phdata);
                syllabic_after_vowel(own, phdata);
                if primary_bcp47_subtag(lang) == "fr" {
                    drop_unsurfaced_liaison(own, phdata);
                }
                // (The marker that opened a run inside a word is out already.)
                if matches!(
                    own.first().and_then(|c| c.marker.as_ref()),
                    Some(CodeMarker::LangSwitch(_))
                ) {
                    own.remove(0);
                }
                out.append(own);
                own_flags.clear();
            };
        for (ix, c) in codes.into_iter().enumerate() {
            if let Some(CodeMarker::LangSwitch(target)) = &c.marker {
                finish(&mut own, &mut own_flags, &mut out);
                in_switch = *target != self.options.lang;
                // Back inside a word: the run that follows doesn't start one.
                let mid_word = !in_switch
                    && out.last().is_some_and(|p| {
                        !p.is_boundary && p.marker.is_none() && p.code != PHON_END_WORD
                    });
                out.push(c.clone());
                if mid_word {
                    own.push(c);
                    own_flags.push(false);
                }
                continue;
            }
            if in_switch {
                out.push(c);
            } else {
                own_flags.push(from_dict.get(ix).copied().unwrap_or(false));
                own.push(c);
            }
        }
        finish(&mut own, &mut own_flags, &mut out);

        // …and so does the inter-word pause insertion.  (A word read for
        // another language's clause takes that clause's gaps, which it inserts.)
        if self.options.word_gap == 0 && allow_switch {
            // A word in another language has the types of its own table.
            let switched_type = |target: &str, code: u8| {
                switch_cache
                    .get(target)
                    .and_then(|tr| tr.data().ok())
                    .map(|data| data.phdata.get(code).map(|ph| ph.typ))
                    .unwrap_or_else(|| embedded_table_type(target, code))
            };
            insert_word_gaps(
                &mut out,
                phdata,
                stress_opts.vowel_pause,
                stress_opts.word_gap,
                lang,
                &switched_type,
            );
        }

        // The default tones, over the whole clause: a word in another language
        // takes them too (Hakka "(en)tS'aI1ni:1z").
        if allow_switch {
            let switched_vowel = |target: &str, code: u8| {
                switch_cache
                    .get(target)
                    .and_then(|tr| tr.data().ok())
                    .map(|data| data.phdata.get(code).map(|ph| ph.typ))
                    .unwrap_or_else(|| embedded_table_type(target, code))
                    == Some(crate::phoneme::PH_VOWEL)
            };
            assign_default_tones_in_codes(&mut out, phdata, lang, &switched_vowel);
        }

        // (C does this in the pass that gives a clause its tune, which a tone
        // language doesn't have.)
        // A word read in another language for a clause belongs to that clause,
        // which does this itself.
        if allow_switch
            && !matches!(
                primary_bcp47_subtag(lang),
                "cmn" | "hak" | "shn" | "vi" | "yue" | "zh"
            )
        {
            let switched_vowel = |target: &str, code: u8| {
                switch_cache
                    .get(target)
                    .and_then(|tr| tr.data().ok())
                    .map(|data| data.phdata.get(code).map(|ph| ph.typ))
                    .unwrap_or_else(|| embedded_table_type(target, code))
                    == Some(crate::phoneme::PH_VOWEL)
            };
            promote_stress_in_unstressed_clauses(
                &mut out,
                phdata,
                lang,
                stress_opts.stress_flags,
                &switched_vowel,
                &no_tone_clauses,
            );
        }

        // Boundary coordinates back into indices.  A token's codes run from its
        // own first code to the *next* token's, so the separator's boundary and
        // any pause the passes added stay with the word they follow.
        if let Some(track) = track {
            let positions: Vec<usize> = out
                .iter()
                .enumerate()
                .filter(|(_, c)| c.is_boundary)
                .map(|(i, _)| i)
                .collect();
            let start_of = |b: usize| -> usize {
                if b == 0 {
                    0
                } else {
                    positions.get(b - 1).map(|&i| i + 1).unwrap_or(out.len())
                }
            };
            let starts: Vec<usize> = groups.iter().map(|&(_, b)| start_of(b)).collect();
            for (i, (text, _)) in groups.into_iter().enumerate() {
                let start = starts.at(i);
                let end = starts.get(i + 1).copied().unwrap_or(out.len()).max(start);
                track.push(TokenCodes {
                    text,
                    codes: start..end,
                });
            }
        }
        Ok(out)
    }
}

/// A source word and the phonemes it was rendered as
/// ([`Translator::text_to_ipa_words`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordPhonemes {
    /// The word as it reached translation.
    pub word: String,
    /// Byte offset of the word in the input text (`0` with `length == 0` when
    /// the tokenizer rewrote it and it can no longer be located).
    pub offset: usize,
    /// Its byte length in the input text, or `0` when unknown.
    pub length: usize,
    /// The IPA for this word — the slice of the clause's own rendering.
    pub phonemes: String,
}

/// Locate `needle` in `text` at or after `from`, exactly or ignoring ASCII case
/// (a capitalised sentence opener is the common case).  `length == 0` means it
/// could not be found, which happens for tokens the rewriting passes created.
fn find_token_span(text: &str, needle: &str, from: usize) -> (usize, usize) {
    let from = from.min(text.len());
    if needle.is_empty() || !text.is_char_boundary(from) {
        return (from, 0);
    }
    let hay = &text[from..];
    if let Some(ix) = hay.find(needle) {
        return (from + ix, needle.len());
    }
    let n = needle.len();
    for (i, _) in hay.char_indices() {
        if i + n <= hay.len()
            && hay.is_char_boundary(i + n)
            && hay[i..i + n].eq_ignore_ascii_case(needle)
        {
            return (from + i, n);
        }
    }
    (from, 0)
}

/// One source token and the span of phoneme codes it was translated into.
pub(crate) struct TokenCodes {
    /// The token's text as it appeared (after the number/abbreviation rewrites,
    /// which is how the tokens reach translation).
    pub(crate) text: String,
    /// Its codes in the finished clause.
    pub(crate) codes: std::ops::Range<usize>,
}

/// The text a token stands for, or `""` for one that is not part of the source
/// text (a separator or an embedded command).
fn token_surface(token: &Token, grammar: &NumberGrammar) -> String {
    match token {
        Token::Word(w) => w.clone(),
        // `NumberToken::surface` writes a decimal point; the source may have
        // used the language's own separator (`de "3,14"`).
        Token::Number(NumberToken::Decimal {
            integer,
            fractional,
        }) => {
            format!("{integer}{}{fractional}", grammar.decimal_separator)
        }
        Token::Number(n) => n.surface(),
        Token::InlinePhonemes(p) => format!("[[{p}]]"),
        Token::Operator(c) | Token::Punctuation(c) | Token::ClauseBoundary(c) => c.to_string(),
        Token::Space | Token::Dot | Token::WordJoin | Token::Embedded(_) => String::new(),
    }
}

/// Insert the inter-word pause phonemes a language asks for — the
/// `(plist3+1)->sourceix != 0` block of `MakePhonemeList` (phonemelist.c).
///
/// `vowel_pause` is `langopts.vowel_pause`: bit 0-1 chooses the pause between
/// two vowels across a word boundary (Italian's `1` puts a very short one
/// between "casa" and "è"), bits 2-3 one before *any* vowel-initial word,
/// bit 8 one before a *stressed* vowel, bit 9 one after a word ending in a
/// consonant.  `word_gap & 7` then selects a minimum gap from
/// `pause_phonemes[]` for every boundary.
fn insert_word_gaps(
    codes: &mut Vec<PhonemeCode>,
    phdata: &PhonemeData,
    vowel_pause: u32,
    word_gap: u8,
    lang: &str,
    switched_type: &dyn Fn(&str, u8) -> Option<u8>,
) {
    use crate::phoneme::PHON_END_WORD;
    const PH_PAUSE: u8 = 0;
    if vowel_pause == 0 && word_gap & 7 == 0 {
        return;
    }

    let is_sound = |c: &PhonemeCode| {
        c.marker.is_none() && !c.is_boundary && c.code > 8 && c.code != PHON_END_WORD
    };
    let real: Vec<usize> = codes
        .iter()
        .enumerate()
        .filter(|(_, c)| is_sound(c))
        .map(|(i, _)| i)
        .collect();
    let (level, _) = stress_levels(codes, phdata);
    let starts = word_starts(codes, &real);

    let mut table: Vec<Option<String>> = Vec::with_capacity(codes.len());
    let mut current: Option<String> = None;
    for c in codes.iter() {
        if let Some(CodeMarker::LangSwitch(t)) = &c.marker {
            current = (t != lang).then(|| t.clone());
        }
        table.push(current.clone());
    }
    let typ_at = |ix: usize, code: u8| match table.get(ix).and_then(|t| t.as_deref()) {
        Some(t) => switched_type(t, code).unwrap_or(PH_PAUSE),
        None => phdata.get(code).map(|p| p.typ).unwrap_or(PH_PAUSE),
    };
    let mut inserts: Vec<(usize, u8)> = Vec::new();
    // Every word start is a boundary, and so is the end of a clause: C's phoneme
    // list ends with a pause phoneme that carries `sourceix`, which is why
    // Chinese gets a trailing `_|` after its last word as well.
    for n in 1..=real.len() {
        let at_clause_end = n == real.len()
            || codes
                .get(real.at(n - 1) + 1..real.at(n))
                .unwrap_or_default()
                .iter()
                .any(|c| c.is_boundary && c.code == 0);
        if !at_clause_end && !starts.at(n) {
            continue; // not a word boundary
        }
        let prev = codes.code_at(real.at(n - 1));
        let next = if n < real.len() && !at_clause_end {
            codes.code_at(real.at(n))
        } else {
            0
        };
        // (A tone isn't a phoneme of C's list: the sound before the gap is
        // the vowel it belongs to — Vietnamese `s'i4_ 'i7`.)
        let mut k = n - 1;
        while k > 0
            && table.get(real.at(k)).is_some_and(Option::is_none)
            && is_tone_phoneme(codes.code_at(real.at(k)), phdata)
        {
            k -= 1;
        }
        let mut pt = typ_at(real.at(k), codes.code_at(real.at(k)));
        let _ = prev;
        let mut nt = if next == 0 {
            PH_PAUSE
        } else {
            typ_at(real.at(n), next)
        };
        // A change of language between the words is a `phonSWITCH` in C's
        // list, which is the sound either side of the gap: after a word in
        // another language C sees no vowel (`(en)nj'u:(it) 'ab`), before one
        // no vowel to come.
        if n < real.len() && table.get(real.at(n - 1)) != table.get(real.at(n)) {
            if table.get(real.at(n - 1)).is_some_and(Option::is_some) {
                pt = PH_PAUSE;
            } else {
                nt = PH_PAUSE;
            }
        }
        let next_stressed = n < real.len() && level.get(real.at(n)).is_some_and(|&l| l >= 4);
        let insert = word_gap_pause(pt, nt, next_stressed, vowel_pause, word_gap);
        if insert != 0 && phdata.get(insert).is_some() {
            // The gap belongs to the *previous* word — upstream prints
            // `bUondZ'O@-*no_| a`, with the pause before the word separator —
            // but after the marker that ends a word in another language.
            inserts.push((real.at(n - 1) + 1, insert));
        }
    }

    for (at, code) in inserts.into_iter().rev() {
        // …but after the marker that ends a word in another language, which C
        // writes straight after the word: `(en)h@1l'oU1(yue)_|`.
        let close = codes.get(at..).and_then(|rest| {
            let k = rest
                .iter()
                .position(|c| !(c.is_boundary && c.code == PHON_END_WORD))?;
            // (Or one that opens the next word's language straight after
            // this word, before its break: `'a55(en)_| w'aI55`.)
            rest.get(k)
                .is_some_and(
                    |c| matches!(&c.marker, Some(CodeMarker::LangSwitch(t)) if t == lang || k == 0),
                )
                .then_some(at + k)
        });
        if let Some(m) = close.filter(|&m| m > at) {
            let marker = codes.remove(m);
            codes.insert(at, marker);
        }
        let at = if close.is_some() { at + 1 } else { at };
        codes.insert(
            at,
            PhonemeCode {
                code,
                is_boundary: false,
                clause_char: None,
                marker: None,
            },
        );
    }
}

/// The pause C's `MakePhonemeList` puts between two words, by the types of
/// the phonemes either side (`vowel_pause`, `word_gap`), or 0.
fn word_gap_pause(pt: u8, nt: u8, next_stressed: bool, vowel_pause: u32, word_gap: u8) -> u8 {
    use crate::phoneme::{
        PHON_PAUSE, PHON_PAUSE_LONG, PHON_PAUSE_NOLINK, PHON_PAUSE_SHORT, PHON_PAUSE_VSHORT,
    };
    const PH_VOWEL: u8 = 2;
    const PH_PAUSE: u8 = 0;
    // C `pause_phonemes[8]`; index 5 is a glottal stop, which no shipped
    // language selects through `word_gap`.
    const PAUSE_PHONEMES: [u8; 8] = [
        0,
        PHON_PAUSE_VSHORT,
        PHON_PAUSE_SHORT,
        PHON_PAUSE,
        PHON_PAUSE_LONG,
        0,
        PHON_PAUSE_LONG,
        PHON_PAUSE_LONG,
    ];
    let mut insert = 0u8;
    if vowel_pause != 0 && pt != PH_PAUSE {
        if pt != PH_VOWEL && vowel_pause & 0x200 != 0 {
            insert = PHON_PAUSE_NOLINK;
        }
        if nt == PH_VOWEL {
            match vowel_pause & 0x0c {
                0 => {}
                0xc => insert = PHON_PAUSE_NOLINK,
                _ => insert = PHON_PAUSE_VSHORT,
            }
            if pt == PH_VOWEL {
                match vowel_pause & 0x03 {
                    0 => {}
                    2 => insert = PHON_PAUSE_SHORT,
                    _ => insert = PHON_PAUSE_VSHORT,
                }
            }
            if vowel_pause & 0x100 != 0 && next_stressed {
                insert = PHON_PAUSE_SHORT;
            }
        }
    }
    let x = (word_gap & 7) as usize;
    if x != 0 && (x > 1 || (insert != PHON_PAUSE_SHORT && insert != PHON_PAUSE_NOLINK)) {
        insert = PAUSE_PHONEMES.at(x);
    }
    insert
}

/// Apply the voice file's `replace <flags> <old> <new>` substitutions to the
/// finished code list — the `replace_phonemes[]` loop at the top of
/// `MakePhonemeList` (phonemelist.c).
///
/// `flags` bit 0 restricts the substitution to a word end, bit 1 skips it in a
/// stressed syllable, bit 2 restricts it to a word start; a `NULL` replacement
/// deletes the phoneme.  Fourteen shipped voices use this — Russian's
/// `replace 03 a a#` is what turns a word-final unstressed `a` into `a#`.
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
/// Returns the indices of the codes it deleted.
fn apply_voice_replacements(
    codes: &mut Vec<PhonemeCode>,
    phdata: &PhonemeData,
    rules: &[(u8, u8, u8)],
) -> Vec<usize> {
    if rules.is_empty() {
        return Vec::new();
    }
    let is_sound = |c: &PhonemeCode| {
        c.marker.is_none()
            && !c.is_boundary
            && c.code > 8
            && c.code != crate::phoneme::PHON_END_WORD
    };
    let real: Vec<usize> = codes
        .iter()
        .enumerate()
        .filter(|(_, c)| is_sound(c))
        .map(|(i, _)| i)
        .collect();
    let (level, _) = stress_levels(codes, phdata);
    let starts = word_starts(codes, &real);

    let mut deleted: Vec<usize> = Vec::new();
    for (n, &i) in real.iter().enumerate() {
        // C keeps `stresslevel` on consonants too (it copies the following
        // vowel's); approximate with the next vowel in the same word.
        let stress = if level.at(i) != NO_STRESS_LEVEL {
            level.at(i)
        } else {
            real.get(n + 1..)
                .unwrap_or_default()
                .iter()
                .enumerate()
                .take_while(|(k, _)| !starts.at(n + 1 + k))
                .find_map(|(_, &j)| (level.at(j) != NO_STRESS_LEVEL).then(|| level.at(j)))
                .unwrap_or(1)
        };
        let word_end = match real.get(n + 1) {
            None => true,
            Some(&next) => {
                starts[n + 1] || matches!(phdata.get(codes.code_at(next)), Some(p) if p.typ == 0)
            }
        };
        for &(flags, old, new) in rules {
            if codes.code_at(i) != old {
                continue;
            }
            if flags & 1 != 0 && !word_end {
                continue;
            }
            if flags & 2 != 0 && stress & 7 > 3 {
                continue;
            }
            if flags & 4 != 0 && !starts.at(n) {
                continue;
            }
            if new == 0 {
                deleted.push(i);
            } else {
                codes.set_code(i, new);
            }
            break;
        }
    }
    for &i in deleted.iter().rev() {
        codes.remove(i);
    }
    deleted
}

/// Apply each phoneme's own `ChangePhoneme` program to a finished code list.
///
/// Upstream resolves context-dependent phonemes while *building* the phoneme
/// list, so `-x`, `--ipa` and the synthesizer all see the resolved phoneme.  The
/// port resolved them only when rendering IPA, so `-x -v pt "três vírgula"` printed
/// the unresolved `t#*'es#` (upstream: `t@-*'eZ`) and the synthesizer voiced the
/// virtual phoneme, which has no acoustic data of its own.
/// Apply the list-editing instructions a phoneme's program asks for —
/// `InsertPhoneme`, `AppendPhoneme`, `IfNextVowelAppend` and
/// `ChangeNextPhoneme` — to a finished code list.
///
/// C does this in `MakePhonemeList`; here it gives `-x` and the synthesizer the
/// same phoneme list the IPA renderer builds, so English gets its linking `r-`
/// ("sofa area" → `s'oUf@r- 'e@ri@`) and the `;` glide after `aI` ("my apple" →
/// `maI; 'ap@L`) in the audio too.
/// Give a clause with no primary stress one, on its last strongest syllable.
///
/// C's intonation pass does this (`count_pitch_vowels`: "no primary stress.
/// Use the highest stress") and writes the level back into the phoneme list,
/// so `-x` and `--ipa` show it: French "est-ce" is `'Es` — "est" is `$u+` and
/// the "ce" after the hyphen has lost its vowel, leaving nothing stressed.
fn promote_stress_in_unstressed_clauses(
    codes: &mut Vec<PhonemeCode>,
    phdata: &PhonemeData,
    lang: &str,
    stress_flags: u32,
    switched_vowel: &dyn Fn(&str, u8) -> bool,
    no_tone_clauses: &[usize],
) {
    use crate::phoneme::{PHON_STRESS_P, PHON_STRESS_P2, PHON_STRESS_TONIC};
    let (level, wordstress) = stress_levels(codes, phdata);
    // A word in another language has its own stress, which counts too.
    let mut switched = vec![false; codes.len()];
    let mut level = level;
    let mut target: Option<String> = None;
    let mut pending = 1u8;
    for (k, c) in codes.iter().enumerate() {
        if let Some(CodeMarker::LangSwitch(t)) = &c.marker {
            target = (t != lang).then(|| t.clone());
            continue;
        }
        let Some(t) = &target else { continue };
        switched.put(k, true);
        // The levels of that word's vowels, by its own table.
        match c.code {
            PHON_STRESS_P | PHON_STRESS_P2 | PHON_STRESS_TONIC if !c.is_boundary => pending = 4,
            2..=5 if !c.is_boundary => pending = c.code - 1,
            code if !c.is_boundary && switched_vowel(t, code) => {
                level.put(k, pending);
                pending = 1;
            }
            _ => {}
        }
    }
    let before = level.clone();
    diminish_unstressed_runs(codes, phdata, &mut level, &wordstress, stress_flags);
    let dims: Vec<usize> = (0..level.len()).filter(|&k| level.at(k) < before.at(k)).collect();
    let mut start = 0usize;
    let mut inserts: Vec<usize> = Vec::new();
    let mut clause_no = 0usize;
    for i in 0..=codes.len() {
        let clause_end = i == codes.len()
            || (codes.boundary_at(i)
                && codes.code_at(i) == 0
                && codes.get(i).is_some_and(|c| c.clause_char.is_some()));
        // A clause pause inside it (Lojban `_;_`) ends a tone group too, once
        // the group has two syllables (`CalcPitches`' `SYL_END_CLAUSE`):
        // `ijo poi` gives `poi` a group, and so a stress, of its own.
        let group_end = !clause_end
            && codes.get(i).is_some_and(|c| {
                !c.is_boundary && c.marker.is_none() && c.code == crate::phoneme::PHON_PAUSE_CLAUSE
            })
            && (start..i)
                .filter(|&k| level.at(k) != NO_STRESS_LEVEL && (!switched.at(k) || level.at(k) > 0))
                .count()
                >= 2;
        if !clause_end && !group_end {
            continue;
        }
        let vowels = (start..i)
            .filter(|&k| level.at(k) != NO_STRESS_LEVEL && (!switched.at(k) || level.at(k) > 0));
        let has_primary = vowels.clone().any(|k| level.at(k) >= 4);
        // The last vowel of the highest level (C's `max_stress_posn`).
        let best = vowels.max_by_key(|&k| level.at(k));
        if let (false, Some(v)) = (has_primary, best) {
            if !no_tone_clauses.contains(&clause_no) {
                inserts.push(v);
            }
        }
        if clause_end {
            clause_no += 1;
        }
        start = i + 1;
    }
    // The synthesizer takes the diminished level from its mark.
    let mut marks: Vec<(usize, u8)> = inserts.iter().map(|&v| (v, PHON_STRESS_P)).collect();
    marks.extend(dims.iter().filter(|v| !inserts.contains(v)).map(|&v| (v, crate::phoneme::PHON_STRESS_D)));
    marks.sort_unstable();
    for &(v, mark) in marks.iter().rev() {
        // Replace the vowel's own (secondary / diminished) mark, or add one.
        let prev = codes
            .get(..v)
            .unwrap_or_default()
            .iter()
            .rposition(|c| c.marker.is_none());
        match prev {
            Some(k)
                if (if mark == PHON_STRESS_P {
                    crate::phoneme::PHON_STRESS_D
                } else {
                    crate::phoneme::PHON_STRESS_U
                }..=PHON_STRESS_P2)
                    .contains(&codes.code_at(k))
                    || codes.code_at(k) == PHON_STRESS_TONIC =>
            {
                codes.set_code(k, mark);
            }
            _ => codes.insert(
                v,
                PhonemeCode {
                    code: mark,
                    ..Default::default()
                },
            ),
        }
    }
}

/// Drop a stress mark whose syllable no longer has a vowel.
///
/// C keeps stress on the *vowel* (`stresslevel` in the phoneme list) and
/// writes the mark only before a syllable that has one, so a vowel that a
/// phoneme program turned into a glide takes its stress with it: French
/// "demi-heure" is `d@mj'Wr` (the `i` of "demi" becomes `j` before the vowel
/// of the joined word), and "joues", whose rules leave only `Zw`, has no mark
/// at all.  The port places explicit stress codes, which outlived the vowel
/// (`d@m'j'Wr`, `Z'w`).
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
fn drop_vowelless_stress(codes: &mut Vec<PhonemeCode>, phdata: &PhonemeData) {
    use crate::phoneme::{PHON_STRESS_2, PHON_STRESS_P2, PHON_STRESS_TONIC, PHON_SYLLABIC};
    let is_stress = |c: u8| {
        (crate::phoneme::PHON_STRESS_U..=crate::phoneme::PHON_STRESS_PREV).contains(&c)
            || c == PHON_STRESS_TONIC
    };
    let mut drop: Vec<usize> = Vec::new();
    for i in 0..codes.len() {
        let c = &codes[i];
        if c.marker.is_some()
            || c.is_boundary
            || !((PHON_STRESS_2..=PHON_STRESS_P2).contains(&c.code) || c.code == PHON_STRESS_TONIC)
        {
            continue;
        }
        // The mark stands straight before its vowel (or before a consonant
        // the next code makes syllabic).
        let mut next = codes
            .get(i + 1..)
            .unwrap_or_default()
            .iter()
            .filter(|d| d.marker.is_none());
        let voiced = match next.next() {
            Some(d)
                if d.is_boundary
                    || d.code == crate::phoneme::PHON_END_WORD
                    || is_stress(d.code) =>
            {
                true
            }
            Some(d) => {
                phdata.get(d.code).is_some_and(|p| p.typ == 2)
                    || next.next().is_some_and(|e| e.code == PHON_SYLLABIC)
            }
            None => true,
        };
        if !voiced {
            drop.push(i);
        }
    }
    for &i in drop.iter().rev() {
        codes.remove(i);
    }
}

/// C's `SetPlist`: the syllabic marker gives the phoneme before it the stress
/// still pending, and isn't a phoneme of its own.  After a vowel, which used
/// its stress, that leaves the vowel unstressed (Faroese `_0and` `u-o` reads
/// `uo`).
fn syllabic_after_vowel(codes: &mut Vec<PhonemeCode>, phdata: &PhonemeData) {
    use crate::phoneme::{PHON_STRESS_2, PHON_STRESS_P2, PHON_STRESS_TONIC, PHON_SYLLABIC};
    let mut i = 0;
    while i < codes.len() {
        let here = codes
            .get(i)
            .is_some_and(|c| c.code == PHON_SYLLABIC && c.marker.is_none() && !c.is_boundary);
        let vowel = here
            && i > 0
            && codes.get(i - 1).is_some_and(|p| {
                p.marker.is_none()
                    && !p.is_boundary
                    && phdata.get(p.code).is_some_and(|ph| ph.typ == 2)
            });
        if !vowel {
            i += 1;
            continue;
        }
        codes.remove(i);
        // The vowel's own stress mark goes.
        let mut k = i - 1;
        while k > 0
            && codes.get(k - 1).is_some_and(|m| {
                m.marker.is_none()
                    && !m.is_boundary
                    && ((PHON_STRESS_2..=PHON_STRESS_P2).contains(&m.code)
                        || m.code == PHON_STRESS_TONIC)
            })
        {
            codes.remove(k - 1);
            k -= 1;
        }
        i = k + 1;
    }
}

/// Drop a French liaison phoneme that doesn't surface.
///
/// `z2`, `t2`, `n2`, `z3` … are consonants that are only pronounced before a
/// vowel ("deux enfants" keeps the /z/, "deux chats" drops it).  The IPA
/// renderer already resolved them; the codes path kept them, so `-x -v fr
/// "3,14"` printed `trwaz2` where upstream has `trwa`.
fn drop_unsurfaced_liaison(codes: &mut Vec<PhonemeCode>, phdata: &PhonemeData) {
    let is_sound =
        |c: &PhonemeCode| !c.is_boundary && c.code > 8 && c.code != crate::phoneme::PHON_END_WORD;
    let real: Vec<usize> = codes
        .iter()
        .enumerate()
        .filter(|(_, c)| is_sound(c))
        .map(|(i, _)| i)
        .collect();
    let mut drop: Vec<usize> = Vec::new();
    for (n, &i) in real.iter().enumerate() {
        let Some(ph) = phdata.get(codes.code_at(i)) else {
            continue;
        };
        let m = ph.mnemonic.to_le_bytes();
        let is_liaison = m[2] == 0 && matches!(m[1], b'2' | b'3') && ph.typ != 2;
        if !is_liaison {
            continue;
        }
        let next_is_vowel = real
            .get(n + 1)
            .and_then(|&k| phdata.get(codes.code_at(k)))
            .is_some_and(|n| n.typ == 2 /* phVOWEL */);
        if !next_is_vowel {
            drop.push(i);
        }
    }
    for &i in drop.iter().rev() {
        codes.remove(i);
    }
}

/// `SetRegressiveVoicing` (`phonemelist.c`) — make a consonant cluster all
/// voiced or all voiceless, taking its voicing from the *last* member.
///
/// Walks the list backwards: a voiced obstruent sets `voicing = 2` and a
/// voiceless one sets `voicing = 1`; whichever is in force converts the
/// obstruents before it through their `end_type` (the voicing-switch partner
/// stored in the phoneme table).  Ukrainian `світ` is `zB'i:t[` because the `в`
/// that follows is voiced — and with `regression & 0x04` unset the assimilation
/// crosses word boundaries, so `привіт світ` ends the first word in `d[`.
///
/// `regression` bits: `0x02` = `v`/`R` stop propagation, `0x04` = stop at a word
/// boundary, `0x08` = propagate through liquids and nasals (Polish), `0x100` =
/// devoice word-finally.
fn set_regressive_voicing(
    codes: &mut [PhonemeCode],
    phdata: &PhonemeData,
    regression: u32,
    lang: &str,
) {
    use crate::phoneme::{PH_FRICATIVE, PH_PAUSE, PH_STOP, PH_VFRICATIVE, PH_VOWEL, PH_VSTOP};

    // 0 = nothing in force, 1 = voiceless, 2 = voiced.
    // The clause ends in a word end whether or not a boundary marks it.
    let mut voicing = u8::from(regression & 0x100 != 0);
    let mut stop_propagation = false;

    // C's `SFLAG_SWITCHED_LANG`: a word in another language keeps its own
    // voicing (German "job" is `dZ'0b`), and its codes index another table.
    let mut in_switch = false;
    let switched: Vec<bool> = codes
        .iter()
        .map(|c| {
            if let Some(CodeMarker::LangSwitch(target)) = &c.marker {
                in_switch = target != lang;
            }
            in_switch
        })
        .collect();

    for i in (0..codes.len()).rev() {
        // A word boundary is a marker, not a phoneme; note it and move on.
        // (The parts of a composed number are joined by a bare `END_WORD`.)
        if codes.boundary_at(i)
            || codes.get(i).is_some_and(|c| c.marker.is_some())
            || codes.code_at(i) == PHON_END_WORD
        {
            // A *clause* boundary is a pause phoneme in C's list, and a pause
            // resets the voicing in force — which is what devoices Polish
            // `flag.` before a clause starting with a voiced consonant
            // (`fl'ak|z'aZuwts;`, where the port used to read `fl'ag`).
            if codes.boundary_at(i) && codes.code_at(i) == 0 {
                voicing = 0;
            }
            if regression & 0x04 != 0 {
                voicing = 0;
            }
            if regression & 0x100 != 0 && voicing == 0 {
                voicing = 1;
            }
            continue;
        }
        if switched.at(i) {
            stop_propagation = false;
            voicing = u8::from(regression & 0x100 != 0);
            continue;
        }
        let code = codes.code_at(i);
        let Some(ph) = phdata.get(code) else { continue };

        if regression & 0x02 != 0 {
            // `v` and `R` don't cause regression.
            let first = (ph.mnemonic & 0xff) as u8;
            if first == b'v' || first == b'R' {
                stop_propagation = true;
                if regression & 0x10 != 0 {
                    voicing = 0;
                }
            }
        }

        // `end_type` doubles as the voicing-switch partner for consonants, but
        // only when it really points at an obstruent of the opposite voicing —
        // several tables use the field for something else, and following it
        // blindly turned Czech `milióni` into `miliʲóni`.
        let switch_to = |want_voiced: bool| -> Option<u8> {
            let t = ph.end_type;
            let target = phdata.get(t)?;
            let ok = if want_voiced {
                matches!(target.typ, PH_VSTOP | PH_VFRICATIVE)
            } else {
                matches!(target.typ, PH_STOP | PH_FRICATIVE)
            };
            (t != 0 && ok).then_some(t)
        };

        match ph.typ {
            PH_STOP | PH_FRICATIVE => {
                if voicing == 0 && regression & 0xf != 0 {
                    voicing = 1;
                } else if voicing == 2 {
                    if let Some(t) = switch_to(true) {
                        codes.set_code(i, t); // → the voiced equivalent
                    }
                }
            }
            PH_VSTOP | PH_VFRICATIVE => {
                if voicing == 0 && regression & 0xf != 0 {
                    voicing = 2;
                } else if voicing == 1 {
                    if let Some(t) = switch_to(false) {
                        codes.set_code(i, t); // → the voiceless equivalent
                    }
                }
            }
            t => {
                if regression & 0x08 != 0 {
                    // Polish: propagate through liquids and nasals.
                    if t == PH_PAUSE || t == PH_VOWEL {
                        voicing = 0;
                    }
                } else {
                    voicing = 0;
                }
            }
        }

        if stop_propagation {
            voicing = 0;
            stop_propagation = false;
        }
    }
}

/// Which of the phonemes in `real` start a word.
///
/// C marks a word's first phoneme with `sourceix != 0`, and the `*PhW`
/// conditions refuse to look across one: Dutch `Q` deletes itself after an `x`
/// *in the same word* (`IF prevPhW(x) THEN ChangePhoneme(NULL)`), so "twintig
/// graden" must keep its `ɣ`.
pub(crate) fn word_starts(codes: &[PhonemeCode], real: &[usize]) -> Vec<bool> {
    let mut out = Vec::with_capacity(real.len());
    let mut prev_end = 0usize;
    for (n, &i) in real.iter().enumerate() {
        // (A run that opens with a switch back from another language inside a
        // word — a letter after its alphabet's English name — doesn't start
        // the word.)
        let mid_word = n == 0
            && matches!(
                codes.first().and_then(|c| c.marker.as_ref()),
                Some(CodeMarker::LangSwitch(_))
            );
        out.push(
            (n == 0 && !mid_word)
                || codes
                    .get(prev_end..i)
                    .unwrap_or_default()
                    .iter()
                    .any(|c| c.is_boundary || c.code == crate::phoneme::PHON_END_WORD),
        );
        prev_end = i + 1;
    }
    out
}

/// How many vowels of its word come up to and including each code — C's
/// `CountVowelPosition`, which `isFirstVowel`/`isSecondVowel` test.
pub(crate) fn vowel_positions(codes: &[PhonemeCode], phdata: &PhonemeData) -> Vec<u8> {
    let mut out = vec![0u8; codes.len()];
    let mut n = 0u8;
    for (i, c) in codes.iter().enumerate() {
        if c.is_boundary || c.code == crate::phoneme::PHON_END_WORD {
            n = 0;
            continue;
        }
        if matches!(phdata.get(c.code), Some(p) if p.typ == 2 /* phVOWEL */) {
            n = n.saturating_add(1);
        }
        // A consonant counts the vowels before it too (`CountVowelPosition`):
        // Malayalam final `k` after the first vowel stays `k`.
        out.put(i, n);
    }
    out
}

/// Per-syllable stress levels for a clause's code list.
///
/// `SetWordStress` emits a stress phoneme ahead of every vowel whose level is
/// *diminished* (0) or above *unstressed* (1) — `phonSTRESS_D` for 0,
/// `phonSTRESS_2`/`_3`/`_P`/`_P2`/`_TONIC` for 2…6 — and nothing at all for 1.
/// Reading them back therefore recovers C's `plist->stresslevel` exactly, which
/// is what the phoneme programs' stress conditions test.
///
/// Returns `(level, wordstress)`, both indexed like `codes`: `level[i]` is the
/// vowel's level or [`NO_STRESS_LEVEL`] for anything that is not a vowel, and
/// `wordstress[i]` is the highest level in `i`'s word (C's `plist->wordstress`).
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
pub(crate) fn stress_levels(codes: &[PhonemeCode], phdata: &PhonemeData) -> (Vec<u8>, Vec<u8>) {
    let mut level = vec![NO_STRESS_LEVEL; codes.len()];
    let mut pending = 1u8; // STRESS_IS_UNSTRESSED
    for (i, c) in codes.iter().enumerate() {
        if c.is_boundary || c.code == crate::phoneme::PHON_END_WORD {
            pending = 1;
            continue;
        }
        match c.code {
            PHON_STRESS_D => pending = 0,
            PHON_STRESS_U => pending = 1,
            PHON_STRESS_2 => pending = 2,
            PHON_STRESS_3 => pending = 3,
            PHON_STRESS_P => pending = 4,
            PHON_STRESS_P2 => pending = 5,
            PHON_STRESS_TONIC => pending = 6,
            code => {
                if matches!(phdata.get(code), Some(p) if p.typ == 2 /* phVOWEL */) {
                    level.put(i, pending);
                    pending = 1;
                }
            }
        }
    }

    // The word's maximum, broadcast back over the word.
    let mut wordstress = vec![0u8; codes.len()];
    let mut start = 0usize;
    let mut i = 0usize;
    while i <= codes.len() {
        let ends = i == codes.len()
            || codes.boundary_at(i)
            || codes.code_at(i) == crate::phoneme::PHON_END_WORD;
        if ends {
            let max = level
                .get(start..i)
                .unwrap_or_default()
                .iter()
                .filter(|&&l| l != NO_STRESS_LEVEL)
                .copied()
                .max()
                .unwrap_or(4);
            wordstress[start..i].fill(max);
            start = i + 1;
        }
        i += 1;
    }
    (level, wordstress)
}

/// `MakePhonemeList`'s pass over a clause's unstressed vowels, "even across
/// word boundaries": every second one of a run is diminished, though not the
/// last sound of a stressed word.  (English instead diminishes a pair of them
/// in unstressed words.)  It decides which vowel takes a clause's stress when
/// no word has one: German "ich habe" is `IC hA:b'@`, "habe" alone `h'A:b@`.
fn diminish_unstressed_runs(
    codes: &[PhonemeCode],
    phdata: &PhonemeData,
    level: &mut [u8],
    wordstress: &[u8],
    stress_flags: u32,
) {
    const S_NO_DIM: u32 = 0x02;
    const S_EN_DIM: u32 = 0x08;
    let n = codes.len().min(level.len());
    let word_final = |k: usize| {
        codes.get(k + 1..).unwrap_or_default().iter().find(|c| c.marker.is_none()).is_none_or(|c| {
            c.is_boundary || c.code == crate::phoneme::PHON_END_WORD
        })
    };
    let clause_end =
        |k: usize| codes.get(k).is_some_and(|c| c.is_boundary && c.code == 0 && c.clause_char.is_some());
    let pause = |k: usize| {
        codes.get(k).is_some_and(|c| {
            c.marker.is_none()
                && !c.is_boundary
                && c.code > 8
                && phdata.get(c.code).is_some_and(|p| p.typ == 0)
        })
    };
    let mut count = 0u32;
    for k in 0..n {
        if clause_end(k) {
            count = 0;
            continue;
        }
        let l = level.at(k);
        if l == NO_STRESS_LEVEL {
            continue;
        }
        if l > 1 {
            count = 0;
            continue;
        }
        count += 1;
        if stress_flags & S_EN_DIM != 0 {
            // The next vowel, up to a pause.
            let next = (k + 1..n)
                .take_while(|&j| !clause_end(j) && !pause(j))
                .find(|&j| level.at(j) != NO_STRESS_LEVEL);
            if let Some(j) = next.filter(|&j| level.at(j) <= 1) {
                if wordstress.get(k).copied().unwrap_or(4) < 4 {
                    level.put(k, 0);
                }
                if wordstress.get(j).copied().unwrap_or(4) < 4 {
                    level.put(j, 0);
                }
            }
        } else if count > 1 && count & 1 == 0 {
            if stress_flags & S_NO_DIM != 0
                || (wordstress.get(k).copied().unwrap_or(0) > 3 && word_final(k))
            {
                count = 1;
            } else {
                level.put(k, 0);
            }
        }
    }
}

/// English's form of the pass over unstressed vowels (`stress_flags & 0x08`):
/// at an unstressed vowel, C already diminishes the next one, up to a pause,
/// when its word is unstressed, before that vowel's phoneme program runs.  So
/// the program sees it: English "we am" is `wi:; 'a#m`.
fn diminish_next_unstressed(codes: &mut Vec<PhonemeCode>, phdata: &PhonemeData) {
    let (level, wordstress) = stress_levels(codes, phdata);
    let pause = |c: &PhonemeCode| {
        c.is_boundary && c.code == 0
            || c.marker.is_none()
                && !c.is_boundary
                && c.code > 8
                && phdata.get(c.code).is_some_and(|p| p.typ == 0)
    };
    let mut marks: Vec<usize> = Vec::new();
    for k in 0..codes.len() {
        if level.at(k) > 1 {
            continue;
        }
        let next = (k + 1..codes.len())
            .take_while(|&j| codes.get(j).is_some_and(|c| !pause(c)))
            .find(|&j| level.at(j) != NO_STRESS_LEVEL);
        if let Some(j) = next.filter(|&j| level.at(j) <= 1 && wordstress.at(j) < 4) {
            if marks.last() != Some(&j) {
                marks.push(j);
            }
        }
    }
    for &v in marks.iter().rev() {
        let prev = codes
            .get(..v)
            .unwrap_or_default()
            .iter()
            .rposition(|c| c.marker.is_none() && !c.is_boundary);
        match prev {
            Some(k)
                if (crate::phoneme::PHON_STRESS_U..=crate::phoneme::PHON_STRESS_P2)
                    .contains(&codes.code_at(k)) =>
            {
                codes.set_code(k, crate::phoneme::PHON_STRESS_D);
            }
            _ => codes.insert(
                v,
                PhonemeCode {
                    code: crate::phoneme::PHON_STRESS_D,
                    ..Default::default()
                },
            ),
        }
    }
}

/// `stress_levels` marker for a code that is not a vowel.
pub(crate) const NO_STRESS_LEVEL: u8 = u8::MAX;

/// C's `MakePhonemeList`: a consonant followed by the lengthener (`:`) is
/// lengthened **by doubling it** when it is a fricative, voiced fricative, nasal
/// or liquid, and the lengthener goes away.  Italian writes its geminates that
/// way — `mm` compiles to `m:` and reads `s'omma`, not `s'om:a`.
///
/// Every other type keeps the lengthener, which is why a long vowel still reads
/// `a:` (Dutch `Qr'a:d@n`) and a phoneme that is itself long, like Italian `tS:`,
/// is untouched.
fn double_lengthened_consonants(codes: &mut Vec<PhonemeCode>, phdata: &PhonemeData) {
    use crate::phoneme::{PH_FRICATIVE, PH_LIQUID, PH_NASAL, PH_PAUSE, PH_VFRICATIVE};
    // Two lengtheners are one: C sets a flag on the phoneme.
    let lengthener = |c: &PhonemeCode| {
        !c.is_boundary && c.marker.is_none() && c.code == crate::phoneme::PHON_LENGTHEN
    };
    let mut k = 1;
    while k < codes.len() {
        if codes.get(k).is_some_and(lengthener) && codes.get(k - 1).is_some_and(lengthener) {
            codes.remove(k);
        } else {
            k += 1;
        }
    }
    for i in 1..codes.len() {
        if codes.boundary_at(i)
            || codes.get(i).is_some_and(|c| c.marker.is_some())
            || codes.code_at(i) != crate::phoneme::PHON_LENGTHEN
        {
            continue;
        }
        let Some(prev) = codes
            .get(..i)
            .unwrap_or_default()
            .iter()
            .rposition(|c| !c.is_boundary && c.marker.is_none() && c.code > 8)
        else {
            continue;
        };
        let doubled = phdata.get(codes.code_at(prev)).is_some_and(|p| {
            matches!(
                p.typ,
                PH_FRICATIVE | PH_VFRICATIVE | PH_NASAL | PH_LIQUID | PH_PAUSE
            )
        });
        // (Not the clause's first phoneme: C has nowhere to insert it.)
        let first = !codes.get(..prev).unwrap_or_default().iter().any(|c| {
            !c.is_boundary
                && c.marker.is_none()
                && c.code > 8
                && c.code != crate::phoneme::PHON_END_WORD
        });
        if !doubled || first {
            continue;
        }
        // A word's first consonant is doubled at the end of the word before,
        // as C inserts it ahead of the word start: Finnish "virkes s'yY".
        let word_start = codes
            .get(..prev)
            .unwrap_or_default()
            .iter()
            .rposition(|c| c.marker.is_none() && (c.is_boundary || c.code > 8))
            .filter(|&k| {
                let c = codes.get(k);
                c.is_some_and(|c| c.is_boundary || c.code == crate::phoneme::PHON_END_WORD)
            });
        let consonant = codes.code_at(prev);
        match word_start {
            Some(k) if k > 0 => {
                codes.remove(i);
                codes.insert(
                    k,
                    PhonemeCode {
                        code: consonant,
                        ..Default::default()
                    },
                );
            }
            _ => codes.set_code(i, consonant),
        }
    }
}

fn apply_list_editing_instructions(
    codes: &mut Vec<PhonemeCode>,
    phdata: &PhonemeData,
    from_dict: &[bool],
    reduce: u32,
    // The language's `vowel_pause` and `word_gap`: a pause between two words
    // takes the place of an appended phoneme (Italian "si alza" is `sI_| 'altsa`).
    vowel_pause: u32,
    word_gap: u8,
) {
    let context = ProgramContext::new(codes, phdata);
    let real = &context.real;
    // (index to insert at, code, before?) — collected first so indices stay valid.
    let mut edits: Vec<(usize, u8, bool)> = Vec::new();
    let mut replacements: Vec<(usize, u8)> = Vec::new();

    for (n, &i) in real.iter().enumerate() {
        let code = codes.code_at(i);
        let Some(ph) = phdata.get(code) else { continue };
        if ph.program == 0 {
            continue;
        }
        let given = from_dict.get(i).copied().unwrap_or(false);
        let nb = context.neighbours(codes, phdata, n, given, reduce);
        let (next, next2) = (nb.next, nb.next2);
        let fx = crate::synthesize::bytecode::interpret_phoneme_ctl(
            ph.program,
            &phdata.phonindex,
            &nb,
            |c| phdata.get(c).cloned(),
            true,
        );

        if let Some(c) = fx.insert_phoneme.filter(|&c| c != 0) {
            edits.push((i, c, true));
        }
        let gap_here = nb.next_wordstart && {
            let typ = |c: u8| phdata.get(c).map_or(0, |ph| ph.typ);
            let next_stressed = context.real.get(n + 1).is_some_and(|&k| {
                context
                    .level
                    .get(k)
                    .is_some_and(|&l| l != NO_STRESS_LEVEL && l >= 4)
            });
            word_gap_pause(typ(code), typ(next), next_stressed, vowel_pause, word_gap) != 0
        };
        let appended = fx
            .append_phoneme
            .filter(|&c| c != 0 && !gap_here)
            .or_else(|| {
                fx.append_if_next_vowel
                    .filter(|&c| c != 0 && !gap_here)
                    .filter(|_| matches!(phdata.get(next), Some(n) if n.typ == 2 /* phVOWEL */))
            });
        if let Some(c) = appended {
            // The appended phoneme runs its own program, which may change it:
            // English linking `r-` is a plain `r` after most vowels.
            let c = phdata
                .get(c)
                .filter(|added| added.program != 0)
                .and_then(|added| {
                    let nb = crate::synthesize::bytecode::Neighbours {
                        prev: code,
                        this: c,
                        next,
                        next2,
                        prev_wordstart: nb.this_wordstart,
                        next_wordstart: nb.next_wordstart,
                        next2_wordstart: nb.next2_wordstart,
                        ..Default::default()
                    };
                    crate::synthesize::bytecode::interpret_phoneme_ctl(
                        added.program,
                        &phdata.phonindex,
                        &nb,
                        |c| phdata.get(c).cloned(),
                        true,
                    )
                    .change_phoneme_code
                })
                .filter(|&changed| changed > 1)
                .unwrap_or(c);
            edits.push((i, c, false));
        }
        if let Some(c) = fx.replace_next_phoneme.filter(|&c| c != 0) {
            if let Some(&k) = real.get(n + 1) {
                replacements.push((k, c));
            }
        }
    }

    for (i, c) in replacements {
        codes.set_code(i, c);
    }
    if edits.is_empty() {
        return;
    }
    // Apply back to front so earlier indices stay valid.
    // (At one phoneme the append goes in first, or the insert in front of
    // the phoneme would move the append's place: Malay `si:_|aI_|'Em`.)
    edits.sort_by(|a, b| b.0.cmp(&a.0).then(a.2.cmp(&b.2)));
    for (i, code, before) in edits {
        let at = if before {
            i
        } else if codes
            .get(i + 1)
            .is_some_and(|c| c.code == crate::phoneme::PHON_LENGTHEN)
        {
            i + 2
        } else {
            i + 1
        };
        codes.insert(
            at,
            PhonemeCode {
                code,
                is_boundary: false,
                clause_char: None,
                marker: None,
            },
        );
    }
}

/// What the phoneme programs of a code list are conditioned on.
struct ProgramContext {
    /// The stress level of each code's syllable, and of its word.
    level: Vec<u8>,
    wordstress: Vec<u8>,
    vowel_pos: Vec<u8>,
    /// The codes that are real sounds, and which of them start a word.
    real: Vec<usize>,
    starts: Vec<bool>,
    after_stress: Vec<bool>,
}

impl ProgramContext {
    fn new(codes: &[PhonemeCode], phdata: &PhonemeData) -> Self {
        // Stress-conditioned allophones (`ChangeIfDiminished(…)` and friends) are
        // applied by the interpreter below, which evaluates the real condition
        // against the syllable's stress level — Russian "молоко" reduces to
        // `mVlVk'o`, while English "unseen" keeps its unreduced `V` because that
        // syllable is *unstressed*, not *diminished*.
        let (level, wordstress) = stress_levels(codes, phdata);
        let vowel_pos = vowel_positions(codes, phdata);

        // Index of the next/previous code that is a real sound (not a boundary).
        // The length mark is *not* one: C turns a `phonLENGTHEN` into an
        // `SFLAG_LENGTHEN` flag on the phoneme before it and never puts it in the
        // list, so a program's `nextPh`/`next2Ph` looks straight past it.  Keeping
        // it visible made Turkish `teşekkür` read `teSEk:'YR` — the `e` before
        // `k:` saw "two non-vowels" where C sees the vowel after it.
        let real: Vec<usize> = codes
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                !c.is_boundary
                    && c.code > 8
                    && c.code != crate::phoneme::PHON_END_WORD
                    && c.code != crate::phoneme::PHON_LENGTHEN
            })
            .map(|(i, _)| i)
            .collect();
        let starts = word_starts(codes, &real);
        // C's `isAfterStress`: a primary stress earlier in the same word.
        let mut stressed = false;
        let after_stress: Vec<bool> = real
            .iter()
            .zip(&starts)
            .map(|(&i, &start)| {
                stressed &= !start;
                let after = stressed;
                stressed |= level.at(i) != NO_STRESS_LEVEL && level.at(i) >= 4;
                after
            })
            .collect();
        ProgramContext {
            level,
            wordstress,
            vowel_pos,
            real,
            starts,
            after_stress,
        }
    }

    /// The surroundings of the `n`-th sound.
    fn neighbours(
        &self,
        codes: &[PhonemeCode],
        phdata: &PhonemeData,
        n: usize,
        given: bool,
        reduce: u32,
    ) -> crate::synthesize::bytecode::Neighbours {
        let (real, starts) = (&self.real, &self.starts);
        let i = real.at(n);
        let code = codes.code_at(i);
        let wordstart = |k: usize| starts.get(k).copied().unwrap_or(false);
        // A clause boundary is a *pause phoneme* in C's list, so a neighbour on
        // the other side of one is not visible as a sound: `es "a , b"` is the
        // stop `b'e`, not the post-vocalic fricative `B'e` the port read by
        // looking past the comma.
        let clause_break = |a: usize, b: usize| {
            codes
                .get(a + 1..b)
                .unwrap_or_default()
                .iter()
                .any(|c| c.is_boundary && c.code == 0)
        };
        let prev = n
            .checked_sub(1)
            .map(|k| real.at(k))
            .filter(|&k| !clause_break(k, i))
            .map(|k| codes.code_at(k))
            .unwrap_or(0);
        let next = real
            .get(n + 1)
            .copied()
            .filter(|&k| !clause_break(i, k))
            .map(|k| codes.code_at(k))
            .unwrap_or(0);
        let next2 = real
            .get(n + 2)
            .copied()
            .filter(|&k| !clause_break(i, k))
            .map(|k| codes.code_at(k))
            .unwrap_or(0);
        // A boundary marker between this phoneme and the next one means the next
        // phoneme starts a new word (`nextPhW` must not see through it).
        // A boundary *or* an `END_WORD` between this phoneme and the next means
        // the next one starts a word, so `nextPhW` must not see through it.
        // (Composed numbers join their parts with `END_WORD` alone, and without
        // this "1km" assimilated across the join: `w'0N k'Il@m,i:t3`.)
        let next_wordstart = real.get(n + 1).is_some_and(|&k| {
            codes
                .get(i + 1..k)
                .unwrap_or_default()
                .iter()
                .any(|c| c.is_boundary || c.code == crate::phoneme::PHON_END_WORD)
        });
        let next_ix = real.get(n + 1).copied();
        // C's `nextVowel` scans forward to the next vowel and gives up at a
        // word boundary; `prevVowel` is the word's previous vowel
        // (`worddata->prev_vowel`).  Both need the whole word, not a window.
        const PH_VOWEL: u8 = 2;
        let crosses = |a: usize, b: usize| {
            codes
                .get(a + 1..b)
                .unwrap_or_default()
                .iter()
                .any(|c| c.is_boundary || c.code == crate::phoneme::PHON_END_WORD)
        };
        let (mut next_vowel, mut next_vowel_stress) = (0u8, NO_STRESS_LEVEL);
        let mut from = i;
        for k in (n + 1)..real.len() {
            let at = real.at(k);
            if crosses(from, at) {
                break;
            }
            let c = codes.code_at(at);
            if phdata.get(c).is_some_and(|p| p.typ == PH_VOWEL) {
                next_vowel = c;
                next_vowel_stress = self.level.at(at);
                break;
            }
            from = at;
        }
        let (mut prev_vowel, mut prev_vowel_stress) = (0u8, NO_STRESS_LEVEL);
        let mut from = i;
        for k in (0..n).rev() {
            let at = real.at(k);
            if crosses(at, from) {
                break;
            }
            let c = codes.code_at(at);
            if phdata.get(c).is_some_and(|p| p.typ == PH_VOWEL) {
                prev_vowel = c;
                prev_vowel_stress = self.level.at(at);
                break;
            }
            from = at;
        }
        crate::synthesize::bytecode::Neighbours {
            prev,
            this: code,
            next,
            next2,
            stress: self.level.at(i),
            next_stress: next_ix.map(|k| self.level.at(k)).unwrap_or(NO_STRESS_LEVEL),
            prev_stress: n
                .checked_sub(1)
                .and_then(|k| real.get(k))
                .map(|&k| self.level.at(k))
                .unwrap_or(NO_STRESS_LEVEL),
            next2_stress: real
                .get(n + 2)
                .map(|&k| self.level.at(k))
                .unwrap_or(NO_STRESS_LEVEL),
            wordstress: self.wordstress.at(i),
            vowel_position: self.vowel_pos.at(i),
            reduce,
            translation_given: given,
            after_stress: self.after_stress.at(n),
            this_wordstart: starts.at(n),
            prev_wordstart: n.checked_sub(1).is_some_and(|k| starts.at(k)),
            next_wordstart,
            next2_wordstart: wordstart(n + 2),
            prev2: n.checked_sub(2).map_or(0, |k| codes.code_at(real.at(k))),
            next3: real.get(n + 3).map_or(0, |&k| codes.code_at(k)),
            next3_wordstart: wordstart(n + 3),
            next_vowel,
            next_vowel_stress,
            prev_vowel,
            prev_vowel_stress,
            vowels_tracked: true,
            prevoicing: false,
        }
    }
}

fn resolve_virtual_phonemes_in_codes(
    codes: &mut Vec<PhonemeCode>,
    phdata: &PhonemeData,
    from_dict: &[bool],
    reduce: u32,
) {
    // (The levels are those before any change.)
    let context = ProgramContext::new(codes, phdata);
    let mut deleted: Vec<usize> = Vec::new();
    // What the previous phoneme appends after itself: C inserts it into the
    // list there and then, so this phoneme's program sees it as `prevPh` —
    // Hindi `i` appends `;`, which makes a following `V` palatal `@3`.
    let mut appended: Option<u8> = None;
    for (n, &i) in context.real.iter().enumerate() {
        let code = codes.code_at(i);
        let prev_append = appended.take();
        let Some(ph) = phdata.get(code) else { continue };
        if ph.program == 0 {
            continue;
        }
        // (Consonant changes apply here too — `l` → `l/2` in "world".  They are
        // invisible in `-x` because espeak drops a mnemonic's `/` variant
        // indicator when printing, which is why upstream shows a plain `l`.)
        let given = from_dict.get(i).copied().unwrap_or(false);
        let mut nb = context.neighbours(codes, phdata, n, given, reduce);
        if let Some(a) = prev_append.filter(|_| !nb.this_wordstart) {
            nb.prev2 = nb.prev;
            nb.prev = a;
        }
        let extract = crate::synthesize::bytecode::interpret_phoneme_ctl(
            ph.program,
            &phdata.phonindex,
            &nb,
            |c| phdata.get(c).cloned(),
            true,
        );
        let next_vowel = context
            .real
            .get(n + 1)
            .and_then(|&k| phdata.get(codes.code_at(k)))
            .is_some_and(|p| p.typ == crate::phoneme::PH_VOWEL);
        appended = extract
            .append_phoneme
            .or(extract.append_if_next_vowel.filter(|_| next_vowel))
            .filter(|&c| c != 0);
        if let Some(changed) = extract.change_phoneme_code.filter(|&c| c != 0 && c != code) {
            if changed == 1 {
                // The NULL phoneme: `ChangePhoneme(NULL)` deletes this phoneme.
                deleted.push(i);
            } else {
                codes.set_code(i, changed);
            }
        }
    }
    for &i in deleted.iter().rev() {
        codes.remove(i);
    }
}

/// A single phoneme event in the synthesizer's input stream.
/// The last letters of the word before `token_ix`, if a space is all that
/// separates them, NUL-padded.
fn previous_word_end(tokens: &[Token], token_ix: usize, lang: &str) -> [u8; 8] {
    let mut out = [0u8; 8];
    let word = match (
        token_ix.checked_sub(1).and_then(|k| tokens.get(k)),
        token_ix.checked_sub(2).and_then(|k| tokens.get(k)),
    ) {
        (Some(Token::Space), Some(Token::Word(w))) => lowercase_for_lang(w, lang),
        // C puts a space between a number and the letters glued to it ("80s"
        // is `80 s`), but two after a number and a space.
        (Some(Token::Number(n)), _) => n.surface().to_string(),
        (None, _) => match SWITCH_PREV.with(|p| p.borrow().clone()) {
            Some(prev) => prev,
            None => return out,
        },
        _ => return out,
    };
    let bytes = word.as_bytes();
    let keep = (0..=bytes.len().min(out.len() - 1))
        .rev()
        .find(|&n| word.is_char_boundary(bytes.len() - n))
        .unwrap_or(0);
    if let Some(tail) = bytes.get(bytes.len() - keep..) {
        if let Some(to) = out.get_mut(..keep) {
            to.copy_from_slice(tail);
        }
    }
    out
}

/// `MakePhonemeList`: a switch back to the clause's language that is
/// followed by another switch, or by a pause and a switch, is dropped, and so
/// is a switch to the table already in use — French spelling "привет" stays
/// in English, `_:(en)sI2r'IlIk_:p'E: _:sI2r'IlIk_:'Er …(fr)`.
fn collapse_language_switches(codes: &mut Vec<PhonemeCode>, lang: &str) {
    let switch_at =
        |codes: &[PhonemeCode], k: usize| match codes.get(k).and_then(|c| c.marker.as_ref()) {
            Some(CodeMarker::LangSwitch(t)) => Some(t.clone()),
            _ => None,
        };
    // The next index from `k` that isn't a word boundary (or an unspoken
    // punctuation mark).
    let skip = |codes: &[PhonemeCode], mut k: usize| {
        while codes.get(k).is_some_and(|c| {
            (c.is_boundary && c.code == PHON_END_WORD)
                || matches!(c.marker, Some(CodeMarker::Punctuation(_)))
        }) {
            k += 1;
        }
        k
    };
    let mut open: Option<String> = None;
    let mut i = 0;
    while i < codes.len() {
        if codes.get(i).is_some_and(|c| c.is_boundary && c.code == 0) {
            open = None;
        }
        let Some(target) = switch_at(codes, i) else {
            i += 1;
            continue;
        };
        if target != lang {
            if open.as_deref() == Some(target.as_str()) {
                codes.remove(i);
                continue;
            }
            open = Some(target);
            i += 1;
            continue;
        }
        let Some(current) = open.clone() else {
            i += 1;
            continue;
        };
        let j = skip(codes, i + 1);
        if switch_at(codes, j).is_some() {
            codes.remove(i);
            continue;
        }
        let paused = codes.get(j).is_some_and(|c| {
            c.marker.is_none() && !c.is_boundary && c.code == crate::phoneme::PHON_PAUSE
        });
        let k = skip(codes, j + 1);
        if paused && switch_at(codes, k).as_deref() == Some(current.as_str()) {
            codes.remove(k);
            codes.remove(i);
            continue;
        }
        open = None;
        i += 1;
    }
}

/// Hakka: a first tone before a first, fourth or sixth tone in the same word
/// becomes the second (`CalcPitches_Tone`, after Sixian's sandhi).
fn hakka_tone_sandhi(codes: &mut [PhonemeCode], phdata: &PhonemeData) {
    let second = phdata.lookup_phoneme("2");
    if second == 0 {
        return;
    }
    let mnemonic = |c: &PhonemeCode| phdata.get(c.code).map_or(0, |ph| ph.mnemonic);
    let one = u32::from(b'1');
    let follows = [one, u32::from(b'4'), u32::from(b'6')];
    // The previous syllable's tone in this word.
    let mut prev: Option<usize> = None;
    for k in 0..codes.len() {
        let Some(c) = codes.get(k) else { break };
        if c.is_boundary || (c.marker.is_none() && c.code == PHON_END_WORD) {
            prev = None;
            continue;
        }
        let tone = match c.marker {
            Some(CodeMarker::ClauseTone) => true,
            None => is_tone_phoneme(c.code, phdata),
            Some(_) => false,
        };
        if !tone {
            continue;
        }
        let this = mnemonic(c);
        if let Some(p) = prev {
            if codes.get(p).map(mnemonic) == Some(one) && follows.contains(&this) {
                set_code(codes, p, second);
            }
        }
        prev = Some(k);
    }
}

fn set_code(codes: &mut [PhonemeCode], at: usize, code: u8) {
    if let Some(c) = codes.get_mut(at) {
        c.code = code;
    }
}

/// Mandarin's tone sandhi over the whole clause (`CalcPitches_Tone`), for what
/// the dictionary pass can't see: a word in another language, a spelled
/// letter, and the tones either side of them.  A vowel with no tone takes `55`
/// after a pause (and after another `55` given this way), otherwise the
/// neutral `11`, which is unstressed and levelled by the tone before it; a
/// `214` becomes `35` before another and `21` before anything else; `51` +
/// `51` in a word becomes `53` + `51`.
fn mandarin_tones_in_switched(
    codes: &mut Vec<PhonemeCode>,
    phdata: &PhonemeData,
    lang: &str,
    switched_vowel: &dyn Fn(&str, u8) -> bool,
) {
    let level = |prev: &str| match prev {
        "55" => "22",
        "35" => "33",
        "214" => "44",
        _ => "",
    };
    let mut pause = true;
    let mut promoted = false;
    // The previous syllable's tone across words (`prevw_tph`) and in this
    // word (`prev_tph`): where its code is, and its name.
    let mut prevw: Option<(usize, String)> = None;
    let mut prev_in_word: Option<(usize, String)> = None;
    let mut switched: Option<String> = None;
    let mut i = 0;
    while i < codes.len() {
        let Some(c) = codes.get(i).cloned() else {
            break;
        };
        if let Some(CodeMarker::LangSwitch(t)) = &c.marker {
            switched = (t != lang).then(|| t.clone());
            i += 1;
            continue;
        }
        if c.is_boundary && c.code == 0 {
            pause = true;
            prevw = None;
            prev_in_word = None;
            i += 1;
            continue;
        }
        if c.is_boundary || (c.marker.is_none() && c.code == PHON_END_WORD) {
            prev_in_word = None;
            i += 1;
            continue;
        }
        if c.marker.is_some() {
            i += 1;
            continue;
        }
        if switched.is_none()
            && phdata
                .get(c.code)
                .is_some_and(|ph| ph.typ == crate::phoneme::PH_PAUSE && ph.std_length > 50)
        {
            pause = true;
            prevw = None;
        }
        let vowel = match &switched {
            Some(t) => switched_vowel(t, c.code),
            None => phdata
                .get(c.code)
                .is_some_and(|ph| ph.typ == crate::phoneme::PH_VOWEL),
        };
        if !vowel {
            i += 1;
            continue;
        }
        let mut at = i + 1;
        while codes.get(at).is_some_and(|n| {
            !n.is_boundary && n.marker.is_none() && n.code == crate::phoneme::PHON_LENGTHEN
        }) {
            at += 1;
        }
        let given = codes.get(at).cloned().filter(|n| {
            !n.is_boundary
                && match n.marker {
                    Some(CodeMarker::ClauseTone) => true,
                    None => switched.is_none() && is_tone_phoneme(n.code, phdata),
                    Some(_) => false,
                }
        });
        let tone = if let Some(n) = given {
            promoted = false;
            phoneme_mnemonic(n.code, phdata)
        } else {
            let name = if pause || promoted { "55" } else { "11" };
            let code = phdata.lookup_phoneme(name);
            if code == 0 {
                i = at;
                continue;
            }
            promoted = name == "55";
            codes.insert(
                at,
                PhonemeCode {
                    code,
                    is_boundary: false,
                    clause_char: None,
                    marker: switched.is_some().then_some(CodeMarker::ClauseTone),
                },
            );
            name.to_string()
        };
        if let Some((p, prev)) = &prevw {
            if prev == "214" {
                let new = phdata.lookup_phoneme(if tone == "214" { "35" } else { "21" });
                if new != 0 {
                    set_code(codes, *p, new);
                }
            }
        }
        if let Some((p, prev)) = &prev_in_word {
            if prev == "51" && tone == "51" {
                let new = phdata.lookup_phoneme("53");
                if new != 0 {
                    set_code(codes, *p, new);
                }
            }
        }
        let mut at = at;
        if tone == "11" {
            let new = prevw
                .as_ref()
                .map_or(0, |(_, p)| phdata.lookup_phoneme(level(p)));
            if new != 0 {
                set_code(codes, at, new);
            }
            // The neutral tone is unstressed: drop the syllable's stress marks.
            let mut k = i;
            while k > 0 {
                let Some(p) = codes.get(k - 1).cloned() else {
                    break;
                };
                if p.marker.is_some() || p.is_boundary {
                    break;
                }
                let is_vowel = match &switched {
                    Some(t) => switched_vowel(t, p.code),
                    None => phdata
                        .get(p.code)
                        .is_some_and(|ph| ph.typ == crate::phoneme::PH_VOWEL),
                };
                if is_vowel || (switched.is_none() && is_tone_phoneme(p.code, phdata)) {
                    break;
                }
                if matches!(
                    p.code,
                    PHON_STRESS_2
                        | PHON_STRESS_3
                        | PHON_STRESS_P
                        | PHON_STRESS_P2
                        | PHON_STRESS_TONIC
                ) {
                    codes.remove(k - 1);
                    at -= 1;
                }
                k -= 1;
            }
        }
        prevw = Some((at, tone.clone()));
        prev_in_word = Some((at, tone));
        pause = false;
        i = at + 1;
    }
}

/// C's `derived_letters`: a superscript or subscript and its base letter
/// (`IsSuperscript`), read as that letter.
const DERIVED_LETTERS: &[(char, char)] = &[
    ('\u{aa}', 'a'),
    ('\u{b2}', '2'),
    ('\u{b3}', '3'),
    ('\u{b9}', '1'),
    ('\u{ba}', 'o'),
    ('\u{2b0}', 'h'),
    ('\u{2b1}', '\u{266}'),
    ('\u{2b2}', 'j'),
    ('\u{2b3}', 'r'),
    ('\u{2b4}', '\u{279}'),
    ('\u{2b5}', '\u{27b}'),
    ('\u{2b6}', '\u{281}'),
    ('\u{2b7}', 'w'),
    ('\u{2b8}', 'y'),
    ('\u{2c0}', '\u{294}'),
    ('\u{2c1}', '\u{295}'),
    ('\u{2e0}', '\u{263}'),
    ('\u{2e1}', 'l'),
    ('\u{2e2}', 's'),
    ('\u{2e3}', 'x'),
    ('\u{2070}', '0'),
    ('\u{2071}', 'i'),
    ('\u{2074}', '4'),
    ('\u{2075}', '5'),
    ('\u{2076}', '6'),
    ('\u{2077}', '7'),
    ('\u{2078}', '8'),
    ('\u{2079}', '9'),
    ('\u{207a}', '+'),
    ('\u{207b}', '-'),
    ('\u{207c}', '='),
    ('\u{207d}', '('),
    ('\u{207e}', ')'),
    ('\u{207f}', 'n'),
    ('\u{2080}', '0'),
    ('\u{2081}', '1'),
    ('\u{2082}', '2'),
    ('\u{2083}', '3'),
    ('\u{2084}', '4'),
    ('\u{2085}', '5'),
    ('\u{2086}', '6'),
    ('\u{2087}', '7'),
    ('\u{2088}', '8'),
    ('\u{2089}', '9'),
    ('\u{208a}', '+'),
    ('\u{208b}', '-'),
    ('\u{208c}', '='),
    ('\u{208d}', '('),
    ('\u{208e}', ')'),
    ('\u{2090}', 'a'),
    ('\u{2091}', 'e'),
    ('\u{2092}', 'o'),
    ('\u{2093}', 'x'),
    ('\u{2094}', '\u{259}'),
    ('\u{2095}', 'h'),
    ('\u{2096}', 'k'),
    ('\u{2097}', 'l'),
    ('\u{2098}', 'm'),
    ('\u{2099}', 'n'),
    ('\u{209a}', 'p'),
    ('\u{209b}', 's'),
    ('\u{209c}', 't'),
];

/// A braille pattern, which C reads as a letter ("braille 1 2") — and the
/// characters of C's 0xa700–0xd7ff "Korean" alphabet, which its `IsAlpha`
/// counts as letters whatever they are (a Kayah Li digit).
fn is_braille(c: char) -> bool {
    ('\u{2800}'..='\u{28ff}').contains(&c) || ('\u{a700}'..='\u{d7ff}').contains(&c)
}

/// Stress a word as C does with another language's phoneme table left
/// selected (`STALE_TABLE`): a phoneme that table lacks is a schwa, and the
/// vowels are the ones it knows.
fn restress_with_stale_table(
    phonemes: &mut Vec<u8>,
    lang: &str,
    dict_flags: u32,
    stress_opts: &StressOpts,
    options: &LangOptions,
) {
    let _ = options;
    let Some(data_dir) = DATA_DIR.with(|d| d.borrow().clone()) else {
        return;
    };
    let Ok(mut stale) = PhonemeData::load(&data_dir) else {
        return;
    };
    if select_phoneme_table(&mut stale, &data_dir, lang).is_err() {
        return;
    }
    let mut ph: Vec<u8> = phonemes
        .iter()
        .copied()
        .take_while(|&b| b != 0)
        // C's input to `SetWordStress` has no stress yet.
        // (An unstressed mark is the dictionary's own, and stays: Spanish
        // `y %i`.)
        .filter(|&b| !matches!(b, PHON_STRESS_2..=PHON_STRESS_P2 | PHON_STRESS_TONIC))
        .map(|b| {
            if b > 8 && b != PHON_END_WORD && stale.get(b).is_none() {
                13
            } else {
                b
            }
        })
        .collect();
    if ph.is_empty() {
        return;
    }
    ph.push(0);
    set_word_stress(&mut ph, &stale, stress_opts, Some(dict_flags), -1, 0);
    *phonemes = ph;
}

thread_local! {
    /// Phoneme tables loaded by name, for reading codes against a table C has
    /// switched to without a translator (`fa`, `base`).
    static TABLES: std::cell::RefCell<std::collections::HashMap<String, std::rc::Rc<PhonemeData>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    /// Codes C reported as "Invalid phoneme code", in order, for the CLI.
    static INVALID_CODES: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The phoneme table `name`, loaded from the translation's data directory.
fn table_by_name(name: &str) -> Option<std::rc::Rc<PhonemeData>> {
    if let Some(t) = TABLES.with(|t| t.borrow().get(name).cloned()) {
        return Some(t);
    }
    let dir = DATA_DIR.with(|d| d.borrow().clone())?;
    let mut data = PhonemeData::load(&dir).ok()?;
    data.select_table_by_name(name).ok()?;
    let data = std::rc::Rc::new(data);
    TABLES.with(|t| t.borrow_mut().insert(name.to_string(), data.clone()));
    Some(data)
}

/// The type of `code` in the table `name`, when no translator has it.
fn embedded_table_type(name: &str, code: u8) -> Option<u8> {
    table_by_name(name).and_then(|t| t.get(code).map(|ph| ph.typ))
}

/// Codes C drops reading `ph` against the table `name`, reporting each.  A
/// stress mark whose vowel went with them goes too.
fn drop_invalid_codes(ph: &[u8], name: &str) -> Vec<u8> {
    let Some(t) = table_by_name(name) else {
        return ph.to_vec();
    };
    let mut out: Vec<u8> = ph
        .iter()
        .copied()
        .filter(|&b| {
            let ok = b <= 8 || b == PHON_END_WORD || t.get(b).is_some();
            if !ok {
                INVALID_CODES.with(|c| c.borrow_mut().push(b));
            }
            ok
        })
        .collect();
    drop_vowelless_stress_marks(&mut out, &t);
    out
}

/// Remove stress marks with no vowel after them in their word, by `table`.
fn drop_vowelless_stress_marks(ph: &mut Vec<u8>, table: &PhonemeData) {
    let mut k = 0;
    while k < ph.len() {
        let b = ph.get(k).copied().unwrap_or(0);
        if matches!(b, PHON_STRESS_U..=PHON_STRESS_P2 | PHON_STRESS_TONIC) {
            let vowel_after = ph
                .get(k + 1..)
                .unwrap_or_default()
                .iter()
                .take_while(|&&c| c != PHON_END_WORD && c != 0)
                .any(|&c| {
                    table
                        .get(c)
                        .is_some_and(|p| p.typ == crate::phoneme::PH_VOWEL)
                });
            if !vowel_after {
                ph.remove(k);
                continue;
            }
        }
        k += 1;
    }
}

/// The codes C reported as "Invalid phoneme code" since the last call: it
/// prints one line for each on standard output.
pub fn take_invalid_phoneme_codes() -> Vec<u8> {
    INVALID_CODES.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// C's `TranslateRoman`: a word of Roman numerals read as its number, after
/// the language's word for "Roman" (before it with `NUM_ROMAN_AFTER`).
fn translate_roman(
    word: &str,
    prev_digit: bool,
    has_dot: bool,
    dot_ordinal: bool,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    options: &LangOptions,
) -> Option<Vec<u8>> {
    const NUM_ROMAN: u32 = 0x0100_0000;
    const NUM_ROMAN_CAPITALS: u32 = 0x0200_0000;
    const NUM_ROMAN_AFTER: u32 = 0x0400_0000;
    const NUM_ROMAN_ORDINAL: u32 = 0x0800_0000;
    const NUM_ORDINAL_DOT: u32 = 0x0001_0000;
    let grammar = &options.number_grammar;
    let numbers = grammar.flags.map_or(0, |f| f.numbers());
    let letters: Vec<char> = word.chars().collect();
    let all_upper = letters.len() > 1 && letters.iter().all(|c| c.is_uppercase())
        || letters.len() == 1 && letters.iter().all(|c| c.is_uppercase());
    let all_lower = letters.iter().all(|c| c.is_lowercase());
    // Either all upper or all lower case.
    if !all_upper && !all_lower {
        return None;
    }
    if numbers & NUM_ROMAN == 0 && !(numbers & NUM_ROMAN_CAPITALS != 0 && all_upper) {
        return None;
    }
    if (numbers & NUM_ROMAN_CAPITALS != 0 && !all_upper) || prev_digit {
        return None;
    }
    if letters.len() == 1
        && !(numbers & (NUM_ROMAN_CAPITALS | NUM_ROMAN_ORDINAL | NUM_ORDINAL_DOT) != 0 && has_dot)
    {
        return None;
    }
    const DIGITS: &[(char, u32)] = &[
        ('i', 1),
        ('x', 10),
        ('c', 100),
        ('m', 1000),
        ('v', 5),
        ('l', 50),
        ('d', 500),
    ];
    let (mut acc, mut prev, mut subtract, mut repeat) = (0u32, 0u32, 0x7fffu32, 0);
    for c in word.to_lowercase().chars() {
        let mut value = DIGITS.iter().find(|d| d.0 == c)?.1;
        if value == prev {
            repeat += 1;
            if repeat >= 3 {
                return None;
            }
        } else {
            repeat = 0;
        }
        if prev > 1 && prev != 10 && prev != 100 && value >= prev {
            return None;
        }
        if prev != 0 && prev < value {
            if acc % 10 != 0 || prev * 10 < value {
                return None;
            }
            subtract = prev;
            value -= subtract;
        } else if value >= subtract {
            return None;
        } else {
            acc += prev;
        }
        prev = value;
    }
    acc += prev;
    let (min, max) = match primary_bcp47_subtag(&options.lang) {
        "hu" => (1, 899),
        "la" | "lt" | "pt" => (2, 5000),
        _ => (2, 49),
    };
    if acc < min || acc > max {
        return None;
    }
    let roman = trim_lookup(&lookup_num_phonemes(dict, "_roman")).to_vec();
    // C's `CheckDotOrdinal`: a dot that doesn't end the clause.
    let dot_ordinal = numbers & NUM_ORDINAL_DOT != 0 && dot_ordinal;
    let lang = primary_bcp47_subtag(&options.lang);
    let ordinal = if numbers & NUM_ROMAN_ORDINAL != 0 {
        // Hungarian's Roman numbers are ordinal only with the dot.
        if lang == "hu" && !dot_ordinal {
            return None;
        }
        true
    } else {
        dot_ordinal
    };
    let token = if ordinal {
        NumberToken::Ordinal(OrdinalNumber {
            digits: acc.to_string(),
            // `langopts.roman_suffix`, the masculine ordinal sign.
            marker: if matches!(lang, "it" | "pap") && numbers & NUM_ROMAN_ORDINAL != 0 {
                OrdinalMarker::Suffix("º".to_string())
            } else {
                OrdinalMarker::Dot
            },
        })
    } else {
        NumberToken::Cardinal(acc.to_string())
    };
    // (As the number's entries give it: the whole is stressed together.)
    let number = match token {
        NumberToken::Ordinal(_) => {
            translate_number_token(&token, dict, phdata, stress_opts, grammar)?.phonemes
        }
        _ => number_token_to_phonemes(&token, dict, grammar)?,
    };
    let number = trim_lookup(&number).to_vec();
    let mut out = Vec::new();
    if numbers & NUM_ROMAN_AFTER == 0 {
        out.extend(&roman);
        if !roman.is_empty() {
            out.push(PHON_END_WORD);
        }
        out.extend(&number);
    } else {
        out.extend(&number);
        if !roman.is_empty() {
            out.push(PHON_END_WORD);
        }
        out.extend(&roman);
    }
    out.push(0);
    set_word_stress(&mut out, phdata, stress_opts, Some(0), -1, 0);
    Some(out)
}

/// C's `ALPHABET` table: name key, offset, range, language, flags.
const ALPHABETS: &[(&str, u32, u32, u32, &str, u32)] = &[
    (
        "_el",
        0x370,
        0x380,
        0x3ff,
        "el",
        AL_DONT_NAME | AL_NOT_LETTERS | AL_WORDS,
    ),
    ("_cyr", 0x420, 0x400, 0x52f, "", 0),
    ("_hy", 0x500, 0x530, 0x58f, "hy", AL_WORDS),
    ("_he", 0x580, 0x590, 0x5ff, "he", 0),
    ("_ar", 0x600, 0x600, 0x6ff, "ar", AL_WORDS),
    ("_syc", 0x700, 0x700, 0x74f, "", 0),
    ("_hi", 0x900, 0x900, 0x97f, "hi", AL_WORDS),
    ("_bn", 0x980, 0x980, 0x9ff, "bn", AL_WORDS),
    ("_gur", 0xa00, 0xa00, 0xa7f, "pa", AL_WORDS),
    ("_gu", 0xa80, 0xa80, 0xaff, "gu", AL_WORDS),
    ("_or", 0xb00, 0xb00, 0xb7f, "", 0),
    ("_ta", 0xb80, 0xb80, 0xbff, "ta", AL_WORDS),
    ("_te", 0xc00, 0xc00, 0xc7f, "te", 0),
    ("_kn", 0xc80, 0xc80, 0xcff, "kn", AL_WORDS),
    ("_ml", 0xd00, 0xd00, 0xd7f, "ml", AL_WORDS),
    ("_si", 0xd80, 0xd80, 0xdff, "si", AL_WORDS),
    ("_th", 0xe00, 0xe00, 0xe7f, "", 0),
    ("_lo", 0xe80, 0xe80, 0xeff, "", 0),
    ("_ti", 0xf00, 0xf00, 0xfff, "", 0),
    ("_my", 0x1000, 0x1000, 0x109f, "", 0),
    ("_ka", 0x1080, 0x10a0, 0x10ff, "ka", AL_WORDS),
    ("_ko", 0x1100, 0x1100, 0x11ff, "ko", AL_WORDS),
    ("_eth", 0x1200, 0x1200, 0x139f, "", 0),
    ("_braille", 0x2800, 0x2800, 0x28ff, "", AL_NO_SYMBOL),
    ("_ja", 0x3040, 0x3040, 0x30ff, "", AL_NOT_CODE),
    (
        "_zh",
        0x3100,
        0x3100,
        0x9fff,
        "",
        AL_NOT_CODE | AL_IDEOGRAPHS,
    ),
    ("_ko", 0xa700, 0xa700, 0xd7ff, "ko", AL_NOT_CODE | AL_WORDS),
    ("_shaw", 0x10450, 0x10450, 0x1047f, "en", 0),
];
const AL_DONT_NAME: u32 = 0x01;
const AL_NOT_LETTERS: u32 = 0x02;
const AL_WORDS: u32 = 0x04;
const AL_NOT_CODE: u32 = 0x08;
const AL_NO_SYMBOL: u32 = 0x10;
const AL_IDEOGRAPHS: u32 = 0x20;

/// `AlphabetFromChar`: name key, offset, language and flags.
fn alphabet_from_char(c: char) -> Option<(&'static str, u32, &'static str, u32)> {
    let c = c as u32;
    ALPHABETS
        .iter()
        .find(|a| (a.2..=a.3).contains(&c))
        .map(|a| (a.0, a.1, a.4, a.5))
}

/// `langopts.alt_alphabet` and `alt_alphabet_lang`.
fn alt_alphabet(lang: &str) -> (u32, &'static str) {
    match primary_bcp47_subtag(lang) {
        "ar" => (0x3100, "cmn"),
        "ka" => (0x420, "ru"),
        _ => (0, ""),
    }
}

/// `langopts.our_alphabet`: a script the language names its letters in
/// without saying which alphabet they are.
fn our_alphabet(lang: &str) -> u32 {
    match primary_bcp47_subtag(lang) {
        "hr" | "bs" | "sr" => 0x420,
        "ko" => 0xa700,
        "cmn" | "yue" | "zh" => 0x3100,
        _ => 0,
    }
}

/// C's `TranslateLetter` for a character this language has no name for: the
/// alphabet's name (here, or in English), then the letter in its alphabet's
/// language, or "letter" and the character's code.
fn letter_parts(
    c: char,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    options: &LangOptions,
) -> Vec<LetterPart> {
    let lang = &options.lang;
    let base = primary_bcp47_subtag(lang);
    let alphabet = alphabet_from_char(c);
    let (al_offset, al_flags) = alphabet.map_or((0, 0), |a| (a.1, a.3));
    let (alt_offset, alt_lang) = alt_alphabet(lang);
    // A superscript or subscript letter is read as its base letter.
    if let Some(&(_, base)) = DERIVED_LETTERS.iter().find(|&&(d, _)| d == c) {
        // (Read as that letter would be: a Latin one in Russian is English.)
        if dict.letter_bits_offset == 0 {
            if let Some(name) =
                letter_name(&base.to_string(), false, dict, phdata, stress_opts, options)
            {
                return vec![LetterPart::Local(trim_lookup(&name).to_vec())];
            }
        }
        return letter_parts(base, dict, phdata, stress_opts, options);
    }
    // A letter of an alphabet C reads as words is switched to its language,
    // and is nothing if that language can't read it (the 0xa700 "Korean"
    // range takes in Javanese, Phags-pa, …).
    if al_flags & AL_WORDS != 0 && al_offset != dict.letter_bits_offset {
        // …and C leaves that language's phoneme table selected (see
        // `STALE_TABLE`).
        if let Some((_, _, l, _)) = alphabet.filter(|a| !a.2.is_empty() && a.2 != base) {
            STALE_TABLE.with(|t| *t.borrow_mut() = Some(l.to_string()));
        }
        return Vec::new();
    }
    let mut parts: Vec<LetterPart> = alphabet_name_part(c, dict, lang).into_iter().collect();
    // "letter" (or "symbol") and the code.
    let mut rest: Vec<u8> = Vec::new();
    if al_flags & AL_NO_SYMBOL == 0 {
        let mut found = Vec::new();
        if c.is_alphabetic() {
            found = trim_lookup(&lookup_num_phonemes(dict, "_?A")).to_vec();
        }
        if found.is_empty() && !c.is_whitespace() {
            found = trim_lookup(&lookup_num_phonemes(dict, "_??")).to_vec();
        }
        if found.is_empty() {
            found = parse_inline_phonemes("l'et@", phdata);
            found.retain(|&b| b != 0);
        }
        rest.extend(found);
    }
    if al_flags & AL_NOT_CODE == 0 {
        const HEX_LETTERS: [&str; 6] = ["'e:j", "b'i:", "s'i:", "d'i:", "'i:", "'ef"];
        let code = c as u32;
        let digits: String = if al_offset == 0x2800 {
            ('1'..='8')
                .enumerate()
                .filter(|&(ix, _)| code & (1 << ix) != 0)
                .map(|(_, d)| d)
                .collect()
        } else {
            format!("{code:x}")
        };
        for d in digits.chars() {
            rest.push(crate::phoneme::PHON_PAUSE_VSHORT);
            let name = letter_name(&d.to_string(), true, dict, phdata, stress_opts, options)
                .filter(|n| n.first() != Some(&crate::phoneme::PHON_SWITCH));
            match name {
                Some(n) => {
                    // A tone belongs to the vowel before it.
                    let mut n = trim_lookup(&n).to_vec();
                    reorder_tone_phonemes(&mut n, phdata);
                    rest.extend(n);
                }
                None => {
                    let hex = u32::from(d)
                        .checked_sub(u32::from('a'))
                        .and_then(|k| HEX_LETTERS.get(k as usize));
                    if let Some(hex) = hex {
                        let mut ph = parse_inline_phonemes(hex, phdata);
                        ph.retain(|&b| b != 0);
                        rest.extend(ph);
                    }
                }
            }
        }
        rest.push(crate::phoneme::PHON_PAUSE);
    }
    // The letter in its alphabet's language, if that isn't this one.
    let language = if al_offset != 0 && al_offset == alt_offset {
        alt_lang
    } else {
        match alphabet {
            Some((_, _, l, flags)) if !l.is_empty() && flags & AL_NOT_LETTERS == 0 => l,
            _ => "en",
        }
    };
    if language != base || language == "ko" {
        parts.push(LetterPart::Letter {
            lang: language.to_string(),
            letter: c,
            otherwise: rest,
        });
    } else {
        parts.push(LetterPart::Local(rest));
    }
    parts
}

/// The name of a letter's alphabet, said before it when it isn't this
/// language's own: here if this language has one, otherwise in English.
fn alphabet_name_part(c: char, dict: &Dictionary, lang: &str) -> Option<LetterPart> {
    let (name, offset, _, flags) = alphabet_from_char(c)?;
    let named = flags & AL_DONT_NAME == 0
        && offset != dict.letter_bits_offset
        && offset != alt_alphabet(lang).0
        && offset != our_alphabet(lang);
    if !named {
        return None;
    }
    let local = lookup_num_phonemes(dict, name);
    let local = trim_lookup(&local);
    Some(if local.is_empty() {
        LetterPart::Key {
            lang: "en".to_string(),
            key: name.to_string(),
        }
    } else {
        LetterPart::Local(local.to_vec())
    })
}

/// A [`TranslateEntryKind::Letter`] entry for these parts.
fn letter_entry_of(parts: Vec<LetterPart>, word_lower: String) -> TranslateEntry {
    let phonemes: Vec<u8> = parts
        .iter()
        .flat_map(|p| match p {
            LetterPart::Local(ph) | LetterPart::Letter { otherwise: ph, .. } => ph.clone(),
            LetterPart::Key { .. } | LetterPart::Raw { .. } | LetterPart::Break => Vec::new(),
        })
        .collect();
    TranslateEntry {
        phonemes,
        dict_flags: 0,
        kind: TranslateEntryKind::Letter(parts),
        word_lower: Some(word_lower),
        no_word_gap: false,
        found_in_list: true,
    }
}

/// `$double` (C's `FLAG_DOUBLING`).
const FLAG_DOUBLING: u32 = 0x4000;

/// A switch entry that also carries an alphabet name to say first
/// (`[phonSWITCH, lang…, 0, name…]`).
fn has_alphabet_name(phonemes: &[u8]) -> bool {
    phonemes.iter().position(|&b| b == 0).is_some_and(|k| {
        phonemes
            .get(k + 1..)
            .is_some_and(|rest| rest.iter().any(|&b| b != 0))
    })
}

/// The target language of a `phonSWITCH` entry (`[phonSWITCH, <lang bytes…>]`).
///
/// A bare `phonSWITCH` with no name switches to the default voice: the
/// dictionary compiler omits the name when it equals `ESPEAKNG_DEFAULT_VOICE`.
fn switch_target(phonemes: &[u8]) -> String {
    let target: String = phonemes
        .iter()
        .skip(1)
        .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'-')
        .map(|&b| (b as char).to_ascii_lowercase())
        .collect();
    if target.is_empty() {
        "en".to_string()
    } else {
        target
    }
}

/// Reads and writes of a code list by index, as [`Bounded`].
trait CodeList {
    fn code_at(&self, i: usize) -> u8;
    fn boundary_at(&self, i: usize) -> bool;
    fn set_code(&mut self, i: usize, code: u8);
}

impl CodeList for [PhonemeCode] {
    fn code_at(&self, i: usize) -> u8 {
        debug_assert!(i < self.len());
        self.get(i).map_or(0, |c| c.code)
    }
    fn boundary_at(&self, i: usize) -> bool {
        debug_assert!(i < self.len());
        self.get(i).is_some_and(|c| c.is_boundary)
    }
    fn set_code(&mut self, i: usize, code: u8) {
        debug_assert!(i < self.len());
        if let Some(c) = self.get_mut(i) {
            c.code = code;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PhonemeCode {
    /// espeak-ng phoneme code.  See `synthesize.h` and the phoneme data files.
    pub code: u8,
    /// True if this is a boundary marker (word boundary, clause boundary).
    pub is_boundary: bool,
    /// When this item ends a clause (`Token::ClauseBoundary`), the punctuation
    /// character (`,`, `.`, etc.).  Used for CLI phoneme output formatting.
    pub clause_char: Option<char>,
    /// A non-phoneme marker in the code stream.  The marker itself has
    /// `code == 0` and `is_boundary == false`, so anything walking the codes for
    /// synthesis skips it unless it looks here.
    pub marker: Option<CodeMarker>,
}

/// A marker carried alongside the phoneme codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeMarker {
    /// A `phonSWITCH`: the codes that follow belong to this language's phoneme
    /// table, not the clause's, until the next marker.
    ///
    /// A marker carrying the clause's own language closes the switch —
    /// mirroring how upstream prints `(en)h@l'oU(ru)` and clears `phoneme_tab`
    /// on a table switch.
    LangSwitch(String),
    /// An inline embedded command (`\x01[±]<value><letter>`): a rate, pitch or
    /// amplitude change that applies to everything **after** this point.
    Embedded(EmbeddedCmd),
    /// The code is a tone phoneme of the *clause's* table on a vowel of a word
    /// in another language (C's `tone_ph`, which belongs to no word): the
    /// default tones a Vietnamese or Hakka clause gives an English word.
    ClauseTone,
    /// A punctuation character that is not spoken.  Only the phonemizer-style
    /// IPA output (`preserve_punctuation`) prints it; every other consumer skips
    /// the marker like any other.
    Punctuation(char),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard for the C `SelectTranslator()` "missing break" bug class
    /// (upstream #2476): assert each language's number grammar is its *own*, with
    /// no cross-contamination between neighbours and no leak to unknown languages.
    #[test]
    fn lang_number_grammars_are_independent() {
        let g = |lang: &str| LangOptions::for_lang(lang).number_grammar;

        // Each language gets its specific distinguishing config …
        assert!(
            g("en").hundreds.use_conjunction_with_remainder,
            "en: 'and' before remainder"
        );
        assert_eq!(
            g("es").tens,
            TensGrammar::WithConjunction,
            "es: 'treinta y cuatro'"
        );
        assert!(g("es").hundreds.omit_one_prefix);
        assert!(g("fr").hundreds.omit_one_prefix, "fr: 'cent' not 'un cent'");
        assert_eq!(
            g("de").tens,
            TensGrammar::UnitsThenConjunction,
            "de: 'vier und dreißig'"
        );
        assert!(g("de").ordinals.dot_marks_ordinal, "de: '3.' is ordinal");

        // … and does NOT inherit a neighbour's config.
        assert!(
            !g("fr").ordinals.dot_marks_ordinal,
            "fr must not take de's dot-ordinal"
        );
        assert_ne!(
            g("fr").tens,
            TensGrammar::UnitsThenConjunction,
            "fr must not take de's tens order"
        );
        assert_ne!(
            g("en").tens,
            TensGrammar::WithConjunction,
            "en must not take es's tens conjunction"
        );
        // English keeps the "one" before hundred/thousand; de/fr/es/ru omit it.
        assert!(!g("en").hundreds.omit_one_prefix, "en keeps 'one hundred'");
        assert!(
            !g("en").thousands.omit_one_prefix,
            "en keeps 'one thousand'"
        );
        // German keeps its "ein" (upstream sets neither `NUM_OMIT_1_HUNDRED` nor
        // `NUM_OMIT_1_THOUSAND`), but takes the combining `_1a` form before a
        // scale word: 100 is "einhundert", 1000 "ein tausend", 1 "eins".
        assert!(
            !g("de").hundreds.omit_one_prefix && !g("de").thousands.omit_one_prefix,
            "de keeps 'ein'"
        );
        assert!(
            g("es").hundreds.omit_one_prefix && g("es").thousands.omit_one_prefix,
            "es: 'cien'/'mil'"
        );

        // Languages sharing a match arm are intentionally identical in their
        // *number* config.  nl additionally carries the Dutch-IJ tokenizer flag
        // and the Germanic compound-ordinal flag (eenentwintigste), both
        // Dutch-only, so normalise them out before comparing to Maltese.
        assert!(
            g("nl").ordinals.compound_cardinal_suffix,
            "nl: compound ordinals suffix the cardinal"
        );
        assert!(!g("mt").ordinals.compound_cardinal_suffix, "mt: does not");
        let mut nl = g("nl");
        nl.dutch_ij = false;
        nl.ordinals.compound_cardinal_suffix = false;
        // Dutch is also one of C's `NUM_1900` languages ("negentienhonderd"),
        // Maltese is not.
        assert!(
            g("nl").year_1900 && !g("mt").year_1900,
            "nl reads 19xx as a year"
        );
        nl.year_1900 = false;
        nl.flags = g("mt").flags;
        // Dutch has C's `NUM_ORDINAL_DOT`, Maltese has not.
        assert!(nl.ordinals.dot_marks_ordinal && !g("mt").ordinals.dot_marks_ordinal);
        nl.ordinals.dot_marks_ordinal = false;
        assert_eq!(
            nl,
            g("mt"),
            "nl and mt share the same number arm (apart from Dutch-only flags)"
        );
        // Danish/Faroese/Slovenian are units-first ("enogtyve"), so they differ
        // from the tens-first Finnish/Estonian they used to share an arm with.
        assert_eq!(
            g("da").tens,
            TensGrammar::UnitsThenConjunction,
            "da: 'enogtyve'"
        );
        assert!(
            g("da").ordinals.compound_cardinal_suffix,
            "da: compound ordinals suffix the cardinal"
        );
        assert_eq!(
            g("fi").tens,
            TensGrammar::Standard,
            "fi: tens-first 'kaksikymmentäyksi'"
        );
        assert_ne!(
            g("da"),
            g("fi"),
            "da (units-first) must differ from fi (tens-first)"
        );
        assert_eq!(g("fi"), g("et"), "fi and et still share the tens-first arm");
        // … but distinct from an unrelated arm.
        assert_ne!(g("nl"), g("de"), "nl (own arm) differs from de");
        assert_ne!(g("da"), g("nl"), "da differs from nl's fuller config");

        // Unknown / untabulated languages fall back to the default, never a
        // neighbour's config (what the missing-break bug produced in C).
        assert_eq!(
            g("zz"),
            NumberGrammar::default(),
            "unknown lang → default grammar"
        );
        assert_eq!(g("xyz"), NumberGrammar::default());
    }

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
    fn normalize_voice_tag_us_underscore() {
        assert_eq!(normalize_voice_tag("en_US"), "en-us");
        assert_eq!(normalize_voice_tag(" EN-us "), "en-us");
    }

    #[test]
    fn split_voice_variant_cases() {
        assert_eq!(split_voice_variant("en"), ("en", None));
        assert_eq!(split_voice_variant("en+f3"), ("en", Some("f3")));
        assert_eq!(split_voice_variant("en-us+m3"), ("en-us", Some("m3")));
        assert_eq!(split_voice_variant("en+whisper"), ("en", Some("whisper")));
        // Empty / degenerate variant → None (still usable base).
        assert_eq!(split_voice_variant("en+"), ("en", None));
        // Leading `+` (no base) is left intact rather than yielding an empty base.
        assert_eq!(split_voice_variant("+f3"), ("+f3", None));
    }

    #[test]
    fn translator_new_strips_variant() {
        // `en+f3` must resolve the `en` language data, not a nonexistent
        // `en+f3` dict — the variant is acoustic-only.
        let t = Translator::new_default("en+f3").unwrap();
        assert_eq!(t.options.lang, "en");
    }

    #[test]
    fn lang_options_en_us_matches_en_number_grammar() {
        let o = LangOptions::for_lang("en-US");
        assert_eq!(o.lang, "en-us");
        assert_eq!(o.number_grammar, LangOptions::for_lang("en").number_grammar);
    }

    #[test]
    fn translator_new_default_normalizes_voice_tag() {
        let t = Translator::new_default("en_US").unwrap();
        assert_eq!(t.options.lang, "en-us");
    }

    #[test]
    fn tokenize_hello_world() {
        let tokens = tokenize("hello world");
        assert_eq!(
            tokens,
            vec![
                Token::Word("hello".to_string()),
                Token::Space,
                Token::Word("world".to_string()),
            ]
        );
    }

    #[test]
    fn tokenize_with_punctuation() {
        let tokens = tokenize("hello, world!");
        assert!(tokens
            .iter()
            .any(|t| t == &Token::Word("hello".to_string())));
        assert!(tokens
            .iter()
            .any(|t| t == &Token::Word("world".to_string())));
        assert!(tokens.iter().any(|t| t == &Token::ClauseBoundary(',')));
        assert!(tokens.iter().any(|t| t == &Token::ClauseBoundary('!')));
    }

    #[test]
    fn comma_groups_thousands_for_english() {
        let en = NumberGrammar::for_lang("en");
        // Well-formed grouping → a single number token.  A number of four
        // digits or fewer keeps its separator, which C reads as two words.
        assert_eq!(
            tokenize_opts("1,000", &en),
            vec![Token::Number(NumberToken::Cardinal("1,000".into()))]
        );
        assert_eq!(
            tokenize_opts("1,234,567", &en),
            vec![Token::Number(NumberToken::Cardinal("1234567".into()))]
        );
        // A decimal after a group still parses.
        assert_eq!(
            tokenize_opts("1,000.5", &en),
            vec![Token::Number(NumberToken::Decimal {
                integer: "1000".into(),
                fractional: "5".into()
            })]
        );
        // Malformed groups (not exactly three digits) keep the comma as a boundary.
        assert!(
            tokenize_opts("1,0000", &en)
                .iter()
                .any(|t| t == &Token::ClauseBoundary(',')),
            "4-digit group must not be absorbed"
        );
        // A language without a group separator (the default) is unchanged.
        assert!(
            tokenize_opts("1,000", &NumberGrammar::default())
                .iter()
                .any(|t| t == &Token::ClauseBoundary(',')),
            "no grouping without a configured separator"
        );
    }

    #[test]
    fn leading_decimal_point_is_a_number() {
        let en = NumberGrammar::for_lang("en");
        assert_eq!(
            tokenize_opts(".5", &en),
            vec![Token::Number(NumberToken::Decimal {
                integer: "0".into(),
                fractional: "5".into()
            })]
        );
        assert_eq!(
            tokenize_opts(".25", &en),
            vec![Token::Number(NumberToken::Decimal {
                integer: "0".into(),
                fractional: "25".into()
            })]
        );
        // A dot NOT followed by a digit is still a clause boundary.
        assert!(
            tokenize_opts("5.", &en)
                .iter()
                .any(|t| matches!(t, Token::ClauseBoundary('.'))),
            "trailing dot must stay a sentence boundary"
        );
        // Works for the language's own decimal separator (German comma).
        let de = NumberGrammar::for_lang("de");
        assert_eq!(
            tokenize_opts(",5", &de),
            vec![Token::Number(NumberToken::Decimal {
                integer: "0".into(),
                fractional: "5".into()
            })]
        );
    }

    #[test]
    fn native_script_digits_become_numbers() {
        // Arabic-Indic, Devanagari, Persian, Thai, Bengali → ASCII digits, so
        // they tokenize as numbers instead of being dropped.
        assert_eq!(native_digit_to_ascii('٥'), Some('5')); // Arabic-Indic
        assert_eq!(native_digit_to_ascii('५'), Some('5')); // Devanagari
        assert_eq!(native_digit_to_ascii('۵'), Some('5')); // Persian
        assert_eq!(native_digit_to_ascii('๕'), Some('5')); // Thai
        assert_eq!(native_digit_to_ascii('a'), None);
        // Consecutive native digits form a single multi-digit number.
        assert_eq!(normalize_number_symbols("١٢٣"), "123");
        let g = NumberGrammar::default();
        assert_eq!(
            tokenize_opts("٥", &g),
            vec![Token::Number(NumberToken::Cardinal("5".into()))]
        );
        assert_eq!(
            tokenize_opts("١٢٣", &g),
            vec![Token::Number(NumberToken::Cardinal("123".into()))]
        );

        // Arabic native separators normalise to their Latin equivalents.
        assert_eq!(normalize_number_symbols("١٫٥"), "1.5"); // decimal ٫
        assert_eq!(normalize_number_symbols("١٬٠٠٠"), "1,000"); // thousands ٬
        assert_eq!(normalize_number_symbols("٥٪"), "5%"); // percent ٪

        // Full-width, enclosed and other compatibility forms are left alone,
        // as C reads them.
        assert_eq!(normalize_number_symbols("Ｈｅｌｌｏ"), "Ｈｅｌｌｏ");
        assert_eq!(normalize_number_symbols("①"), "①");
    }

    #[test]
    fn unicode_number_symbols_are_normalized() {
        use std::borrow::Cow;
        // Subscripts stay digits (H₂O).
        assert_eq!(normalize_number_symbols("H₂O"), "H 2O");
        // Vulgar fractions are symbols of their own, as in C.
        assert_eq!(normalize_number_symbols("½"), "½");
        assert_eq!(normalize_number_symbols("3½"), "3½");
        // Plain text is returned borrowed (no allocation).
        assert!(matches!(
            normalize_number_symbols("hello"),
            Cow::Borrowed("hello")
        ));

        // Stylized letters are left alone too.
        assert_eq!(normalize_number_symbols("𝐇𝐞𝐥𝐥𝐨"), "𝐇𝐞𝐥𝐥𝐨");

        // End-to-end: `x²` reads "x squared"; a subscript `H₂` still yields a
        // number token (not vanishing).
        let en = NumberGrammar::for_lang("en");
        assert!(
            tokenize_opts("x²", &en)
                .iter()
                .any(|t| t == &Token::Word("squared".into())),
            "superscript ² should read 'squared'"
        );
        assert!(
            tokenize_opts("H₂", &en)
                .iter()
                .any(|t| t == &Token::Number(NumberToken::Cardinal("2".into()))),
            "subscript should still yield a number token"
        );
    }

    #[test]
    fn non_ordinal_suffix_keeps_the_number() {
        let en = NumberGrammar::for_lang("en");
        // A plural "s" (decades) is not an ordinal → the number is still a number,
        // and the "s" is a separate word (previously the number was swallowed).
        assert_eq!(
            tokenize_opts("1990s", &en),
            vec![
                Token::Number(NumberToken::Cardinal("1990".into())),
                Token::Word("s".into()),
            ]
        );
        // A unit suffix likewise stays a separate word.
        assert_eq!(
            tokenize_opts("5km", &en),
            vec![
                Token::Number(NumberToken::Cardinal("5".into())),
                Token::Word("km".into()),
            ]
        );
        // A suffix the dictionary has a `_#suffix` entry for makes an ordinal
        // (the translator finds them in the text), whether or not it suits the
        // number: C reads "2st" as `s'Ekst`.
        let mut en = en;
        en.ordinal_suffixes = ["st", "nd", "rd", "th"].map(String::from).to_vec();
        for (input, num) in [("3rd", "3"), ("21st", "21"), ("2st", "2"), ("11th", "11")] {
            assert!(
                matches!(
                    tokenize_opts(input, &en).as_slice(),
                    [Token::Number(NumberToken::Ordinal(o))] if o.digits == num
                ),
                "{input} should be an ordinal",
            );
        }
    }

    #[test]
    fn european_decimal_and_grouping_are_swapped() {
        let de = NumberGrammar::for_lang("de");
        assert_eq!((de.decimal_separator, de.group_separator), (',', Some('.')));
        // German uses `.` to group and `,` for the decimal.
        assert_eq!(
            tokenize_opts("1.000", &de),
            vec![Token::Number(NumberToken::Cardinal("1.000".into()))]
        );
        assert_eq!(
            tokenize_opts("3,14", &de),
            vec![Token::Number(NumberToken::Decimal {
                integer: "3".into(),
                fractional: "14".into()
            })]
        );
        assert_eq!(
            tokenize_opts("1.000,5", &de),
            vec![Token::Number(NumberToken::Decimal {
                integer: "1000".into(),
                fractional: "5".into()
            })]
        );

        // English is the mirror image and stays as before.
        let en = NumberGrammar::for_lang("en");
        assert_eq!((en.decimal_separator, en.group_separator), ('.', Some(',')));
        assert_eq!(
            tokenize_opts("3.14", &en),
            vec![Token::Number(NumberToken::Decimal {
                integer: "3".into(),
                fractional: "14".into()
            })]
        );
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
        assert!((ClauseFlags::PAUSE_MASK & ClauseFlags::INTONATION_MASK).is_empty());
        assert!((ClauseFlags::INTONATION_MASK & ClauseFlags::TYPE_MASK).is_empty());
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
        if !dir.join("phontab").exists() {
            return None;
        }
        let mut phdata = PhonemeData::load(dir).ok()?;
        phdata.select_table_by_name("en").ok()?;
        Some(phdata)
    }

    #[test]
    fn phonemes_to_ipa_the() {
        // "the" dict phonemes: [87, 115] = [D, @2] → ðə
        let phdata = match make_phdata() {
            Some(d) => d,
            None => return,
        };
        let (ipa, _) = phonemes_to_ipa(&[87, 115], &phdata, PendingStress::None, false);
        assert_eq!(ipa, "ðə");
    }

    #[test]
    fn phonemes_to_ipa_be() {
        // "be" dict phonemes: [72, 137] = [b, i:] → biː
        let phdata = match make_phdata() {
            Some(d) => d,
            None => return,
        };
        let (ipa, _) = phonemes_to_ipa(&[72, 137], &phdata, PendingStress::None, false);
        assert_eq!(ipa, "biː");
    }

    #[test]
    fn phonemes_to_ipa_with_stress() {
        // "not" dict: [4, 50, 129, 47] = [STRESS_2, n, 0, t]
        // secondary stress before the vowel (ɒ), consonant onset comes before stress mark
        // → "nˌɒt" (stress mark immediately before the stressed vowel)
        let phdata = match make_phdata() {
            Some(d) => d,
            None => return,
        };
        let (ipa, _) = phonemes_to_ipa(&[4, 50, 129, 47], &phdata, PendingStress::None, false);
        assert_eq!(ipa, "nˌɒt");
    }

    #[test]
    fn text_to_ipa_be() {
        let t = Translator::new_default("en").unwrap();
        if !Path::new("/usr/share/espeak-ng-data/en_dict").exists() {
            return;
        }
        let ipa = t.text_to_ipa("be").unwrap();
        // "be" in isolation gets primary stress via clause-level promotion
        assert_eq!(ipa, "bˈiː");
    }

    #[test]
    fn text_to_ipa_en_us_shares_en_dict_but_not_its_phoneme_table() {
        let data_dir = Path::new("espeak-ng-data");
        let data_dir = if data_dir.join("en_dict").exists() {
            data_dir
        } else if Path::new("/usr/share/espeak-ng-data/en_dict").exists() {
            Path::new("/usr/share/espeak-ng-data")
        } else {
            return;
        };
        let t = Translator::new("en_US", Some(data_dir)).unwrap();
        assert_eq!(t.options.lang, "en-us");
        let ipa = t.text_to_ipa("hello").unwrap();
        assert!(!ipa.is_empty());
        let t_en = Translator::new("en", Some(data_dir)).unwrap();
        // en-US has no en-us_dict, so it shares en_dict — but it has its own
        // *phoneme table*, and the GOAT vowel is where the two part company.
        // (This used to assert the two were equal, which was the bug: the
        // en-GB IPA overrides outranked the en-us table's own phondata.)
        assert_eq!(ipa, "həlˈoʊ");
        assert_eq!(t_en.text_to_ipa("hello").unwrap(), "həlˈəʊ");
    }

    #[test]
    fn word_to_ipa_unseen_retranslates_stem_after_un_prefix() {
        let data_dir = Path::new("espeak-ng-data");
        let data_dir = if data_dir.join("en_dict").exists() {
            data_dir
        } else if Path::new("/usr/share/espeak-ng-data/en_dict").exists() {
            Path::new("/usr/share/espeak-ng-data")
        } else {
            return;
        };
        let t = Translator::new("en-us", Some(data_dir)).unwrap();
        let ipa = t.text_to_ipa("unseen").unwrap();
        assert!(
            ipa.contains("sˈiːn") || ipa.contains("siːn"),
            "expected 'seen' syllable in output: {ipa:?}"
        );
    }

    #[test]
    fn word_to_ipa_seen_or_unseen_us_has_no_control_chars() {
        let data_dir = Path::new("espeak-ng-data");
        let data_dir = if data_dir.join("en_dict").exists() {
            data_dir
        } else if Path::new("/usr/share/espeak-ng-data/en_dict").exists() {
            Path::new("/usr/share/espeak-ng-data")
        } else {
            return;
        };
        let t = Translator::new("en-us", Some(data_dir)).unwrap();
        let ipa = t.text_to_ipa("seen or unseen,").unwrap();
        assert!(
            !ipa.chars().any(|c| c.is_control()),
            "IPA must not contain C0 controls: {ipa:?}"
        );
    }

    /// GitHub #4: `translate_to_codes` must apply the same clause promotions as `text_to_ipa`
    /// (WH-word secondary stress; linking **r** after **or**).
    #[test]
    fn github_issue4_translate_codes_matches_ipa_pipeline() {
        let data_dir = Path::new("espeak-ng-data");
        let data_dir = if data_dir.join("en_dict").exists() {
            data_dir
        } else if Path::new("/usr/share/espeak-ng-data/en_dict").exists() {
            Path::new("/usr/share/espeak-ng-data")
        } else {
            return;
        };
        let mut phdata = PhonemeData::load(data_dir).unwrap();
        phdata.select_table_by_name("en-us").unwrap();
        let r_code = phdata.lookup_phoneme("r");
        assert!(r_code > 0, "en-us phontab should define r");

        let t = Translator::new("en-us", Some(data_dir)).unwrap();
        let wh = t.translate_to_codes("When choices cease").unwrap();
        let flat: Vec<u8> = wh
            .iter()
            .filter(|c| !c.is_boundary)
            .map(|c| c.code)
            .collect();
        assert!(
            flat.len() >= 3 && flat[0] == 58 && flat[1] == 4,
            "expected w then secondary stress (4) for clause-initial when: {flat:?}"
        );

        let or_line = t.translate_to_codes("seen or unseen").unwrap();
        let flat: Vec<u8> = or_line
            .iter()
            .filter(|c| !c.is_boundary)
            .map(|c| c.code)
            .collect();
        assert!(
            flat.windows(2).any(|w| w[0] == 140 && w[1] == r_code),
            "expected linking /r/ phoneme after dict tail of or (140): {flat:?}"
        );
    }

    #[test]
    fn text_to_ipa_he() {
        let t = Translator::new_default("en").unwrap();
        if !Path::new("/usr/share/espeak-ng-data/en_dict").exists() {
            return;
        }
        let ipa = t.text_to_ipa("he").unwrap();
        // "he" in isolation gets primary stress via clause-level promotion
        assert_eq!(ipa, "hˈiː");
    }

    #[test]
    fn text_to_ipa_do() {
        let t = Translator::new_default("en").unwrap();
        if !Path::new("/usr/share/espeak-ng-data/en_dict").exists() {
            return;
        }
        let ipa = t.text_to_ipa("do").unwrap();
        assert_eq!(ipa, "dˈuː");
    }

    #[test]
    fn text_to_ipa_the() {
        let t = Translator::new_default("en").unwrap();
        if !Path::new("/usr/share/espeak-ng-data/en_dict").exists() {
            return;
        }
        let ipa = t.text_to_ipa("the").unwrap();
        // "the" in isolation gets primary stress via clause-level promotion (matches C oracle)
        assert_eq!(ipa, "ðˈə");
    }

    // ── CJK tokenize ────────────────────────────────────────────────────

    #[test]
    fn tokenize_chinese_chars_are_individual_words() {
        // Each ideograph is its own word *and* is separated like a
        // whitespace-delimited one — upstream breaks words between ideographs,
        // which is what puts the `_|` gap between syllables in `-x`.
        let tokens = tokenize("你好世界");
        assert_eq!(
            tokens,
            vec![
                Token::Word("你".to_string()),
                Token::Space,
                Token::Word("好".to_string()),
                Token::Space,
                Token::Word("世".to_string()),
                Token::Space,
                Token::Word("界".to_string()),
            ]
        );
    }

    #[test]
    fn tokenize_cjk_with_spaces() {
        let tokens = tokenize("你好 世界");
        assert_eq!(
            tokens,
            vec![
                Token::Word("你".to_string()),
                Token::Space,
                Token::Word("好".to_string()),
                Token::Space,
                Token::Word("世".to_string()),
                Token::Space,
                Token::Word("界".to_string()),
            ]
        );
    }

    #[test]
    fn tokenize_mixed_cjk_and_latin() {
        let tokens = tokenize("Hello你好World世界");
        assert_eq!(
            tokens,
            vec![
                Token::Word("Hello".to_string()),
                Token::Word("你".to_string()),
                Token::Space,
                Token::Word("好".to_string()),
                Token::Word("World".to_string()),
                Token::Word("世".to_string()),
                Token::Space,
                Token::Word("界".to_string()),
            ]
        );
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

    // ── ordinal numbers ───────────────────────────────────────────────────

    fn run_ipa_table(lang: &str, dict_name: &str, cases: &[(&str, &str)]) {
        let dict_path = format!("espeak-ng-data/{dict_name}");
        if !Path::new(&dict_path).exists() {
            return;
        }
        let t = Translator::new_default(lang).unwrap();
        for &(input, expected) in cases {
            let ipa = t.text_to_ipa(input).unwrap();
            assert_eq!(ipa, expected, "lang={lang} input={input:?}");
        }
    }

    #[test]
    fn text_to_ipa_english_rule_regressions() {
        run_ipa_table(
            "en",
            "en_dict",
            &[
                ("sky", "skˈaɪ"),
                ("caused", "kˈɔːzd"),
                ("reflection", "ɹɪflˈɛkʃən"),
                ("droplets", "dɹˈɒplɪts"),
                ("appearing", "ɐpˈiəɹɪŋ"),
                ("meteorological", "mˌiːtɪˌɔːɹəlˈɒdʒɪkəl"),
            ],
        );
    }

    #[test]
    fn text_to_ipa_english_sentence_weak_forms() {
        let t = Translator::new_default("en").unwrap();
        if !Path::new("/usr/share/espeak-ng-data/en_dict").exists() {
            return;
        }
        let ipa = t.text_to_ipa("A rainbow is a meteorological phenomenon that is caused by reflection, refraction and dispersion of light in water droplets resulting in a spectrum of light appearing in the sky.").unwrap();
        assert_eq!(
            ipa,
            "ɐ ɹˈeɪnbəʊ ɪz ɐ mˌiːtɪˌɔːɹəlˈɒdʒɪkəl fɪnˈɒmɪnən ðat ɪz kˈɔːzd baɪ ɹɪflˈɛkʃən\nɹɪfɹˈakʃən and dɪspˈɜːʃən ɒv lˈaɪt ɪn wˈɔːtə dɹˈɒplɪts ɹɪzˈʌltɪŋ ɪn ɐ spˈɛktɹəm ɒv lˈaɪt ɐpˈiəɹɪŋ ɪnðə skˈaɪ"
        );
    }

    #[test]
    fn ordinals_english() {
        run_ipa_table(
            "en",
            "en_dict",
            &[
                ("1st", "fˈɜːst"),
                ("2nd", "sˈɛkənd"),
                ("3rd", "θˈɜːd"),
                ("4th", "fˈɔːθ"),
                ("21st", "twˈɛnti fˈɜːst"),
                ("100th", "wˈɒnhˈʌndɹɪd θ"),
            ],
        );
    }

    #[test]
    fn ordinals_english_large_scales() {
        run_ipa_table(
            "en",
            "en_dict",
            &[
                ("1000th", "wˈɒn θˈaʊzənd θ"),
                ("1001st", "wˈɒn θˈaʊzənd ən fˈɜːst"),
                ("1000000th", "wˈɒn mˈɪliən θ"),
            ],
        );
    }

    #[test]
    fn ordinals_spanish() {
        run_ipa_table(
            "es",
            "es_dict",
            &[
                ("1º", "pɾimˈɛɾo"),
                ("21º", "βixˌɛsimˌopɾimˈɛɾo"),
                ("100º", "θɛntˈɛsim o"),
            ],
        );
    }

    #[test]
    fn ordinals_spanish_large_scale_do_not_use_hundred_root() {
        let dict_path = "espeak-ng-data/es_dict";
        if !Path::new(dict_path).exists() {
            return;
        }
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
        run_ipa_table("nl", "nl_dict", &[("1e", " ˈɪːrstə"), ("3e", "dˈɛrdə")]);
    }

    #[test]
    fn ordinals_german_dot() {
        // NUM_ORDINAL_DOT mechanism.  A compound ordinal takes the suffix on the
        // whole cardinal ("einundzwanzigste"), not tens-ord + units-ord (which
        // gave the wrong two-word "zwanzig erste").
        run_ipa_table(
            "de",
            "de_dict",
            &[
                ("1.", "ˈaɪns"),
                ("3.", "dɾˈaɪ"),
                ("20.", "tsvˈantsɪç"),
                // "einundzwanzigste" — the unit "1" apocopates to "ein" (not "eins").
                ("21.", "ˈaɪn ʊnttsvˈantsɪç"),
            ],
        );
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
             "ˈunmiʝˈon dosθjˈentos tɾˌeɪntaikwˈatɾo mˈil kinjˈɛntos sɛsˌɛntaisjˈete",
             "ˈunmiʝˈon dosθjˈentos tɾˌeɪntaikwˈatɾo mˈil kinjˈɛntos sɛsˌɛntaisjˈete"),
            ("fr", "fr_dict",
             "œ̃ miljˈɔ̃ døsɑ̃ tʁɑ̃tkatʁ mˈil sɛ̃ksɑ̃ swasɑ̃tsˈɛt",
             "œ̃ miljˈɔ̃ døsɑ̃ tʁɑ̃tkatʁ mˈil sɛ̃ksɑ̃ swasɑ̃tsˈɛt"),
            ("de", "de_dict",
             "ˈaɪnə mɪljˈoːn tsvˈaɪhˈʊndɜt fˈiːɾ ʊntdɾˈaɪsɪç tˈaʊzənt fˈʏnfhˈʊndɜt zˈiːbən ʊntzˈɛçtsɪç",
             "ˈaɪnə mɪljˈoːn tsvˈaɪhˈʊndɜt fˈiːɾ ʊntdɾˈaɪsɪç tˈaʊzənt fˈynfhˈʊndɜt zˈiːbən ʊntzˈɛçtsɪç"),
            // (C starts with a space: the number begins with a pause word.)
            ("nl", "nl_dict",
             " ˈeːn mˌiljun tʋˈeːhˌɔndərt vˌirɛndˌɛrtəx dˌœyzɛnt vˈɛɪfhˌɔndərt zˌeːvənɛnzˌɛstəx",
             "ˈeːn mˌiljun tʋˈeːhˌɔndərt vˌirɛndˌɛrtəx dˌœyzɛnt vˈɛɪfhˌɔndərt zˌeːvənɛnzˌɛstəx"),
        ];
        for &(lang, dict, expected, _oracle) in cases {
            let dict_path = format!("espeak-ng-data/{dict}");
            if !Path::new(&dict_path).exists() {
                continue;
            }
            let t = Translator::new_default(lang).unwrap();
            let ipa = t.text_to_ipa("1234567").unwrap();
            assert_eq!(ipa, expected, "lang={lang} input=\"1234567\"");
        }
    }

    #[test]
    fn cardinals_english_billion_scale() {
        let dict_path = "espeak-ng-data/en_dict";
        if !Path::new(dict_path).exists() {
            return;
        }
        let dict = Dictionary::load("en", Path::new("espeak-ng-data")).unwrap();
        let grammar = LangOptions::for_lang("en").number_grammar;
        let pronunciation = cardinal_pronunciation("1000000000", &dict, &grammar).unwrap();
        let billion_lookup = lookup_num_phonemes(&dict, "_0M3");
        let billion = trim_lookup(&billion_lookup);
        assert!(!billion.is_empty(), "en_dict is missing _0M3");
        let trimmed = &pronunciation.bytes[..pronunciation.trimmed_len()];
        assert!(
            trimmed
                .windows(billion.len())
                .any(|window| window == billion),
            "1000000000 should include the billion scale phonemes",
        );
    }

    #[test]
    fn cardinals_french() {
        let dict_path = "espeak-ng-data/fr_dict";
        if !Path::new(dict_path).exists() {
            return;
        }
        let t = Translator::new_default("fr").unwrap();
        for input in ["1", "2", "3", "4", "20", "80", "87", "100", "101"] {
            let ipa = t.text_to_ipa(input).unwrap();
            assert!(!ipa.is_empty(), "fr {input} produced empty IPA");
            assert!(
                !ipa.chars().any(|c| c.is_ascii_digit()),
                "fr {input} has raw digits in IPA: {ipa}"
            );
        }
    }

    #[test]
    fn french_cardinal_text_vigesimal() {
        // The vigesimal speller used to build ordinals (data-free).
        let cases = [
            (0, "zéro"),
            (7, "sept"),
            (16, "seize"),
            (17, "dix-sept"),
            (20, "vingt"),
            (21, "vingt et un"),
            (22, "vingt-deux"),
            (31, "trente et un"),
            (60, "soixante"),
            (61, "soixante et un"),
            (70, "soixante-dix"),
            (71, "soixante et onze"),
            (77, "soixante-dix-sept"),
            (80, "quatre-vingts"),
            (81, "quatre-vingt-un"),
            (90, "quatre-vingt-dix"),
            (91, "quatre-vingt-onze"),
            (99, "quatre-vingt-dix-neuf"),
            (100, "cent"),
            (101, "cent un"),
            (200, "deux cents"),
            (234, "deux cent trente-quatre"),
            (999, "neuf cent quatre-vingt-dix-neuf"),
        ];
        for (n, want) in cases {
            assert_eq!(fr_cardinal_text(n), want, "fr_cardinal_text({n})");
        }
    }

    #[test]
    fn french_ordinal_word_generation() {
        // digits + written suffix → ordinal word (data-free).
        let cases = [
            ("1", "er", "premier"),
            ("1", "re", "première"),
            ("1", "ère", "première"),
            ("2", "e", "deuxième"),
            ("2", "nd", "second"),
            ("2", "nde", "seconde"),
            ("3", "e", "troisième"),
            ("4", "e", "quatrième"),
            ("5", "e", "cinquième"),
            ("9", "e", "neuvième"),
            ("11", "e", "onzième"),
            ("17", "e", "dix-septième"),
            ("20", "e", "vingtième"),
            ("21", "e", "vingt et unième"),
            ("22", "e", "vingt-deuxième"),
            ("70", "e", "soixante-dixième"),
            ("71", "e", "soixante et onzième"),
            ("80", "e", "quatre-vingtième"),
            ("81", "e", "quatre-vingt-unième"),
            ("90", "e", "quatre-vingt-dixième"),
            ("99", "e", "quatre-vingt-dix-neuvième"),
            ("100", "e", "centième"),
            ("200", "e", "deux centième"),
            ("234", "e", "deux cent trente-quatrième"),
            ("1000", "e", "millième"),
        ];
        for (d, suf, want) in cases {
            assert_eq!(
                french_ordinal_word(d, suf).as_deref(),
                Some(want),
                "{d}{suf}"
            );
        }
        // Out of range (no cheap ordinal word) → None (caller reads cardinal).
        assert_eq!(french_ordinal_word("0", "e"), None);
        assert_eq!(french_ordinal_word("5000", "e"), None);
    }

    #[test]
    fn portuguese_cardinal_generation() {
        // The pt dict lacks number keys, so cardinals are generated as text and
        // pronounced by the rules (data-free check of the generator).
        let cases = [
            ("0", "zero"),
            ("1", "um"),
            ("3", "três"),
            ("5", "cinco"),
            ("15", "quinze"),
            ("16", "dezasseis"),
            ("21", "vinte e um"),
            ("100", "cem"),
            ("101", "cento e um"),
            ("123", "cento e vinte e três"),
            ("200", "duzentos"),
            ("234", "duzentos e trinta e quatro"),
            ("999", "novecentos e noventa e nove"),
            ("1000", "mil"),
            ("1001", "mil e um"),
            ("1100", "mil e cem"),
            ("1120", "mil cento e vinte"), // no "e" (120 not <100/round)
            ("1500", "mil e quinhentos"),  // "e" (round hundred)
            ("2000", "dois mil"),
            ("2020", "dois mil e vinte"),
            ("2234", "dois mil duzentos e trinta e quatro"),
            ("100000", "cem mil"),
            ("1000000", "um milhão"),
            ("2000000", "dois milhões"),
            ("1500000", "um milhão e quinhentos mil"),
            ("2000001", "dois milhões e um"),
        ];
        for (digits, want) in cases {
            assert_eq!(
                portuguese_cardinal_word(digits).as_deref(),
                Some(want),
                "pt {digits}"
            );
        }
        // Above the covered range → None (caller keeps the existing path).
        assert_eq!(portuguese_cardinal_word("1000000000"), None);
    }

    #[test]
    fn comma_decimal_languages_are_configured() {
        // These use a comma decimal (period/space thousands); without it, "3,14"
        // read as "three [pause] fourteen".
        for lang in [
            "sv", "pl", "cs", "ro", "el", "lv", "is", "ca", "eu", "sq", "bs", "id", "vi", "az",
            "hy",
        ] {
            let g = NumberGrammar::for_lang(lang);
            assert_eq!(g.decimal_separator, ',', "{lang} decimal");
            assert_eq!(g.group_separator, Some('.'), "{lang} group");
            assert!(g.space_group, "{lang} should allow space grouping too");
        }
        // Period-decimal languages are unchanged — including af, ka and kk, which
        // read "3,14" as two numbers (verified against upstream).
        for lang in ["af", "ka", "kk"] {
            assert_eq!(
                NumberGrammar::for_lang(lang).decimal_separator,
                '.',
                "{lang} decimal"
            );
        }
        assert_eq!(NumberGrammar::for_lang("en").decimal_separator, '.');
        assert_eq!(NumberGrammar::for_lang("en").group_separator, Some(','));
    }
}
