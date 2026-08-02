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
pub fn resolve_dict_stem(data_dir: &Path, voice_tag: &str) -> Option<String> {
    let parts: Vec<&str> = voice_tag
        .split('-')
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    for len in (1..=parts.len()).rev() {
        let stem = parts[..len].join("-");
        if data_dir.join(format!("{stem}_dict")).exists() {
            return Some(stem);
        }
    }
    // No `<tag>_dict`: a voice may borrow another language's dictionary via a
    // `dictionary <name>` directive (Norwegian Bokmål `nb` → `no`).
    let (dictionary, _) = crate::voices::voice_data_overrides(data_dir, voice_tag);
    if let Some(dict) = dictionary {
        if data_dir.join(format!("{dict}_dict")).exists() {
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
pub fn select_phoneme_table(
    phdata: &mut PhonemeData,
    data_dir: &Path,
    lang: &str,
) -> Result<()> {
    let resolved = resolve_phoneme_table(data_dir, lang);
    let base = primary_bcp47_subtag(lang);
    for candidate in [lang, resolved.as_str(), base] {
        if phdata.select_table_by_name(candidate).is_ok() {
            return Ok(());
        }
    }
    // Re-run the base selection to surface a clear error if nothing matched.
    phdata.select_table_by_name(base).map(|_| ())
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
    let query = crate::voices::VoiceQuery { name: Some(selector.to_string()), ..Default::default() };
    crate::voices::find_voice(&voices, &query)
        .and_then(|v| resolve_dict_stem(data_dir, &v.language))
}

use crate::error::{Error, Result};
use crate::phoneme::load::PhonemeData;
use crate::dictionary::file::Dictionary;
use crate::dictionary::lookup::{lookup, LookupCtx};
use crate::dictionary::rules::is_letter_wc;
use crate::dictionary::rules::translate_rules_phdata;
use crate::dictionary::{
    FLAG_PREFIX_REMOVED, FLAG_SUFX, FLAG_SUFX_E_ADDED, FLAG_SUFFIX_REMOVED, FLAG_SUFFIX_VOWEL,
    FLAG_SUFX_S, LETTERGP_B, LETTERGP_VOWEL2, SUFX_A, SUFX_E, SUFX_I, SUFX_P,
};
use crate::dictionary::stress::{set_word_stress, promote_strend_stress, change_word_stress,
                               apply_word_final_devoicing, apply_alt_stress_upgrade, StressOpts};

use ipa_table::{
    EN_IPA_OVERRIDES,
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
    /// BCP-47 language tag, e.g. `"en"`, `"en-us"`, `"fr"`, `"de"`
    pub lang: String,
    /// Words per minute (default 175)
    pub rate: u32,
    /// Base pitch (0–100, default 50)
    pub pitch: u32,
    /// Word gap in units of 10ms
    pub word_gap: i32,
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
    /// Split a word at a lowercase→uppercase boundary (`CamelCase` → "Camel
    /// Case", `iPhone` → "i Phone"), mirroring espeak's `TranslateClause`
    /// behaviour (a space is inserted before the uppercase letter).  Universal
    /// in espeak; only Irish (`ga`), whose lenition prefixes legitimately mix
    /// case, disables it — so this is `true` by default and `false` for `ga`.
    pub caps_word_split: bool,
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
                grammar.combining_one = Some("ein".to_string());
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
                // "hundert"/"tausend", not "eins hundert"/"eins tausend".
                grammar.hundreds.omit_one_prefix = true;
                grammar.thousands.omit_one_prefix = true;
            }
            // Russian: "сто"/"тысяча", not "один сто"/"один тысяча".
            "ru" | "bg" | "uk" | "be" => {
                grammar.hundreds.omit_one_prefix = true;
                grammar.thousands.omit_one_prefix = true;
                grammar.decimal_separator = ',';
                if lang == "ru" {
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
            // Portuguese: the dict lacks number keys, so cardinals are generated
            // as text (`portuguese_cardinal_word`) and pronounced by the rules.
            // Comma decimal / period thousands like the rest of the Romance set.
            "pt" => {
                grammar.portuguese_cardinals = true;
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
            }
            // Continental-European comma-decimal languages.
            // Units-first cardinals: Danish/Faroese "enogtyve", Slovenian
            // "enaindvajset" (units + "og"/"in" + tens), like German — and their
            // compound ordinals suffix the whole cardinal ("enogtyvende").
            "da" | "fo" | "sl" => {
                grammar.ordinals.dot_marks_ordinal = true;
                grammar.tens = TensGrammar::UnitsThenConjunction;
                grammar.ordinals.compound_cardinal_suffix = true;
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
            }
            // Tens-first, dot-ordinal, comma-decimal (Finnish, Estonian,
            // Lithuanian, Norwegian "tjueen", Greenlandic).
            "et" | "fi" | "kl" | "nb" | "no" => {
                grammar.ordinals.dot_marks_ordinal = true;
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
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
            "kk" => {
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
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
            "af" => {
                grammar.decimal_separator = ',';
                grammar.group_separator = Some('.');
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
            "sv" | "el" | "lv" | "ca" | "eu"
            | "bs" | "id" | "vi" | "ka" => {
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
            group_separator: None,
            decimal_separator: '.',
            vigesimal_70_90: false,
            space_group: false,
            portuguese_cardinals: false,
            caps_word_split: true,
            caps_mark_stress: false,
            dutch_ij: false,
            combining_one: None,
            elide_tens_vowel: false,
            thousands_variant: SlavicThousands::None,
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
            capitals:    0,
            punct:       None,
            number_grammar: NumberGrammar::default(),
            dict_condition: 0,
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
        Self {
            number_grammar: NumberGrammar::for_lang(&lang),
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
    /// Sentence/clause boundary punctuation: `.`, `,`, `!`, `?`, `;`, `:`.
    ClauseBoundary(char),
    /// Any other punctuation character.
    Punctuation(char),
    /// Inline phonemes given directly in `[[ … ]]` (espeak phoneme mnemonics).
    /// These bypass the dictionary and are emitted verbatim.
    InlinePhonemes(String),
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
        "er" | "ers" | "re" | "res" | "ère" | "ères"
            | "e" | "es" | "ème" | "èmes" | "eme" | "emes"
            | "nd" | "nds" | "nde" | "ndes"
    )
}

/// Spell a French cardinal number in the range `0..=999` as text (the vigesimal
/// system: `71` → "soixante et onze", `80` → "quatre-vingts", `234` → "deux cent
/// trente-quatre").  Used only to build ordinals (the phoneme path renders
/// cardinals directly), so it stays under 1000.
fn fr_cardinal_text(n: u32) -> String {
    const U: [&str; 20] = [
        "zéro", "un", "deux", "trois", "quatre", "cinq", "six", "sept", "huit", "neuf",
        "dix", "onze", "douze", "treize", "quatorze", "quinze", "seize",
        "dix-sept", "dix-huit", "dix-neuf",
    ];
    if n < 20 {
        return U[n as usize].to_string();
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
            _ => format!("{base}-{}", U[u as usize]),
        };
    }
    if n < 80 {
        // soixante + (0..=19): "soixante", "soixante et un/onze", "soixante-dix"…
        return match n - 60 {
            0 => "soixante".to_string(),
            1 => "soixante et un".to_string(),
            11 => "soixante et onze".to_string(),
            inner => format!("soixante-{}", U[inner as usize]),
        };
    }
    if n < 100 {
        // quatre-vingt(s) + (0..=19); no "et", plural "s" only on bare 80.
        return match n - 80 {
            0 => "quatre-vingts".to_string(),
            inner => format!("quatre-vingt-{}", U[inner as usize]),
        };
    }
    // 100..=999.
    let (h, rest) = (n / 100, n % 100);
    let mut head = if h == 1 { "cent".to_string() } else { format!("{} cent", U[h as usize]) };
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
        "primo", "secondo", "terzo", "quarto", "quinto",
        "sesto", "settimo", "ottavo", "nono", "decimo",
    ];
    const FEM: [&str; 10] = [
        "prima", "seconda", "terza", "quarta", "quinta",
        "sesta", "settima", "ottava", "nona", "decima",
    ];
    let table = if suffix == "ª" { &FEM } else { &MASC };
    Some(table[(n - 1) as usize].to_string())
}

/// Spell a Portuguese cardinal group `0..=999` as text (European Portuguese).
/// Components are joined with "e" (`234` → "duzentos e trinta e quatro"); `100`
/// alone is "cem", within a larger number it's "cento".  Empty for `0`.
fn pt_group(n: u32) -> String {
    const U: [&str; 20] = [
        "zero", "um", "dois", "três", "quatro", "cinco", "seis", "sete", "oito", "nove",
        "dez", "onze", "doze", "treze", "catorze", "quinze",
        "dezasseis", "dezassete", "dezoito", "dezanove",
    ];
    const T: [&str; 10] = [
        "", "", "vinte", "trinta", "quarenta", "cinquenta",
        "sessenta", "setenta", "oitenta", "noventa",
    ];
    const H: [&str; 10] = [
        "", "cento", "duzentos", "trezentos", "quatrocentos", "quinhentos",
        "seiscentos", "setecentos", "oitocentos", "novecentos",
    ];
    let tens_units = |tu: u32| -> String {
        if tu < 20 {
            U[tu as usize].to_string()
        } else {
            let (t, u) = ((tu / 10) as usize, tu % 10);
            if u == 0 {
                T[t].to_string()
            } else {
                format!("{} e {}", T[t], U[u as usize])
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
        parts.push(H[(n / 100) as usize].to_string());
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
            if millions == 1 { "um milhão".to_string() } else { format!("{} milhões", pt_group(millions)) },
        ));
    }
    if thousands > 0 {
        pieces.push((
            thousands,
            if thousands == 1 { "mil".to_string() } else { format!("{} mil", pt_group(thousands)) },
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
        NumberToken::Decimal { integer, fractional } => {
            let mut s = portuguese_cardinal_word(integer)?;
            s.push_str(" vírgula");
            for d in fractional.chars() {
                s.push(' ');
                s.push_str(&portuguese_cardinal_word(&d.to_string())?);
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
fn normalize_number_symbols(text: &str) -> std::borrow::Cow<'_, str> {
    fn mapping(c: char) -> Option<&'static str> {
        Some(match c {
            // Superscript digits (⁰¹²³⁴⁵⁶⁷⁸⁹).
            '\u{2070}' | '⁴' | '⁵' | '⁶' | '⁷' | '⁸' | '⁹' => match c {
                '\u{2070}' => " 0", '⁴' => " 4", '⁵' => " 5", '⁶' => " 6",
                '⁷' => " 7", '⁸' => " 8", _ => " 9",
            },
            // Superscript ²/³ are powers, read "squared"/"cubed" (not "two").
            '¹' => " 1", '²' => " squared ", '³' => " cubed ",
            // Subscript digits (₀₁₂₃₄₅₆₇₈₉ = U+2080..2089).
            '₀' => " 0", '₁' => " 1", '₂' => " 2", '₃' => " 3", '₄' => " 4",
            '₅' => " 5", '₆' => " 6", '₇' => " 7", '₈' => " 8", '₉' => " 9",
            // Vulgar fractions.
            '½' => " 1/2 ", '⅓' => " 1/3 ", '⅔' => " 2/3 ", '¼' => " 1/4 ",
            '¾' => " 3/4 ", '⅕' => " 1/5 ", '⅖' => " 2/5 ", '⅗' => " 3/5 ",
            '⅘' => " 4/5 ", '⅙' => " 1/6 ", '⅚' => " 5/6 ", '⅐' => " 1/7 ",
            '⅛' => " 1/8 ", '⅜' => " 3/8 ", '⅝' => " 5/8 ", '⅞' => " 7/8 ",
            '⅑' => " 1/9 ", '⅒' => " 1/10 ",
            // Arabic/Persian native separators → their Latin equivalents (kept
            // adjacent to the digits so `١٫٥`→`1.5`, `١٬٠٠٠`→`1,000`, `٥٪`→`5%`).
            '\u{066B}' => ".", // ARABIC DECIMAL SEPARATOR ٫
            '\u{066C}' => ",", // ARABIC THOUSANDS SEPARATOR ٬
            '\u{066A}' => "%", // ARABIC PERCENT SIGN ٪
            _ => return None,
        })
    }

    let changed = |c: char| mapping(c).is_some()
        || native_digit_to_ascii(c).is_some()
        || fullwidth_to_ascii(c).is_some()
        || stylized_to_ascii(c).is_some()
        || enclosed_number_value(c).is_some();
    if !text.chars().any(changed) {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        if let Some(s) = mapping(c) {
            out.push_str(s);
        } else if let Some(d) = native_digit_to_ascii(c) {
            // Consecutive native digits stay adjacent so they form one number.
            out.push(d);
        } else if let Some(a) = fullwidth_to_ascii(c) {
            out.push(a);
        } else if let Some(a) = stylized_to_ascii(c) {
            // Mathematical/stylized letters & digits stay adjacent (form one word).
            out.push(a);
        } else if let Some(n) = enclosed_number_value(c) {
            // A standalone enclosed/Roman numeral → its value, spaced so it
            // doesn't merge with an adjacent digit.
            out.push(' ');
            out.push_str(&n.to_string());
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    std::borrow::Cow::Owned(out)
}

/// The numeric value of a circled/parenthesised digit (`①`=1, `⑩`=10, `⑴`=1) or a
/// Roman-numeral codepoint (`Ⅳ`=4, `Ⅻ`=12, `Ⅿ`=1000), which are otherwise dropped.
fn enclosed_number_value(c: char) -> Option<u32> {
    let cp = c as u32;
    Some(match cp {
        0x24EA => 0,                    // ⓪
        0x2460..=0x2473 => cp - 0x245F, // ①..⑳ = 1..20
        0x2474..=0x2487 => cp - 0x2473, // ⑴..⒇ = 1..20 (parenthesised)
        0x2160..=0x216B => cp - 0x215F, // Ⅰ..Ⅻ = 1..12
        0x2170..=0x217B => cp - 0x216F, // ⅰ..ⅻ = 1..12 (lowercase)
        0x216C | 0x217C => 50,          // Ⅼ ⅼ
        0x216D | 0x217D => 100,         // Ⅽ ⅽ
        0x216E | 0x217E => 500,         // Ⅾ ⅾ
        0x216F | 0x217F => 1000,        // Ⅿ ⅿ
        _ => return None,
    })
}

/// Map a stylized Latin letter or digit — the Mathematical Alphanumeric Symbols
/// (bold `𝐇`, italic `𝐻`, script `𝓗`, fraktur `𝕳`, double-struck `𝔻`,
/// sans-serif `𝖳`, monospace `𝙲`, bold digits `𝟏`), the circled letters
/// (`Ⓗ`/`ⓗ`), and the letter-like holes (`ℝ`, `ℋ`, `ℎ`) — to plain ASCII.  These
/// appear in styled social-media text and are otherwise dropped.
fn stylized_to_ascii(c: char) -> Option<char> {
    let cp = c as u32;
    // Mathematical alphanumeric Latin letters: 13 styles × 52 letters, laid out
    // as A–Z then a–z; reserved holes never occur, so a plain offset works.
    if (0x1D400..=0x1D6A3).contains(&cp) {
        let idx = (cp - 0x1D400) % 52;
        return Some(if idx < 26 {
            (b'A' + idx as u8) as char
        } else {
            (b'a' + (idx - 26) as u8) as char
        });
    }
    // Mathematical digits (5 styles × 10) → 0–9.
    if (0x1D7CE..=0x1D7FF).contains(&cp) {
        return char::from_digit((cp - 0x1D7CE) % 10, 10);
    }
    // Circled Latin letters (Ⓐ–Ⓩ, ⓐ–ⓩ).
    if (0x24B6..=0x24CF).contains(&cp) {
        return Some((b'A' + (cp - 0x24B6) as u8) as char);
    }
    if (0x24D0..=0x24E9).contains(&cp) {
        return Some((b'a' + (cp - 0x24D0) as u8) as char);
    }
    // Letters that were unified into the Letterlike Symbols block, leaving holes
    // in the math ranges (script/fraktur/double-struck).
    Some(match c {
        'ℎ' => 'h', 'ℬ' => 'B', 'ℰ' => 'E', 'ℱ' => 'F', 'ℋ' => 'H', 'ℐ' => 'I',
        'ℒ' => 'L', 'ℳ' => 'M', 'ℛ' => 'R', 'ℯ' => 'e', 'ℊ' => 'g', 'ℴ' => 'o',
        'ℭ' => 'C', 'ℌ' => 'H', 'ℑ' => 'I', 'ℜ' => 'R', 'ℨ' => 'Z',
        'ℂ' => 'C', 'ℍ' => 'H', 'ℕ' => 'N', 'ℙ' => 'P', 'ℚ' => 'Q', 'ℝ' => 'R', 'ℤ' => 'Z',
        _ => return None,
    })
}

/// Map a full-width (CJK-compatibility) form of an ASCII character to plain ASCII
/// — `５`→`5`, `Ａ`→`A`, `＄`→`$`, `％`→`%`, the ideographic space → `' '`.  These
/// appear in Japanese/Chinese text and are otherwise dropped.  `None` otherwise.
fn fullwidth_to_ascii(c: char) -> Option<char> {
    match c as u32 {
        // FF01 '！' .. FF5E '～' are ASCII 0x21 '!' .. 0x7E '~' + 0xFEE0.
        cp @ 0xFF01..=0xFF5E => char::from_u32(cp - 0xFEE0),
        0x3000 => Some(' '), // IDEOGRAPHIC SPACE
        _ => None,
    }
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
/// range/reverb/tone/… (C `EmbeddedCommand`'s `"PSARHTIVYMUBF"`).
const EMBED_LETTERS: &str = "PSARHTIVYMUBF";

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
            Some('+') => { chars.next(); 1 }
            Some('-') => { chars.next(); -1 }
            _ => 0,
        };
        let mut digits = String::new();
        while matches!(chars.peek(), Some(d) if d.is_ascii_digit()) {
            digits.push(chars.next().unwrap());
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

/// Remove inline embedded commands from `text` (borrowing when there are none).
fn strip_embedded_commands(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains(CTRL_EMBEDDED) {
        return std::borrow::Cow::Borrowed(text);
    }
    std::borrow::Cow::Owned(parse_embedded_commands(text).0)
}

pub fn tokenize_opts(text: &str, grammar: &NumberGrammar) -> Vec<Token> {
    let text = strip_embedded_commands(text);
    let text = normalize_number_symbols(&text);
    let text = text.as_ref();
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            // Collapse runs of whitespace into a single Space token.
            while chars.peek().map(|c| c.is_whitespace()).unwrap_or(false) {
                chars.next();
            }
            tokens.push(Token::Space);
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
            let integer = if matches!(tokens.last(), Some(Token::Number(_))) { "" } else { "0" };
            let mut fractional = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    fractional.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(Token::Number(NumberToken::Decimal { integer: integer.into(), fractional }));
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
            // not a clause break — "10:30" (a time; the colon is silent, handled
            // by `apply_time_reading`) or "3:2" (a ratio; the colon reads as the
            // ":" symbol in English, silent elsewhere).  A ':' before whitespace/
            // end ("Note: this") stays a clause boundary below.
            if c == ':' && matches!(chars.peek(), Some(n) if n.is_ascii_digit()) {
                tokens.push(Token::Punctuation(':'));
                continue;
            }
            // Clause/sentence boundary punctuation
            // Absorb trailing whitespace after punctuation
            while chars.peek().map(|ch| ch.is_whitespace()).unwrap_or(false) {
                chars.next();
            }
            tokens.push(Token::ClauseBoundary(c));
        } else if c == '-'
            && minus_precedes_number(&chars)
            && matches!(
                tokens.last(),
                None | Some(Token::Space) | Some(Token::ClauseBoundary(_))
            )
        {
            // A unary minus written with an ASCII hyphen directly before a
            // number ("-5") or a currency amount ("-$5") reads as the language's
            // "minus" word.  Gated to a clause start or just-after-a-space — a
            // hyphen following a word or number ("COVID-19", "5-10") is a
            // separator/range, not a sign, and is left to the punctuation path.
            // Emit the U+2212 MINUS SIGN so it reuses the existing per-language
            // symbol reading (`lookup_symbol_name`); the currency symbol and
            // digits tokenise normally on the following iterations.
            tokens.push(Token::Punctuation('−'));
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
                } else if next == grammar.decimal_separator && !has_dot {
                    // Peek ahead to see if followed by a digit (a real decimal).
                    let mut lookahead = chars.clone();
                    lookahead.next(); // skip the decimal separator
                    if lookahead.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
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
            // A Romance abbreviated ordinal may write a dot before the
            // masculine/feminine indicator ("1.º", "2.ª" — the RAE-recommended
            // Spanish form; also pt/it).  Skip that dot so the indicator is
            // still recognised; a bare "3." (no indicator) is left to the
            // cardinal/`dot_marks_ordinal` paths below.
            // Not for `dot_marks_ordinal` languages (German "3." is its own
            // ordinal; keep the dot for that path).
            let mut dot_before_indicator = false;
            if !grammar.ordinals.dot_marks_ordinal && lookahead.peek() == Some(&'.') {
                let mut probe = lookahead.clone();
                probe.next(); // the '.'
                if matches!(probe.peek(), Some('º') | Some('ª')) {
                    lookahead.next(); // consume the '.' in the lookahead only
                    dot_before_indicator = true;
                }
            }
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
                } else if grammar.caps_word_split
                    && prev_char.is_lowercase()
                    && next.is_uppercase()
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
                } else if next.is_alphabetic() || next == '\'' || is_indic_combining(next) {
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
                tokens.push(Token::Space);
            }
        } else if is_emoji_char(c) {
            // Emoji read via the dict's emoji-name entries.  A whole sequence
            // (variation selector, skin-tone modifier, ZWJ-joined parts, or a
            // regional-indicator flag pair) stays one word so it matches the
            // multi-codepoint dictionary entry.
            let mut emoji = String::from(c);
            let is_regional = |ch: char| (0x1F1E6..=0x1F1FF).contains(&(ch as u32));
            if is_regional(c) {
                if matches!(chars.peek(), Some(&n) if is_regional(n)) {
                    emoji.push(chars.next().unwrap());
                }
            } else {
                while let Some(&m) = chars.peek() {
                    if is_emoji_modifier(m) {
                        chars.next();
                        // Drop the emoji-presentation variation selector (U+FE0F) —
                        // the dictionary keys the base emoji without it — but keep
                        // the ZWJ and skin-tone, which change the emoji's identity.
                        if m != '\u{FE0F}' {
                            emoji.push(m);
                        }
                        // A ZWJ binds the following emoji into the same sequence.
                        if m == '\u{200D}' {
                            if matches!(chars.peek(), Some(&n) if is_emoji_char(n)) {
                                emoji.push(chars.next().unwrap());
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
    use crate::phoneme::{PH_PAUSE, PH_VOWEL, PHON_END_WORD};
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
        let body = &phonemes[..phonemes.len() - 1];
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
    matches!(c as u32,
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
    phonemes_to_ipa_full(
        phoneme_bytes, phdata, pending_stress_in, word_sep, use_en_overrides,
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
        // English `--ipa` output matches the C dictionary path: keep phoneme codes
        // from rules without synthesis-stage stress reductions (e.g. /V/ → schwa),
        // or "unseen" renders as ə… instead of ʌ….
        // Other languages (e.g. Spanish, Russian) still need ChangeIf resolution
        // for stressed vowel allophones.
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
                let level = ((b1 == b'2' || b1 == b'3') && b2 == 0 && !is_vowel)
                    .then(|| b1 - b'0');
                if let Some(level) = level {
                    let next_is_vowel = phoneme_bytes[idx + 1..]
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

            let word_final = phoneme_bytes[idx+1..].iter()
                .all(|&c| c == 0 || c <= 8 || c == 15);

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
            let override_ipa = if use_en_overrides {
                EN_IPA_OVERRIDES.iter()
                    .find_map(|&(override_code, ipa)| (override_code == code).then(|| ipa.to_string()))
            } else {
                None
            };
            let ipa = if let Some(override_ipa) = override_ipa {
                override_ipa
            } else if let Some(ipa_str) = phdata.phoneme_ipa_string(ph.program) {
                ipa_str
            } else {
                phoneme_ipa_lang(code, ph.mnemonic, is_vowel, false)
            };
            let mut ipa = ipa;
            if use_en_overrides && is_vowel {
                let b1 = ((ph.mnemonic >> 8) & 0xff) as u8;
                let b2 = ((ph.mnemonic >> 16) & 0xff) as u8;
                let b3 = ((ph.mnemonic >> 24) & 0xff) as u8;
                let has_rhotic_liaison = b1 != 0 && (b2 == b'3' || b3 == b'3');
                if has_rhotic_liaison && !word_final {
                    ipa.push('ɹ');
                }
            }
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
    const ADD_E_ADDITIONS: &[&str] = &["c", "rs", "ir", "ur", "ath", "ns", "u", "spong", "rang", "larg"];

    let chars: Vec<char> = stem.chars().collect();
    if chars.len() < 2 {
        return false;
    }

    let penultimate = chars[chars.len() - 2] as u32;
    let last = chars[chars.len() - 1] as u32;
    if is_letter_wc(&dict.letter_bits, penultimate, dict.letter_bits_offset, LETTERGP_VOWEL2)
        && is_letter_wc(&dict.letter_bits, last, dict.letter_bits_offset, LETTERGP_B)
    {
        return !ADD_E_EXCEPTIONS.iter().any(|suffix| stem.ends_with(suffix));
    }

    ADD_E_ADDITIONS.iter().any(|suffix| stem.ends_with(suffix))
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

fn remove_standard_suffix(word: &str, end_type: u32, dict: &Dictionary) -> Option<(String, u32, u32)> {
    let suffix_len_chars = (end_type & 0x3f) as usize;
    if suffix_len_chars == 0 {
        return None;
    }

    let mut chars: Vec<char> = word.chars().collect();
    if suffix_len_chars > chars.len() {
        return None;
    }

    let suffix_start = chars.len() - suffix_len_chars;
    let ending: String = chars[suffix_start..].iter().collect();
    chars.truncate(suffix_start);

    if (end_type & SUFX_I) != 0 && chars.last() == Some(&'i') {
        *chars.last_mut().unwrap() = 'y';
    }

    let mut stem: String = chars.iter().collect();
    let mut end_flags = (end_type & 0xfff0) | FLAG_SUFX;

    if (end_type & SUFX_E) != 0 && dict.lang == "en" && english_suffix_needs_e(&stem, dict) {
        stem.push('e');
        end_flags |= FLAG_SUFX_E_ADDED;
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

/// The largest scale group that fits in a `u64`: 10^18 (scale 6).  `u64::MAX`
/// is ≈1.8×10^19, so the scale-6 group can be up to 18.
const MAX_SCALE_GROUP: u32 = 6;

/// Split a value into three-digit groups, most-significant first.  Scale `k`
/// covers 10^(3k): 1 = thousand, 2 = million, 3 = billion, 4 = trillion, … up
/// to the largest that fits in a `u64`.  Leading zero groups are emitted but
/// skipped by `append_cardinal_group`, so small numbers are unaffected; this
/// fixes large numbers ≥ 10^12, which previously overflowed the billion group
/// (`1_000_000_000_000` → "ten hundred billion" instead of "one trillion").
fn split_scale_groups(value: u64, dict: &Dictionary) -> Vec<ScaleGroup> {
    let mut groups = Vec::with_capacity(MAX_SCALE_GROUP as usize + 1);
    for scale in (0..=MAX_SCALE_GROUP).rev() {
        let divisor = 1_000u64.pow(scale);
        groups.push(ScaleGroup {
            value: ((value / divisor) % 1_000) as u32,
            scale: if scale == 0 { None } else { Some(scale as u8) },
        });
    }
    // Fold any non-zero group whose scale word this language does not define
    // down into the next-lower scale (×1000), so magnitude is still conveyed.
    // A language with every scale word (en/de/ru up to 10^18) is untouched; one
    // that stops earlier (French has no `_0M4` past "milliard") still reads
    // 10^12 as "dix cent milliards" rather than dropping to a bare "un".
    // Groups are ordered most-significant first, so each carry lands on a group
    // processed later in the loop and can cascade further down if needed.
    for i in 0..groups.len() {
        let Some(scale) = groups[i].scale else { continue };
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
        dst.push_pronunciation(&num3_phonemes_g(dict, count, false, grammar, feminine));
        dst.push_lookup_word(&scale_word);
        return;
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
    let singular_key = format!("_1M{scale}");

    if scale == 1 && group_value == 1 && grammar.thousands.omit_one_prefix {
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

    dst.push_pronunciation(&num3_phonemes(dict, group_value, false, grammar));
    dst.push_lookup_word(&scale_word);
}

fn append_cardinal_group(
    dst: &mut Pronunciation,
    group: ScaleGroup,
    dict: &Dictionary,
    grammar: &NumberGrammar,
    exact: bool,
) {
    if group.value == 0 {
        return;
    }

    if let Some(scale) = group.scale {
        append_scale_word(dst, group.value, scale, dict, grammar, exact);
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

    // Ordinal fallback (no `_0M{n}o` key): keep the plain scale word, not the
    // exact `x` variant — that variant is a cardinal-only distinction.
    append_scale_word(dst, group_value, scale, dict, grammar, false);
    false
}

/// Convert a number value (0-999) to phonemes.
/// Mirrors C's LookupNum3 with per-language number flags.
/// Phonemes for a unit digit, using the feminine form (`_1f`/`_2f`) for 1 and 2
/// when `feminine` — Slavic scale-word agreement ("одна", "две").
fn unit_ph(dict: &Dictionary, n: u32, feminine: bool) -> Vec<u8> {
    if feminine && (n == 1 || n == 2) {
        let f = lookup_num_phonemes(dict, &format!("_{n}f"));
        if !f.is_empty() {
            return f;
        }
    }
    lookup_num_phonemes(dict, &num_key(n))
}

fn num3_phonemes(
    dict: &Dictionary,
    value: u32,
    suppress_null: bool,
    grammar: &NumberGrammar,
) -> Pronunciation {
    num3_phonemes_g(dict, value, suppress_null, grammar, false)
}

/// Like [`num3_phonemes`], but with `feminine` gender for a trailing 1/2 unit
/// (Slavic thousands agreement).
fn num3_phonemes_g(
    dict: &Dictionary,
    value: u32,
    suppress_null: bool,
    grammar: &NumberGrammar,
    feminine: bool,
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
                hundreds_part.push_lookup_word(&lookup_num_phonemes(dict, &num_key(hundreds)));
            }
            // Bare hundred word.  For the Malayalam bare-1-hundred, an exact
            // hundred uses the dedicated `_0C0` form ("നൂറ്"); everyone else keeps
            // `_0C`, so no other language is affected.
            let bare = if skip_dedicated && tensunits == 0 {
                let e = lookup_num_phonemes(dict, "_0C0");
                if e.is_empty() { lookup_num_phonemes(dict, "_0C") } else { e }
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
            let whole = unit_ph(dict, tensunits, feminine);
            // Resolve the word for "ten": a dedicated `_10`, else the `_1X`
            // ten-prefix (Vietnamese "mười" has only `_1X`).
            let ten_word = || {
                let t = lookup_num_phonemes(dict, "_10");
                if t.is_empty() { lookup_num_phonemes(dict, "_1X") } else { t }
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
                let ten = if ten.is_empty() { lookup_num_phonemes(dict, "_10") } else { ten };
                if !ten.is_empty() {
                    tens_part.push_lookup_word(&ten);
                    // Some languages join the compositional teen with a
                    // conjunction ("kumi NA tano" = 15 in Swahili), the same one
                    // used between tens and units.  Turkish/Turkmen ("on beş")
                    // do not — they are `Standard`, so this stays gated.
                    if grammar.tens == TensGrammar::WithConjunction {
                        tens_part.append_lookup_suffix(&lookup_num_phonemes(dict, "_0and"));
                    }
                    tens_part.append_lookup_suffix(&lookup_num_phonemes(dict, &num_key(tensunits - 10)));
                }
            }
        } else {
            let ph_full = lookup_num_phonemes(dict, &num_key(tensunits));
            if !ph_full.is_empty() {
                tens_part.push_lookup_word(&ph_full);
            } else if grammar.vigesimal_70_90 && (70..80).contains(&tensunits) {
                // French 70–79 = "soixante" (60) + (10–19).
                tens_part.push_lookup_word(&lookup_num_phonemes(dict, "_6X"));
                tens_part.append_lookup_suffix(&lookup_num_phonemes(dict, &num_key(tensunits - 60)));
            } else if grammar.vigesimal_70_90 && (90..100).contains(&tensunits) {
                // French 90–99 = "quatre-vingt" (80) + (10–19).
                tens_part.push_lookup_word(&lookup_num_phonemes(dict, "_8X"));
                tens_part.append_lookup_suffix(&lookup_num_phonemes(dict, &num_key(tensunits - 80)));
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
                            // Italian: the tens word elides its final vowel before
                            // a vowel-initial unit (uno=1, otto=8) → "trentuno".
                            if grammar.elide_tens_vowel && (units == 1 || units == 8) {
                                tens_part.drop_final_phoneme();
                            }
                            // A trailing 1/2 agrees in gender ("двадцать одна").
                            tens_part.append_lookup_suffix(&unit_ph(dict, units, feminine));
                        }
                    }
                }
            }
        }
    }

    if hundreds > 0 && tensunits > 0 && grammar.hundreds.use_conjunction_with_remainder {
        hundreds_part.append_lookup_suffix(&lookup_num_phonemes(dict, "_0and"));
    } else if hundreds > 0 && tensunits > 0 && grammar.hundreds.conjunction_before_simple_remainder {
        // Only when the remainder is a single word (round ten, or a number with
        // a direct dictionary entry) — a compound remainder already has its own
        // conjunction, so a second one would be wrong ("honderd vier-en-dertig").
        let simple = tensunits % 10 == 0
            || !lookup_num_phonemes(dict, &num_key(tensunits)).is_empty();
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
        NumberToken::Cardinal(digits) => Some(cardinal_pronunciation(digits, dict, grammar)?.finish()),
        NumberToken::Decimal { integer, fractional } => {
            // An empty integer is a continued decimal ("1.2.3" → "…point three"):
            // render just the point and the fractional digits, no leading cardinal.
            let mut pronunciation = if integer.is_empty() {
                Pronunciation::default()
            } else {
                cardinal_pronunciation(integer, dict, grammar)?
            };
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
        for _ in 0..n_leading {
            result.push_lookup_word(&lookup_num_phonemes(dict, "_0"));
        }
        let value: u64 = digits.parse().ok()?;
        if value == 0 {
            // "000" → the leading zeros plus a final "zero" (an all-zero run).
            result.push_lookup_word(&lookup_num_phonemes(dict, "_0"));
        } else {
            for group in split_scale_groups(value, dict) {
                let exact = scale_group_is_exact(value, group.scale);
                append_cardinal_group(&mut result, group, dict, grammar, exact);
            }
        }
        return Some(result);
    }

    let value: u64 = digits.parse().ok()?;
    if value == 0 {
        let mut pronunciation = Pronunciation::default();
        pronunciation.push_lookup_word(&lookup_num_phonemes(dict, "_0"));
        return Some(pronunciation);
    }

    // A round hundred in 1100–1900 reads as "N hundred" ("nineteen hundred",
    // "quinze cents") — the common reading for years.  2000 and above use the
    // standard thousands form ("two thousand", "deux mille"); reading them as
    // "twenty hundred" / "vingt cents" was wrong (a plain number is never a year).
    let is_year_form = (1100..=1999).contains(&value) && value % 100 == 0;
    if is_year_form {
        let mut pronunciation = num3_phonemes(dict, (value / 100) as u32, false, grammar);
        pronunciation.append_lookup_suffix(&lookup_num_phonemes(dict, "_0C"));
        return Some(pronunciation);
    }

    let mut result = Pronunciation::default();
    for group in split_scale_groups(value, dict) {
        let exact = scale_group_is_exact(value, group.scale);
        append_cardinal_group(&mut result, group, dict, grammar, exact);
    }

    Some(result)
}

/// Whether a scale group is *exact* — the whole number has no non-zero digit
/// below this scale (so `1000` is exact at the thousands scale, `1234` is not).
/// A scale-less (units) group is always "exact".  Used to select the `_0M{n}x`
/// variant scale word.
fn scale_group_is_exact(value: u64, scale: Option<u8>) -> bool {
    match scale {
        Some(s) => value % 1000u64.pow(s as u32) == 0,
        None => true,
    }
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
    let groups = split_scale_groups(value, dict);
    // For a zero value there is no non-zero group; fall back to the *last* group
    // (the units, scale-less) so "0th" renders "zeroth" instead of dropping the
    // number or attaching a spurious scale word.
    let last_nonzero = groups
        .iter()
        .rposition(|group| group.value != 0)
        .unwrap_or(groups.len().saturating_sub(1));

    for &group in &groups[..last_nonzero] {
        // These groups all precede the last non-zero group, so a lower non-zero
        // digit always follows — never an exact scale group.
        append_cardinal_group(&mut pronunciation, group, dict, grammar, false);
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
/// Part-of-speech expectation for a word, derived from the *preceding* word (a
/// `FLAG_VERBF`/`NOUNF`/`PASTF` function word like "to"/"the"/"had").  Selects
/// the `$verb`/`$noun`/`$past` homograph pronunciation ("to lˈɪv" vs "lˈaɪv").
#[derive(Clone, Copy, Debug, Default)]
pub struct PosExpect {
    pub verb: bool,
    pub noun: bool,
    pub past: bool,
}

/// The part-of-speech expectation a word inherits from the preceding word.
/// English only; other languages get the default (no expectation).  Kept
/// conservative — only high-confidence verb/past triggers — since a false
/// positive mispronounces an otherwise-correct default.
fn pos_expect_after(prev: Option<&str>) -> PosExpect {
    let Some(p) = prev else { return PosExpect::default() };
    // After "to", a modal, or a subject pronoun, the next content word is
    // (almost always) a verb ("I lˈɪv", "we jˈuːz", "they rɪkˈɔːd").
    const VERB_TRIGGERS: &[&str] = &[
        "to", "will", "would", "shall", "should", "can", "could", "may", "might",
        "must", "cannot", "don't", "doesn't", "didn't", "let", "lets", "please",
        "i", "we", "you", "they", "he", "she", "it",
    ];
    // After a perfect/passive auxiliary, a homograph takes its past form
    // ("have rˈɛd", "was tˈɔːn").
    const PAST_TRIGGERS: &[&str] = &["had", "has", "have", "having", "was", "were", "been"];
    // After a determiner the next word is a noun/adjective, selecting the `$noun`
    // form ("a ˈkɒnvɜːt", "the ˈprɒdjuːs", "the lˈɜːnɪd professor").  Only the ~15
    // `$noun`-flagged words are affected; everything else keeps its default.
    const NOUN_TRIGGERS: &[&str] = &[
        "a", "an", "the", "this", "that", "these", "those", "my", "your", "his",
        "her", "its", "our", "their", "no", "some", "any", "each", "every", "another",
    ];
    PosExpect {
        verb: VERB_TRIGGERS.contains(&p),
        noun: NOUN_TRIGGERS.contains(&p),
        past: PAST_TRIGGERS.contains(&p),
    }
}

pub fn word_to_phonemes(
    word: &str,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    lang_opts: &LangOptions,
) -> WordResult {
    word_to_phonemes_inner(word, dict, phdata, stress_opts, lang_opts, 0, PosExpect::default())
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
    word_to_phonemes_inner(word, dict, phdata, stress_opts, lang_opts, 0, expect)
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
            let (l, v, t) = (s / (VCOUNT * TCOUNT), (s % (VCOUNT * TCOUNT)) / TCOUNT, s % TCOUNT);
            // Leading ㅇ (choseong ieung, index 11) is a silent placeholder for a
            // vowel-initial syllable — it never carries the "ng" sound (that is
            // only the *trailing* ᆼ), so omit it rather than emit the jamo's
            // letter-name pronunciation.
            if l != 11 {
                out.push(char::from_u32(LBASE + l).unwrap());
            }
            out.push(char::from_u32(VBASE + v).unwrap());
            if t != 0 {
                out.push(char::from_u32(TBASE + t).unwrap());
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
        lookup_symbol: true,
        expect_verb: expect.verb,
        expect_noun: expect.noun,
        expect_past: expect.past,
        dict_condition: lang_opts.dict_condition,
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
            // TEXTMODE entry: the "phonemes" are really a replacement *text*
            // (emoji-name entries like `😀`→"grinning face", `$text` rules).
            // Re-translate it as words instead of reading the bytes as phonemes.
            if result.flags1.textmode() && rule_word_flags & FLAG_IN_TEXTMODE == 0 {
                let replacement = String::from_utf8_lossy(&result.phonemes);
                let replacement = replacement.trim_end_matches('\0');
                let mut phonemes: Vec<u8> = Vec::new();
                for part in replacement.split_whitespace() {
                    let wr = word_to_phonemes_inner(
                        part, dict, phdata, stress_opts, lang_opts,
                        rule_word_flags | FLAG_IN_TEXTMODE, PosExpect::default(),
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
                if !phonemes.is_empty() {
                    return WordResult { phonemes, dict_flags: 0 };
                }
            }
            let dict_flags = result.flags1.0;
            // Language-switch entry (`_^_LL`): the phonemes are `phonSWITCH`
            // followed by the target language name.  Return them verbatim —
            // stress placement must not touch the embedded language bytes; the
            // caller re-translates the word with the target language.
            if result.phonemes[0] == crate::phoneme::PHON_SWITCH {
                return WordResult { phonemes: result.phonemes.clone(), dict_flags };
            }
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

    if rule_word_flags == 0 {
        if let Some(token) = NumberToken::parse(word, &lang_opts.number_grammar) {
            if let Some(result) =
                translate_number_token(&token, dict, phdata, stress_opts, &lang_opts.number_grammar)
            {
                return result;
            }
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
        rule_word_flags,
        0,   // dict_flags
        &letter_bits,
        lang_opts.dict_condition,
        &mut vowel_count,
        &mut stressed_count,
        Some(phdata),
    );

    fn rules_produced_output(r: &crate::dictionary::rules::RulesResult) -> bool {
        if r.spellword || r.end_type != 0 {
            return true;
        }
        r.phonemes.iter().any(|&b| b != 0) || r.end_phonemes.iter().any(|&b| b != 0)
    }

    if rules_produced_output(&result) {
        let mut stress_dict_flags = dict_flags_from_lookup;
        let mut phonemes = if result.end_type != 0 && (result.end_type & SUFX_P) != 0 {
            if let Some((stem, stem_word_flags)) = remove_standard_prefix(word, result.end_type) {
                let mut combined = Vec::new();
                append_raw_phonemes(&mut combined, &result.end_phonemes);
                let stem_wr = word_to_phonemes_inner(
                    &stem,
                    dict,
                    phdata,
                    stress_opts,
                    lang_opts,
                    stem_word_flags,
                    expect,
                );
                append_raw_phonemes(&mut combined, &stem_wr.phonemes);
                stress_dict_flags = stem_wr.dict_flags;
                combined.push(0);
                combined
            } else {
                let mut fallback = combine_rules_result(&result);
                fallback.push(0);
                fallback
            }
        } else if result.end_type != 0 && result.suffix_start > 1 {
            if let Some((stem, end_flags, stem_word_flags)) =
                remove_standard_suffix(word, result.end_type, dict)
            {
                let stem_lookup = lookup(
                    dict,
                    &stem,
                    &LookupCtx {
                        lookup_symbol: true,
                        end_flags,
                        expect_verb: expect.verb,
                        expect_noun: expect.noun,
                        expect_past: expect.past,
                        dict_condition: lang_opts.dict_condition,
                        ..Default::default()
                    },
                );

                let mut combined = Vec::new();
                let mut used_stem = false;

                if let Some(stem_lookup) = stem_lookup {
                    if !stem_lookup.phonemes.is_empty() {
                        combined.extend_from_slice(&stem_lookup.phonemes);
                        stress_dict_flags = stem_lookup.flags1.0;
                        used_stem = true;
                    }
                }

                if !used_stem {
                    let mut stem_buf = Vec::with_capacity(stem.len() + 3);
                    stem_buf.push(b' ');
                    stem_buf.extend_from_slice(stem.as_bytes());
                    stem_buf.push(b' ');
                    stem_buf.push(0);

                    let mut stem_vc = 0i32;
                    let mut stem_sc = 0i32;
                    let stem_rules = translate_rules_phdata(
                        dict,
                        &stem_buf,
                        1,
                        stem_word_flags,
                        0,
                        &letter_bits,
                        lang_opts.dict_condition,
                        &mut stem_vc,
                        &mut stem_sc,
                        Some(phdata),
                    );
                    let stem_phonemes = combine_rules_result(&stem_rules);
                    if !stem_phonemes.is_empty() {
                        combined.extend_from_slice(&stem_phonemes);
                        used_stem = true;
                    }
                }

                if used_stem {
                    append_raw_phonemes(&mut combined, &result.end_phonemes);
                    combined.push(0);
                    combined
                } else {
                    let mut fallback = combine_rules_result(&result);
                    fallback.push(0);
                    fallback
                }
            } else {
                let mut fallback = combine_rules_result(&result);
                fallback.push(0);
                fallback
            }
        } else {
            let mut combined = combine_rules_result(&result);
            combined.push(0);
            combined
        };

        // Apply stress placement; use dict flags from FLAGS-only entry if available
        let flags_for_stress = if stress_dict_flags != 0 {
            Some(stress_dict_flags as u32)
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
        return WordResult { phonemes, dict_flags: stress_dict_flags };
    }

    // Could not translate (unknown word)
    WordResult { phonemes: Vec::new(), dict_flags: dict_flags_from_lookup }
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
}

pub(crate) struct TranslateEntry {
    pub(crate) phonemes:   Vec<u8>,
    pub(crate) dict_flags: u32,
    pub(crate) kind:       TranslateEntryKind,
    pub(crate) word_lower: Option<String>,
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
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let maxlen = 4.min(bytes.len() - i);
        let mut matched = false;
        for len in (1..=maxlen).rev() {
            let Some(slice) = content.get(i..i + len) else { continue };
            let code = phdata.lookup_phoneme(slice);
            if code != 0 {
                codes.push(code);
                i += len;
                matched = true;
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
    ssml::process_markup(text)
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
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
];

/// Render an ISO `YYYY-MM-DD` (or `YYYY/MM/DD`) date as English words:
/// "2024-01-15" → "January 15th 2024" (read as "January fifteenth …").
/// Unrecognised input is returned unchanged (read normally).
fn dateize_english(text: &str) -> String {
    let parts: Vec<&str> = text.trim().split(['-', '/']).collect();
    if parts.len() == 3 && parts[0].len() == 4 {
        if let (Ok(y), Ok(m), Ok(d)) =
            (parts[0].parse::<u32>(), parts[1].parse::<u32>(), parts[2].parse::<u32>())
        {
            if (1..=12).contains(&m) && (1..=31).contains(&d) {
                let ds = d.to_string();
                return format!("{} {d}{} {y}", MONTHS[(m - 1) as usize], english_ordinal_suffix(&ds));
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
    matches!(c, '.' | '#' | '$' | '%' | '&' | '*' | '+' | '/' | '=' | '@' | '~'
                | '×' | '÷' | '°' | '±' | '−' | '§' | '№'
                | '∞' | '√' | '∑' | '≈' | '≠' | '≤' | '≥'
                | '℃' | '℉' | '™' | '®' | '©'
                | '‰' | '‱')
}

/// A currency symbol that prefixes an amount (`$5`, `€10`).  Matches the set
/// `currency_words` reads, so a leading minus before one is a negative amount.
fn is_currency_symbol(c: char) -> bool {
    matches!(c, '$' | '€' | '£' | '¥' | '¢')
}

/// Whether the characters after a leading `-` begin a number, so the hyphen is a
/// unary minus: a digit (`-5`), or a currency symbol then a digit — allowing one
/// optional space (`-$5`, `-$ 5`).  Takes a clone of the tokenizer iterator
/// positioned just past the `-`.
fn minus_precedes_number<I>(chars: &std::iter::Peekable<I>) -> bool
where
    I: Iterator<Item = char> + Clone,
{
    let mut la = chars.clone();
    match la.next() {
        Some(d) if d.is_ascii_digit() => true,
        Some(cur) if is_currency_symbol(cur) => {
            let mut next = la.next();
            if next == Some(' ') {
                next = la.next();
            }
            next.map_or(false, |d| d.is_ascii_digit())
        }
        _ => false,
    }
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
        if let Token::Number(NumberToken::Cardinal(first)) = &tokens[i] {
            if first.len() <= 3 && is_digits(first) {
                // Greedily consume `<space> <exactly-3-digit cardinal>` groups.
                let mut merged = first.clone();
                let mut j = i + 1;
                let mut groups = 0;
                while matches!(tokens.get(j), Some(Token::Space)) {
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
                if matches!(tokens.get(j), Some(Token::Space)) {
                    if let Some(Token::Number(NumberToken::Decimal { integer, fractional })) =
                        tokens.get(j + 1)
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
                        Some(fractional) => {
                            NumberToken::Decimal { integer: merged, fractional }
                        }
                        None => NumberToken::Cardinal(merged),
                    };
                    out.push(Token::Number(token));
                    i = j;
                    continue;
                }
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    *tokens = out;
}

/// A writing system the port can route to a dedicated voice.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Script {
    Cyrillic,
    Greek,
}

/// The script of a single character (Cyrillic or Greek), or `None` for Latin and
/// everything else.
fn char_script(c: char) -> Option<Script> {
    match c as u32 {
        0x0400..=0x052F => Some(Script::Cyrillic), // Cyrillic + Supplement
        0x0370..=0x03FF => Some(Script::Greek),    // Greek and Coptic
        _ => None,
    }
}

/// The script a voice reads natively, or `None` for Latin/other voices.
fn native_script(lang: &str) -> Option<Script> {
    match primary_bcp47_subtag(lang) {
        "ru" | "uk" | "bg" | "be" | "mk" | "sr" | "mn" | "kk" | "ky" | "tt" | "ba"
        | "cv" | "tg" | "os" | "cu" => Some(Script::Cyrillic),
        "el" | "grc" => Some(Script::Greek),
        _ => None,
    }
}

/// If `word` is written in a script the `lang` voice doesn't read (a Cyrillic or
/// Greek word inside, say, an English clause), return a voice that does, so the
/// word is re-translated instead of dropped.  Cyrillic → `ru`, Greek → `el`.
fn foreign_script_voice(word: &str, lang: &str) -> Option<&'static str> {
    let script = word.chars().find_map(char_script)?;
    if native_script(lang) == Some(script) {
        return None; // the current voice already reads this script
    }
    Some(match script {
        Script::Cyrillic => "ru",
        Script::Greek => "el",
    })
}

/// The canonical Roman numeral for `n` (`4` → "IV"), used to validate parses.
fn to_roman(mut n: u32) -> String {
    const TABLE: [(u32, &str); 13] = [
        (1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"),
        (50, "L"), (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I"),
    ];
    let mut out = String::new();
    for (v, sym) in TABLE {
        while n >= v {
            out.push_str(sym);
            n -= v;
        }
    }
    out
}

/// Parse a strict, canonical Roman numeral (`"XIV"` → `14`).  Rejects anything
/// non-canonical (`"IIII"`, `"VX"`) by round-tripping through `to_roman`, so real
/// words that happen to use Roman letters (`"DID"`, `"MIMIC"`) don't match.
fn roman_value(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let digit = |c| match c {
        'I' => Some(1),
        'V' => Some(5),
        'X' => Some(10),
        'L' => Some(50),
        'C' => Some(100),
        'D' => Some(500),
        'M' => Some(1000),
        _ => None,
    };
    let mut total: i64 = 0;
    let mut highest = 0;
    for c in s.chars().rev() {
        let v = digit(c)?;
        if v < highest {
            total -= v as i64;
        } else {
            total += v as i64;
            highest = v;
        }
    }
    let n = u32::try_from(total).ok()?;
    (n > 0 && to_roman(n) == s).then_some(n)
}

/// Words after which an uppercase Roman numeral is (almost) certainly a number,
/// read as a cardinal ("Chapter IV" → "four", "World War II" → "two").
fn is_roman_context_keyword(lower: &str) -> bool {
    matches!(
        lower,
        "chapter" | "part" | "section" | "book" | "volume" | "vol" | "act" | "scene"
            | "appendix" | "article" | "war" | "grade" | "level" | "phase" | "type"
            | "class" | "mark" | "figure" | "stage" | "episode" | "series"
    )
}

/// Convert an uppercase Roman numeral that immediately follows a recognised
/// enumeration keyword into a cardinal number ("Chapter IV" → "Chapter 4").
/// Keyword-gated to avoid misreading real all-caps words (MIX, MI, CID…) as
/// numbers; regnal ordinals ("Louis XIV") are deliberately left alone (they are
/// ambiguous — "Apollo XI" is *eleven*, not *the eleventh*).  English-only.
fn apply_roman_numerals(tokens: &mut Vec<Token>, lang: &str) {
    if primary_bcp47_subtag(lang) != "en" {
        return;
    }
    for i in 0..tokens.len() {
        let Token::Word(kw) = &tokens[i] else { continue };
        if !is_roman_context_keyword(&kw.to_lowercase()) {
            continue;
        }
        // The next token past a single space.
        let j = if matches!(tokens.get(i + 1), Some(Token::Space)) { i + 2 } else { i + 1 };
        if let Some(Token::Word(w)) = tokens.get(j) {
            // Uppercase only (lowercase roman collides with words like "i"/"mix").
            if w.chars().all(|c| c.is_ascii_uppercase()) {
                if let Some(n) = roman_value(w) {
                    tokens[j] = Token::Number(NumberToken::Cardinal(n.to_string()));
                }
            }
        }
    }
}

/// A title/abbreviation that precedes a name, after which a period is part of
/// the abbreviation rather than a sentence end ("Dr. Smith", "St. Louis").
/// Lowercased; English-only.  Deliberately limited to titles that almost always
/// precede a name — an ambiguous or trailing abbreviation ("etc.", "Jr.") keeps
/// its clause break so a real sentence end is never swallowed.
fn is_period_abbreviation(lower: &str) -> bool {
    matches!(
        lower,
        "dr" | "mr" | "mrs" | "ms" | "mx" | "prof" | "rev" | "fr" | "st"
            | "gen" | "col" | "sgt" | "capt" | "lt" | "maj" | "cmdr" | "adm"
            | "gov" | "sen" | "rep" | "pres" | "hon" | "sir" | "messrs"
            | "mme" | "mlle"
    )
}

/// Downgrade a clause-ending period that immediately follows a title
/// abbreviation to a plain word break, so "Dr. Smith" reads without a sentence
/// pause (upstream #2340).  A period after any other word (a real sentence end,
/// "the end. Next.") is untouched.  Adjacency is implicit: a space before the
/// period would make the previous token a `Space`, not the abbreviation `Word`.
/// English-only.
fn suppress_abbreviation_periods(tokens: &mut [Token], lang: &str) {
    if primary_bcp47_subtag(lang) != "en" {
        return;
    }
    for i in 1..tokens.len() {
        if matches!(tokens[i], Token::ClauseBoundary('.')) {
            if let Token::Word(w) = &tokens[i - 1] {
                if is_period_abbreviation(&w.to_lowercase()) {
                    tokens[i] = Token::Space;
                }
            }
        }
    }
}

/// Expand a common, unambiguous English abbreviation whose letters would
/// otherwise read as an unpronounceable cluster ("vs"→"vz", "govt"→"govt") or a
/// wrong word.  Only entries that are *not* real words and have a single obvious
/// expansion are listed (so `St`=saint/street and `No`=number/no are excluded).
fn expand_abbreviation(lower: &str) -> Option<&'static str> {
    Some(match lower {
        "vs" => "versus",
        "govt" => "government",
        "blvd" => "boulevard",
        "sgt" => "sergeant",
        "capt" => "captain",
        "lt" => "lieutenant",
        "rd" => "road",
        "inc" => "incorporated",
        "prof" => "professor",
        "approx" => "approximately",
        "misc" => "miscellaneous",
        _ => return None,
    })
}

/// Read a `H:MM` (or `H:MM:SS`) time.  The colon is a time separator, not a
/// clause boundary: it is silent, the hour drops any leading zero (`02:30` →
/// "two thirty", `00:30` → "zero thirty"), and languages with a spoken connector
/// insert it between hour and minute for a valid hour (0–23) — German "uhr",
/// French "heures", Italian "e" (`10:30` → "zehn uhr dreißig" / "dix heures
/// trente" / "dieci e trenta").  A field must be *exactly* two digits, so `10:3`
/// and `10:300` are not times (their `:` stays a boundary).  The minute keeps its
/// leading zero (`10:05` → "…zero five"), matching espeak-ng.
fn apply_time_reading(tokens: &mut Vec<Token>, lang: &str) {
    let connector = match primary_bcp47_subtag(lang) {
        "de" => Some("uhr"),
        "fr" => Some("heures"),
        "it" => Some("e"),
        _ => None,
    };
    let is_colon = |t: Option<&Token>| matches!(t, Some(Token::Punctuation(':')));
    let read_colon = primary_bcp47_subtag(lang) == "en";
    // A cardinal token whose surface is exactly two digits (a minute/second).
    let two_digit = |t: Option<&Token>| -> Option<String> {
        match t {
            Some(Token::Number(NumberToken::Cardinal(s)))
                if s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit()) =>
            {
                Some(s.clone())
            }
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        // Hour: a run of digits immediately followed by ':' + two-digit minutes.
        let hour = match &tokens[i] {
            Token::Number(NumberToken::Cardinal(h)) if h.bytes().all(|b| b.is_ascii_digit()) => {
                Some(h.clone())
            }
            _ => None,
        };
        if let Some(h) = hour {
            if is_colon(tokens.get(i + 1)) {
                if let Some(minute) = two_digit(tokens.get(i + 2)) {
                    // Hour with leading zeros stripped, but never to nothing.
                    let h_trim = h.trim_start_matches('0');
                    let h_norm = if h_trim.is_empty() { "0" } else { h_trim };
                    let h_val: u32 = h.parse().unwrap_or(u32::MAX);
                    out.push(Token::Number(NumberToken::Cardinal(h_norm.to_string())));
                    out.push(Token::Space);
                    if let Some(word) = connector {
                        if h_val <= 23 {
                            out.push(Token::Word(word.to_string()));
                            out.push(Token::Space);
                        }
                    }
                    out.push(Token::Number(NumberToken::Cardinal(minute)));
                    let mut j = i + 3;
                    // Optional ":SS" seconds — colon silent, no connector.
                    if is_colon(tokens.get(j)) {
                        if let Some(sec) = two_digit(tokens.get(j + 1)) {
                            out.push(Token::Space);
                            out.push(Token::Number(NumberToken::Cardinal(sec)));
                            j += 2;
                        }
                    }
                    i = j;
                    continue;
                }
            }
        }
        // A ':' that isn't a valid time (a ratio like "3:2"): English reads it as
        // "colon", every other language drops it — neither breaks the clause.
        if matches!(&tokens[i], Token::Punctuation(':')) {
            if read_colon {
                out.push(Token::Space);
                out.push(Token::Word("colon".to_string()));
                out.push(Token::Space);
            }
            i += 1;
            continue;
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    *tokens = out;
}

/// A hyphen-minus `-` directly between two numbers is read as the "-" symbol,
/// which espeak-ng voices only for some languages: English "dash", German
/// "Strich" (`5-10` → "five dash ten" / "fünf strich zehn", `3-4-5` → "three
/// dash four dash five").  Every other language drops it (the port already did
/// for all), so this is purely additive.  Only fires between two `Number`s — a
/// letter on either side (`x-5`, `5-x`) or a word-joining hyphen ("well-known",
/// handled in the tokenizer) is untouched, and en/em dashes are not `-`.
fn apply_dash_reading(tokens: &mut Vec<Token>, lang: &str) {
    let dash = match primary_bcp47_subtag(lang) {
        "en" => "dash",
        "de" => "Strich",
        _ => return,
    };
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if matches!(&tokens[i], Token::Number(_))
            && matches!(tokens.get(i + 1), Some(Token::Punctuation('-')))
            && matches!(tokens.get(i + 2), Some(Token::Number(_)))
        {
            out.push(tokens[i].clone()); // the first number
            out.push(Token::Space);
            out.push(Token::Word(dash.to_string()));
            out.push(Token::Space);
            // Leave the second number for the next iteration so a chain
            // ("3-4-5") keeps matching.
            i += 2;
            continue;
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    *tokens = out;
}

/// "No." before a number is the numero abbreviation, read "number" (`No. 5` →
/// "number five", like the `№` sign).  Requires a capital "No", a period, and a
/// following number — so the word "no" (`No way`, `No. I refuse`) is left alone.
/// English-only.
fn apply_number_abbrev(tokens: &mut Vec<Token>, lang: &str) {
    if primary_bcp47_subtag(lang) != "en" {
        return;
    }
    let is_dot = |t: Option<&Token>| {
        matches!(t, Some(Token::Punctuation('.')) | Some(Token::ClauseBoundary('.')))
    };
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if matches!(&tokens[i], Token::Word(w) if w == "No") && is_dot(tokens.get(i + 1)) {
            let j = if matches!(tokens.get(i + 2), Some(Token::Space)) { i + 3 } else { i + 2 };
            if matches!(tokens.get(j), Some(Token::Number(_))) {
                out.push(Token::Word("number".to_string()));
                out.push(Token::Space);
                i = j; // continue from the number itself (the period is dropped)
                continue;
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    *tokens = out;
}

/// A lowercase `x` between two numbers is a dimension separator, read "by":
/// `8x10` → "eight by ten", `1920x1080` → "…by…".  English-only; requires a
/// number on both sides so algebra (`2x`) and multipliers (`3x faster`) — which
/// have no number after the `x` — are left as the letter.
fn apply_dimensions(tokens: &mut [Token], lang: &str) {
    if primary_bcp47_subtag(lang) != "en" {
        return;
    }
    // A real dimension has a non-zero number on each side; a `0` side means it's
    // something else (a hex literal like `0x1F` → "0","x","1","F").
    let is_num = |t: Option<&Token>| {
        matches!(t, Some(Token::Number(NumberToken::Cardinal(n))) if n != "0")
            || matches!(t, Some(Token::Number(NumberToken::Decimal { .. })))
    };
    for i in 0..tokens.len() {
        if !matches!(&tokens[i], Token::Word(w) if w == "x") {
            continue;
        }
        let prev = match (i, tokens.get(i.wrapping_sub(1))) {
            (0, _) => None,
            (_, Some(Token::Space)) => tokens.get(i.wrapping_sub(2)),
            (_, t) => t,
        };
        let next = match tokens.get(i + 1) {
            Some(Token::Space) => tokens.get(i + 2),
            t => t,
        };
        if is_num(prev) && is_num(next) {
            tokens[i] = Token::Word("by".to_string());
        }
    }
}

/// Replace recognised abbreviations with their full word (English-only).
fn apply_abbreviations(tokens: &mut [Token], lang: &str) {
    if primary_bcp47_subtag(lang) != "en" {
        return;
    }
    for tok in tokens.iter_mut() {
        if let Token::Word(w) = tok {
            if let Some(expanded) = expand_abbreviation(&w.to_lowercase()) {
                *w = expanded.to_string();
            }
        }
    }
}

/// A degree sign followed by `C`/`F` is a temperature scale, not a stray letter:
/// `20°C` → "twenty degrees Celsius".  The `°` already reads "degrees" (or the
/// language's word); this just expands the trailing `C`/`F`.  Any language.
fn apply_temperature(tokens: &mut Vec<Token>, _lang: &str) {
    for i in 0..tokens.len() {
        if !matches!(tokens[i], Token::Punctuation('°')) {
            continue;
        }
        let j = if matches!(tokens.get(i + 1), Some(Token::Space)) { i + 2 } else { i + 1 };
        if let Some(Token::Word(w)) = tokens.get(j) {
            let scale = match w.as_str() {
                "C" => Some("celsius"),
                "F" => Some("fahrenheit"),
                _ => None,
            };
            if let Some(s) = scale {
                tokens[j] = Token::Word(s.to_string());
            }
        }
    }
}

/// Spell out a common, unambiguous unit abbreviation as `(singular, plural)` for
/// the language.  Only multi-letter units are listed — single letters (`m`, `g`,
/// `s`, `W`) are too ambiguous.  English uses British spelling (metre/litre); the
/// continental languages get the regular metric words (data/frequency units,
/// where "byte"/"octet" and forms differ more, stay English-only).
fn unit_words(abbr: &str, lang: &str) -> Option<(&'static str, &'static str)> {
    Some(match primary_bcp47_subtag(lang) {
        "en" => match abbr {
            "km" => ("kilometre", "kilometres"),
            "cm" => ("centimetre", "centimetres"),
            "mm" => ("millimetre", "millimetres"),
            "kg" => ("kilogram", "kilograms"),
            "mg" => ("milligram", "milligrams"),
            "ml" => ("millilitre", "millilitres"),
            "kb" => ("kilobyte", "kilobytes"),
            "mb" => ("megabyte", "megabytes"),
            "gb" => ("gigabyte", "gigabytes"),
            "tb" => ("terabyte", "terabytes"),
            "kw" => ("kilowatt", "kilowatts"),
            "hz" => ("hertz", "hertz"),
            "khz" => ("kilohertz", "kilohertz"),
            "mhz" => ("megahertz", "megahertz"),
            "ghz" => ("gigahertz", "gigahertz"),
            "mph" => ("mile per hour", "miles per hour"),
            _ => return None,
        },
        // German metric units are invariable (fünf Kilometer).
        "de" => match abbr {
            "km" => ("kilometer", "kilometer"),
            "cm" => ("zentimeter", "zentimeter"),
            "mm" => ("millimeter", "millimeter"),
            "kg" => ("kilogramm", "kilogramm"),
            "mg" => ("milligramm", "milligramm"),
            "ml" => ("milliliter", "milliliter"),
            _ => return None,
        },
        "fr" => match abbr {
            "km" => ("kilomètre", "kilomètres"),
            "cm" => ("centimètre", "centimètres"),
            "mm" => ("millimètre", "millimètres"),
            "kg" => ("kilogramme", "kilogrammes"),
            "mg" => ("milligramme", "milligrammes"),
            "ml" => ("millilitre", "millilitres"),
            _ => return None,
        },
        "es" => match abbr {
            "km" => ("kilómetro", "kilómetros"),
            "cm" => ("centímetro", "centímetros"),
            "mm" => ("milímetro", "milímetros"),
            "kg" => ("kilogramo", "kilogramos"),
            "mg" => ("miligramo", "miligramos"),
            "ml" => ("mililitro", "mililitros"),
            _ => return None,
        },
        "it" => match abbr {
            "km" => ("chilometro", "chilometri"),
            "cm" => ("centimetro", "centimetri"),
            "mm" => ("millimetro", "millimetri"),
            "kg" => ("chilogrammo", "chilogrammi"),
            "mg" => ("milligrammo", "milligrammi"),
            "ml" => ("millilitro", "millilitri"),
            _ => return None,
        },
        _ => return None,
    })
}

/// Read a unit abbreviation after a number as its full word (`5 km` → "five
/// kilometres", `1 kg` → "one kilogram").  The unit may abut the number (`5km`)
/// or be separated by a single space.  Languages without a table are unaffected.
fn apply_units(tokens: &mut Vec<Token>, lang: &str) {
    let after = |i: usize| if matches!(tokens.get(i + 1), Some(Token::Space)) { i + 2 } else { i + 1 };
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if let Token::Number(n) = &tokens[i] {
            let j = after(i);
            if let Some(Token::Word(w)) = tokens.get(j) {
                if let Some((sing, plur)) = unit_words(&w.to_lowercase(), lang) {
                    let one = matches!(n, NumberToken::Cardinal(s) if s == "1");
                    out.push(tokens[i].clone());
                    out.push(Token::Word(if one { sing } else { plur }.to_string()));
                    i = j + 1;
                    continue;
                }
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    *tokens = out;
}

/// English decades: a bare "s" right after a round-ten number is a spoken decade,
/// not a letter — `1980s` → "nineteen eighties", `80s`/`'80s` → "eighties",
/// `1900s` → "nineteen hundreds".  English-only.
fn apply_decades(tokens: &mut Vec<Token>, lang: &str) {
    if primary_bcp47_subtag(lang) != "en" {
        return;
    }
    // A trailing plural "s", allowing a possessive apostrophe ("1980s",
    // "1980's", "1980’s") — the "s"/"'s" may be one token or a separate `'`.
    let is_s = |t: Option<&Token>| {
        matches!(t, Some(Token::Word(w)) if w.trim_start_matches(['\'', '’']).eq_ignore_ascii_case("s"))
    };
    let apostrophe = |t: Option<&Token>| matches!(t, Some(Token::Punctuation('\'' | '’')));

    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if let Token::Number(NumberToken::Cardinal(digits)) = &tokens[i] {
            let consumed = if is_s(tokens.get(i + 1)) {
                2
            } else if apostrophe(tokens.get(i + 1)) && is_s(tokens.get(i + 2)) {
                3
            } else {
                0
            };
            if consumed > 0 {
                if let Some(decade) = decade_reading(digits) {
                    out.extend(decade);
                    i += consumed;
                    continue;
                }
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    *tokens = out;
}

/// The spoken tokens for a written decade `<digits>s`, or `None` if it isn't one
/// (not a round ten, or outside the handled range).  The century of a 4-digit
/// year reads as a number ("nineteen"); the decade pluralises ("eighties").
fn decade_reading(digits: &str) -> Option<Vec<Token>> {
    let n: u32 = digits.parse().ok()?;
    if n % 10 != 0 {
        return None;
    }
    let tens_plural = |t: u32| -> Option<&'static str> {
        Some(match t {
            1 => "tens", 2 => "twenties", 3 => "thirties", 4 => "forties",
            5 => "fifties", 6 => "sixties", 7 => "seventies", 8 => "eighties", 9 => "nineties",
            _ => return None,
        })
    };
    // Bare two-digit decade ("80s" → "eighties", "'90s" → "nineties").
    if (10..=90).contains(&n) {
        return Some(vec![Token::Word(tens_plural(n / 10)?.to_string())]);
    }
    // Full 11xx–19xx year decade ("1980s" → "nineteen eighties", "1900s" →
    // "nineteen hundreds").  2000s are ambiguous ("two thousands" vs "twenty
    // twenties") and left alone.  The century reads as its teen word ("nineteen").
    if (1100..=1999).contains(&n) {
        const TEENS: [&str; 9] = [
            "eleven", "twelve", "thirteen", "fourteen", "fifteen",
            "sixteen", "seventeen", "eighteen", "nineteen",
        ];
        let (century, dec) = (n / 100, n % 100);
        Some(vec![
            Token::Word(TEENS[(century - 11) as usize].to_string()),
            Token::Word(if dec == 0 { "hundreds".to_string() } else { tens_plural(dec / 10)?.to_string() }),
        ])
    } else {
        None
    }
}

/// Read `<base>^<exp>` as "<base> to the power of <exp>" (`2^10` → "two to the
/// power of ten") — the caret is otherwise dropped.  Only between two numbers
/// (an unambiguous exponent), English-only.
fn apply_exponents(tokens: &mut Vec<Token>, lang: &str) {
    if primary_bcp47_subtag(lang) != "en" {
        return;
    }
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if let Token::Number(base) = &tokens[i] {
            if matches!(tokens.get(i + 1), Some(Token::Punctuation('^'))) {
                if let Some(Token::Number(exp)) = tokens.get(i + 2) {
                    out.push(Token::Number(base.clone()));
                    for w in ["to", "the", "power", "of"] {
                        out.push(Token::Word(w.to_string()));
                    }
                    out.push(Token::Number(exp.clone()));
                    i += 3;
                    continue;
                }
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    *tokens = out;
}

/// English fraction denominator word for a small denominator (`2`–`10`),
/// singular or plural — `1/2`→"half", `3/4`→"quarters".  `None` otherwise.
fn fraction_denominator_word(denom: &str, plural: bool) -> Option<&'static str> {
    Some(match (denom, plural) {
        ("2", false) => "half",     ("2", true) => "halves",
        ("3", false) => "third",    ("3", true) => "thirds",
        ("4", false) => "quarter",  ("4", true) => "quarters",
        ("5", false) => "fifth",    ("5", true) => "fifths",
        ("6", false) => "sixth",    ("6", true) => "sixths",
        ("7", false) => "seventh",  ("7", true) => "sevenths",
        ("8", false) => "eighth",   ("8", true) => "eighths",
        ("9", false) => "ninth",    ("9", true) => "ninths",
        ("10", false) => "tenth",   ("10", true) => "tenths",
        _ => return None,
    })
}

/// Read `<num>/<den>` as a spoken fraction (`1/2`→"one half", `3/4`→"three
/// quarters") for small denominators — this also renders the vulgar-fraction
/// characters, which `normalize_number_symbols` turns into `n/m`.  English-only;
/// a trailing `/` (a date like `1/2/2024`) is left alone.
fn apply_fractions(tokens: &mut Vec<Token>, lang: &str) {
    if primary_bcp47_subtag(lang) != "en" {
        return;
    }
    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if let Token::Number(NumberToken::Cardinal(num)) = &tokens[i] {
            let is_slash = matches!(tokens.get(i + 1), Some(Token::Punctuation('/')));
            let not_date = !matches!(tokens.get(i + 3), Some(Token::Punctuation('/')));
            if is_slash && not_date {
                if let Some(Token::Number(NumberToken::Cardinal(den))) = tokens.get(i + 2) {
                    if let Some(word) = fraction_denominator_word(den, num != "1") {
                        // A whole number immediately before (`1 1/2`, i.e. `1½`) is a
                        // mixed number → "<whole> and <num> <fraction>" ("one and one
                        // half").  (The idiomatic "and a half" is avoided: a bare "a"
                        // reads as the letter name, not the article.)
                        let mixed = matches!(out.last(), Some(Token::Space))
                            && matches!(out.get(out.len().wrapping_sub(2)), Some(Token::Number(NumberToken::Cardinal(_))));
                        if mixed {
                            out.pop(); // drop the space between whole and fraction
                            out.push(Token::Word("and".to_string()));
                        }
                        out.push(Token::Number(NumberToken::Cardinal(num.clone())));
                        out.push(Token::Word(word.to_string()));
                        i += 3;
                        continue;
                    }
                }
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    *tokens = out;
}

/// The singular/plural spoken words for a currency symbol in the given language,
/// or `None` if that currency isn't spelled out for the language.  Invariable
/// nouns (German "Euro", Italian "euro") repeat the same word for both.
fn currency_words(c: char, lang: &str) -> Option<(&'static str, &'static str)> {
    Some(match (primary_bcp47_subtag(lang), c) {
        ("en", '$') => ("dollar", "dollars"),
        ("en", '€') => ("euro", "euros"),
        ("en", '£') => ("pound", "pounds"),
        ("en", '¥') => ("yen", "yen"),
        ("en", '¢') => ("cent", "cents"),
        // Eurozone / continental readings (word follows the amount).
        ("de", '€') => ("euro", "euro"),
        ("de", '$') => ("dollar", "dollar"),
        ("de", '£') => ("pfund", "pfund"),
        ("fr", '€') => ("euro", "euros"),
        ("fr", '$') => ("dollar", "dollars"),
        ("fr", '£') => ("livre", "livres"),
        ("es", '€') => ("euro", "euros"),
        ("es", '$') => ("dólar", "dólares"),
        ("es", '£') => ("libra", "libras"),
        ("it", '€') => ("euro", "euro"),
        ("it", '$') => ("dollaro", "dollari"),
        ("it", '£') => ("sterlina", "sterline"),
        ("pt", '€') => ("euro", "euros"),
        ("pt", '$') => ("dólar", "dólares"),
        ("pt", '£') => ("libra", "libras"),
        ("nl", '€') => ("euro", "euro"),
        ("nl", '$') => ("dollar", "dollar"),
        ("nl", '£') => ("pond", "pond"),
        _ => return None,
    })
}

/// The apocopated "one" used before a currency noun when the amount is exactly
/// one — German "ein Euro" (not "eins"), Spanish/Italian "un euro" (not "uno").
/// `None` where the plain cardinal is already right (French "un", Portuguese
/// "um", English "one").  `£` is feminine in es/it → "una".
fn currency_one_word(lang: &str, c: char) -> Option<&'static str> {
    match (primary_bcp47_subtag(lang), c) {
        ("de", _) => Some("ein"),      // Euro/Dollar masc., Pfund neut. → "ein"
        ("es", '£') | ("it", '£') => Some("una"), // libra/sterlina fem.
        ("es", _) | ("it", _) => Some("un"),
        _ => None,
    }
}

/// A large-number scale word that can follow a currency amount (`$5 million`).
/// English only — used to keep the scale before the currency word.
fn is_scale_word(w: &str) -> bool {
    matches!(
        w,
        "thousand" | "million" | "billion" | "trillion" | "quadrillion" | "quintillion"
    )
}

/// Emit a currency `amount` as spoken tokens: `<n> <unit>` (pluralised unless the
/// value is one), splitting a decimal amount into a whole part and a two-digit
/// cents part (`$5.99` → "five dollars ninety nine cents").
fn emit_currency_amount(
    out: &mut Vec<Token>,
    amount: &NumberToken,
    sing: &str,
    plur: &str,
    split_cents: bool,
    one_word: Option<&str>,
) {
    let push_unit = |out: &mut Vec<Token>, n_str: &str, sing: &str, plur: &str| {
        let one = n_str.trim_start_matches('0') == "1";
        out.push(Token::Number(NumberToken::Cardinal(n_str.to_string())));
        out.push(Token::Word(if one { sing } else { plur }.to_string()));
    };
    match amount {
        // `$5.99` → "five dollars ninety nine cents" (English cents split only;
        // other languages read the decimal as-is + the currency word).
        NumberToken::Decimal { integer, fractional } if split_cents => {
            let mut cents: String = fractional.chars().take(2).collect();
            while cents.len() < 2 {
                cents.push('0');
            }
            let dollars_zero = integer.chars().all(|d| d == '0');
            let cents_zero = cents == "00";
            if !dollars_zero || cents_zero {
                push_unit(out, integer, sing, plur);
            }
            if !cents_zero {
                push_unit(out, &cents, "cent", "cents");
            }
        }
        _ => {
            let one = matches!(amount, NumberToken::Cardinal(s) if s == "1");
            // "ein Euro" / "un euro": use the apocopated "one" before the noun
            // instead of the counting cardinal ("eins"/"uno").
            if let (true, Some(w)) = (one, one_word) {
                out.push(Token::Word(w.to_string()));
            } else {
                out.push(Token::Number(amount.clone()));
            }
            out.push(Token::Word(if one { sing } else { plur }.to_string()));
        }
    }
}

/// Rewrite a currency amount so it is read `<number> <currency word>` — the
/// symbol may precede the figure (`$5`, US style) or follow it (`5$`/`10€`,
/// continental style), with an optional space either way.  Pluralised unless the
/// amount is one; decimals split into whole + cents (see [`emit_currency_amount`]).
/// A bare symbol with no adjacent number is left untouched (it still reads via
/// the symbol-name path).  Cents are split only in English (`$5.99` → "…ninety
/// nine cents"); other languages read the decimal amount as-is + the word.
fn apply_currency(tokens: &mut Vec<Token>, lang: &str) {
    if !tokens.iter().any(|t| matches!(t, Token::Punctuation(c) if currency_words(*c, lang).is_some())) {
        return;
    }
    let split_cents = primary_bcp47_subtag(lang) == "en";
    // The token after `i`, skipping one optional space.
    let after = |i: usize| if matches!(tokens.get(i + 1), Some(Token::Space)) { i + 2 } else { i + 1 };

    let mut out = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        // Symbol before number: `$5`, `€5`.
        if let Token::Punctuation(c) = tokens[i] {
            if let Some((sing, plur)) = currency_words(c, lang) {
                let j = after(i);
                if let Some(Token::Number(n)) = tokens.get(j) {
                    // A scale word after the amount ("$5 million") belongs *before*
                    // the currency: "five million dollars", not "five dollars
                    // million".  The amount is read as-is (no cents split) + the
                    // scale + the plural currency word.
                    let k = after(j);
                    if let Some(Token::Word(w)) = tokens.get(k) {
                        if is_scale_word(&w.to_lowercase()) {
                            out.push(Token::Number(n.clone()));
                            out.push(Token::Word(w.clone()));
                            out.push(Token::Word(plur.to_string()));
                            i = k + 1;
                            continue;
                        }
                    }
                    emit_currency_amount(&mut out, n, sing, plur, split_cents, currency_one_word(lang, c));
                    i = j + 1;
                    continue;
                }
            }
        }
        // Number before symbol: `5$`, `10€`, `99¢`.
        if let Token::Number(n) = &tokens[i] {
            let j = after(i);
            if let Some(Token::Punctuation(c)) = tokens.get(j) {
                if let Some((sing, plur)) = currency_words(*c, lang) {
                    emit_currency_amount(&mut out, n, sing, plur, split_cents, currency_one_word(lang, *c));
                    i = j + 1;
                    continue;
                }
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    *tokens = out;
}

/// Look up the spoken name of a standalone symbol via the dictionary's
/// character-name entries (`_%`, `_$`, …), mirroring `LookupCharName`.
/// Returns the phoneme bytes, or empty if the symbol has no name in this voice.
fn lookup_symbol_name(
    c: char,
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    options: &LangOptions,
) -> Vec<u8> {
    // A few symbols have a short "word" reading that differs from their
    // spell-out character name (`_&` = "ampersand" but `&` is read "and").
    // These readings are language-specific.
    let lang = primary_bcp47_subtag(&options.lang);
    let word: Option<&str> = if lang == "en" {
        match c {
            '&' => Some("and"),
            '+' => Some("plus"),
            '@' => Some("at"),
            '×' => Some("times"),
            '÷' => Some("divided by"),
            '°' => Some("degrees"),
            '±' => Some("plus or minus"),
            '−' => Some("minus"), // U+2212 MINUS SIGN (not ASCII hyphen)
            '§' => Some("section"),
            '№' => Some("numero"),
            '∞' => Some("infinity"),
            '√' => Some("square root"),
            '∑' => Some("sum"),
            '≈' => Some("approximately equal to"),
            '≠' => Some("not equal to"),
            '≤' => Some("less than or equal to"),
            '≥' => Some("greater than or equal to"),
            '℃' => Some("degrees celsius"),
            '℉' => Some("degrees fahrenheit"),
            '™' => Some("trademark"),
            '®' => Some("registered trademark"),
            '©' => Some("copyright"),
            '‰' => Some("per thousand"),
            '‱' => Some("per ten thousand"),
            _ => None,
        }
    } else {
        // Confident readings for the common arithmetic/degree symbols in a few
        // major languages.  Without these, the dictionary character-name
        // fallback below spells the symbol out as garbage letters (e.g. German
        // `+` → "ˈɪːɜˌaʊɐ", `°` → "r.ˈʊʑç"), which is worse than dropping it.
        match c {
            '+' => match lang {
                "de" | "fr" | "nl" | "pl" | "cs" | "ro" | "sv" | "da" => Some("plus"),
                "nb" => Some("pluss"),
                "es" => Some("más"),
                "pt" => Some("mais"),
                "it" => Some("più"),
                "ru" | "uk" | "bg" => Some("плюс"),
                "tr" => Some("artı"),
                "he" => Some("פְּלוּס"),
                _ => None,
            },
            '−' => match lang {
                "de" | "pl" | "cs" | "ro" | "sv" | "da" | "nb" => Some("minus"),
                "fr" => Some("moins"),
                "es" | "pt" => Some("menos"),
                "it" => Some("meno"),
                "nl" => Some("min"),
                "ru" | "bg" => Some("минус"),
                "uk" => Some("мінус"),
                "tr" => Some("eksi"),
                "he" => Some("מִנוּס"),
                _ => None,
            },
            // Degree sign — read as "degrees".  A fixed plural form is used for
            // the Slavic/Nordic languages that drop it; temperatures are almost
            // always > 1°, so the plural is right in the common case (and any
            // reading beats the silent drop).
            '°' => match lang {
                "de" => Some("grad"),
                "fr" => Some("degrés"),
                "es" => Some("grados"),
                "it" => Some("gradi"),
                "nl" => Some("graden"),
                "sv" | "da" | "nb" => Some("grader"),
                "uk" => Some("градусів"),
                "ru" => Some("градусов"),
                "bg" => Some("градуса"),
                "he" => Some("מַעֲלוֹת"),
                _ => None,
            },
            // Multiplication sign — currently dropped (silent) outside the five
            // languages below, so any standard reading is an improvement.
            '×' => match lang {
                "de" => Some("mal"),
                "fr" => Some("fois"),
                "es" => Some("por"),
                "it" => Some("per"),
                "nl" => Some("keer"),
                "pt" => Some("vezes"),
                "pl" => Some("razy"),
                "cs" => Some("krát"),
                "ro" => Some("ori"),
                "sv" => Some("gånger"),
                "da" => Some("gange"),
                "nb" => Some("ganger"),
                "tr" => Some("çarpı"),
                "ru" => Some("умножить на"),
                "uk" => Some("помножити на"),
                "bg" => Some("по"),
                "he" => Some("כָּפוּל"),
                _ => None,
            },
            '÷' => match lang {
                "de" => Some("geteilt durch"),
                "fr" => Some("divisé par"),
                "es" => Some("dividido por"),
                "it" => Some("diviso"),
                "nl" => Some("gedeeld door"),
                "pt" => Some("dividido por"),
                "pl" => Some("przez"),
                "cs" => Some("děleno"),
                "ro" => Some("împărțit la"),
                "sv" => Some("delat med"),
                "da" => Some("divideret med"),
                "nb" => Some("delt på"),
                "tr" => Some("bölü"),
                "ru" => Some("разделить на"),
                "uk" => Some("поділити на"),
                "bg" => Some("разделено на"),
                "he" => Some("חֶלְקֵי"),
                _ => None,
            },
            // Plus-minus (tolerance) sign — read "plus minus" in the languages
            // that drop it (de/fr/nl already read it via their dictionaries, so
            // they are left out to keep e.g. French "plus ou moins").
            '±' => match lang {
                "es" => Some("más menos"),
                "it" => Some("più meno"),
                "pt" => Some("mais menos"),
                "pl" | "cs" | "ro" | "sv" | "da" => Some("plus minus"),
                "nb" => Some("pluss minus"),
                "tr" => Some("artı eksi"),
                "ru" | "bg" => Some("плюс минус"),
                "uk" => Some("плюс мінус"),
                _ => None,
            },
            // The `%` char-name entry is missing in several Cyrillic voices, so
            // the dict fallback spells it as garbage ("f"); read the word.
            '%' => match lang {
                "ru" | "bg" => Some("процент"),
                "uk" => Some("відсоток"),
                _ => None,
            },
            // The № (numero) sign is very common in Cyrillic text ("дом №3");
            // without a reading it is dropped ("№5" → "five").  English reads
            // "number" in the branch above.
            // The numero sign is read as each language's own word, but only
            // where espeak-ng defines one; fr/es/uk/pl/sv/nb/da/tr drop it (the
            // bare number is spoken).  Matches the C oracle.
            '№' => match lang {
                "ru" | "bg" => Some("номер"),
                "de" | "it" => Some("numero"),
                "pt" => Some("número"),
                "nl" => Some("nummer"),
                "cs" => Some("číslo"),
                _ => None,
            },
            _ => None,
        }
    };
    if let Some(w) = word {
        // Some readings are multi-word ("divided by"); translate each and
        // join with a word boundary so both words are spoken.
        let mut out: Vec<u8> = Vec::new();
        for part in w.split_whitespace() {
            let wr = word_to_phonemes(part, dict, phdata, stress_opts, options);
            let ph: &[u8] = wr.phonemes.strip_suffix(&[0]).unwrap_or(&wr.phonemes);
            if ph.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push(crate::phoneme::PHON_END_WORD);
            }
            out.extend_from_slice(ph);
        }
        if !out.is_empty() {
            return out;
        }
    }
    // Otherwise use the dictionary character-name entry (`_%`, `_$`, …).
    for key in [format!("_{c}"), c.to_string()] {
        let wr = word_to_phonemes(&key, dict, phdata, stress_opts, options);
        if !wr.phonemes.is_empty() {
            return wr.phonemes;
        }
    }
    Vec::new()
}

pub(crate) fn build_translate_entries(
    tokens: &[Token],
    dict: &Dictionary,
    phdata: &PhonemeData,
    stress_opts: &StressOpts,
    options: &LangOptions,
) -> Vec<TranslateEntry> {
    let mut entries: Vec<TranslateEntry> = Vec::with_capacity(tokens.len());
    // Part-of-speech homograph selection: a verb-triggering function word ("to",
    // a modal) makes the next word take its `$verb` pronunciation ("to lˈɪv" not
    // "lˈaɪv"); a perfect/passive auxiliary ("had"/"have"/"was") selects `$past`
    // ("have rˈɛd" not "rˈiːd").  The dictionary's default is already the
    // noun/adjective form, so only verb/past need triggering.  `prev_word` is the
    // previous *word* (carried across spaces, reset at a clause boundary).
    let mut prev_word: Option<String> = None;
    for token in tokens {
        match token {
            Token::Word(word) => {
                // A word in a script the voice can't read (Cyrillic/Greek inside a
                // Latin clause) switches to a voice for that script — the same
                // `phonSWITCH` re-translation used for `_^_LL` dict entries — so it
                // is spoken instead of dropped.
                if let Some(target) = foreign_script_voice(word, &options.lang) {
                    let mut phonemes = vec![crate::phoneme::PHON_SWITCH];
                    phonemes.extend_from_slice(target.as_bytes());
                    entries.push(TranslateEntry {
                        phonemes,
                        dict_flags: 0,
                        kind:       TranslateEntryKind::Word, // retagged to LangSwitch below
                        word_lower: Some(word.clone()),
                    });
                    prev_word = None;
                    continue;
                }
                let lower = word.to_lowercase();
                let expect = pos_expect_after(prev_word.as_deref());
                let mut wr = word_to_phonemes_pos(&lower, dict, phdata, stress_opts, options, expect);
                prev_word = Some(lower.clone());
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
                entries.push(TranslateEntry {
                    phonemes:   wr.phonemes,
                    dict_flags: wr.dict_flags,
                    kind:       TranslateEntryKind::Word,
                    word_lower: Some(lower),
                });
            }
            Token::Number(token) => {
                // Portuguese numbers: the pt dict lacks number keys, so build the
                // number word(s) from the value and pronounce them via the rules.
                if options.number_grammar.portuguese_cardinals {
                    if let Some(word) = portuguese_number_word(token) {
                        let mut phonemes: Vec<u8> = Vec::new();
                        for part in word.split_whitespace() {
                            let wr = word_to_phonemes(part, dict, phdata, stress_opts, options);
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
                            kind:       TranslateEntryKind::Word,
                            word_lower: Some(word),
                        });
                        continue;
                    }
                }
                // Italian irregular ordinals 1–10 (1º=primo … 10º=decimo): the it
                // dict lacks them, so the regular `-esimo` fallback produces wrong
                // words ("unesimo"); generate the correct form (11+ is left to the
                // regular path, which is right).
                if options.number_grammar.ordinals.italian {
                    if let NumberToken::Ordinal(ord) = token {
                        if let OrdinalMarker::Suffix(suffix) = &ord.marker {
                            if let Some(word) = italian_ordinal_word(&ord.digits, suffix) {
                                let wr = word_to_phonemes(&word, dict, phdata, stress_opts, options);
                                entries.push(TranslateEntry {
                                    phonemes:   wr.phonemes,
                                    dict_flags: wr.dict_flags,
                                    kind:       TranslateEntryKind::Word,
                                    word_lower: Some(word),
                                });
                                continue;
                            }
                        }
                    }
                }
                // French letter-suffix ordinals (1er, 2e, 21e…): the fr dict has
                // no `_No` ordinal words, so build the ordinal word from the value
                // and let the French rules pronounce it ("premier"/"deuxième").
                if options.number_grammar.ordinals.french {
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
                                for part in word
                                    .split(|c: char| c.is_whitespace() || c == '-')
                                    .filter(|p| !p.is_empty())
                                {
                                    let wr = word_to_phonemes(part, dict, phdata, stress_opts, options);
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
                                    kind:       TranslateEntryKind::Word,
                                    word_lower: Some(word),
                                });
                                continue;
                            }
                            // Beyond the generator's range (≥1001): the fr dict has
                            // no ordinal words, so the ordinal path is silent — read
                            // the plain cardinal so the number isn't dropped.
                            let card = NumberToken::Cardinal(ord.digits.clone());
                            if let Some(wr) = translate_number_token(
                                &card, dict, phdata, stress_opts, &options.number_grammar,
                            ) {
                                entries.push(TranslateEntry {
                                    phonemes:   wr.phonemes,
                                    dict_flags: wr.dict_flags,
                                    kind:       TranslateEntryKind::Word,
                                    word_lower: Some(ord.digits.clone()),
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
                entries.push(TranslateEntry {
                    phonemes:   wr.phonemes,
                    dict_flags: wr.dict_flags,
                    kind:       TranslateEntryKind::Word,
                    word_lower: Some(token.surface().to_string().to_lowercase()),
                });
            }
            Token::InlinePhonemes(content) => {
                entries.push(TranslateEntry {
                    phonemes:   parse_inline_phonemes(content, phdata),
                    dict_flags: 0,
                    kind:       TranslateEntryKind::Word,
                    word_lower: None,
                });
            }
            Token::ClauseBoundary(c) => {
                // A clause boundary resets the part-of-speech expectation.
                prev_word = None;
                // `--punct`: announce clause punctuation by name instead of a
                // silent break.  Falls back to a break if the char has no name.
                let name = if punct_covers(options, *c) {
                    lookup_symbol_name(*c, dict, phdata, stress_opts, options)
                } else {
                    Vec::new()
                };
                if name.is_empty() {
                    entries.push(TranslateEntry {
                        phonemes:   Vec::new(),
                        dict_flags: 0,
                        kind:       TranslateEntryKind::ClauseBoundary,
                        word_lower: None,
                    });
                } else {
                    entries.push(TranslateEntry {
                        phonemes:   name,
                        dict_flags: 0,
                        kind:       TranslateEntryKind::Word,
                        word_lower: None,
                    });
                }
            }
            Token::Punctuation(c) if is_spoken_symbol(*c) || punct_covers(options, *c) => {
                // Technical symbols read as their name by default (# $ % & * + / = @ ~).
                let phonemes = lookup_symbol_name(*c, dict, phdata, stress_opts, options);
                if phonemes.is_empty() {
                    entries.push(TranslateEntry {
                        phonemes, dict_flags: 0,
                        kind: TranslateEntryKind::Other, word_lower: None,
                    });
                } else {
                    entries.push(TranslateEntry {
                        phonemes, dict_flags: 0,
                        kind: TranslateEntryKind::Word, word_lower: None,
                    });
                }
            }
            _ => {
                entries.push(TranslateEntry {
                    phonemes:   Vec::new(),
                    dict_flags: 0,
                    kind:       TranslateEntryKind::Other,
                    word_lower: None,
                });
            }
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

pub(crate) fn promote_translate_entries(
    entries: &mut [TranslateEntry],
    phdata: &PhonemeData,
    lang: &str,
) {
    const FLAG_STREND: u32 = 1 << 9;
    const FLAG_STREND2: u32 = 1 << 10;
    const PHON_STRESS_P_CODE: u8 = 6;
    const PHON_STRESS_P2_CODE: u8 = 7;

    fn promote_clause(entries: &mut [TranslateEntry], phdata: &PhonemeData, lang: &str) {
        let n = entries.len();
        #[allow(clippy::needless_range_loop)]
        for i in 0..n {
            if entries[i].kind != TranslateEntryKind::Word {
                continue;
            }
            let dict_flags = entries[i].dict_flags;
            if dict_flags & (FLAG_STREND | FLAG_STREND2) == 0 {
                continue;
            }

            let is_last_word = entries[i + 1..]
                .iter()
                .all(|e| e.kind != TranslateEntryKind::Word);
            let following_all_unstressed = entries[i + 1..]
                .iter()
                .filter(|e| e.kind == TranslateEntryKind::Word)
                .all(|e| {
                    !e.phonemes
                        .iter()
                        .any(|&c| c == PHON_STRESS_P_CODE || c == PHON_STRESS_P2_CODE)
                });

            promote_strend_stress(
                &mut entries[i].phonemes,
                phdata,
                dict_flags,
                is_last_word,
                following_all_unstressed,
            );
        }

        let has_primary = entries
            .iter()
            .filter(|e| e.kind == TranslateEntryKind::Word)
            .any(|e| {
                e.phonemes
                    .iter()
                    .any(|&c| c == PHON_STRESS_P_CODE || c == PHON_STRESS_P2_CODE)
            });

        if !has_primary {
            let last_secondary = entries
                .iter()
                .enumerate()
                .rev()
                .find(|(_, e)| {
                    e.kind == TranslateEntryKind::Word
                        && !e.phonemes.is_empty()
                        && e.phonemes.iter().any(|&c| c == 4 || c == 5)
                })
                .map(|(i, _)| i);

            if let Some(idx) = last_secondary {
                change_word_stress(&mut entries[idx].phonemes, phdata, 4);
            } else {
                let last_word = entries
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, e)| {
                        e.kind == TranslateEntryKind::Word && !e.phonemes.is_empty()
                    })
                    .map(|(i, _)| i);
                if let Some(idx) = last_word {
                    change_word_stress(&mut entries[idx].phonemes, phdata, 4);
                }
            }
        }

        if primary_bcp47_subtag(lang) == "en" {
            apply_en_wh_clause_initial_secondary(entries, phdata);
        }
    }

    fn apply_en_wh_clause_initial_secondary(
        entries: &mut [TranslateEntry],
        phdata: &PhonemeData,
    ) {
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
        let i0 = word_ix[0];
        let i1 = word_ix[1];
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
        if ph[..fv].iter().any(|&c| matches!(c, 4 | 5 | 6 | 7)) {
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
            let slice_end = if bound < entries.len() { bound } else { entries.len() };
            if slice_end > prev_end {
                promote_clause(&mut entries[prev_end..slice_end], phdata, lang_for_promo);
            }
            prev_end = if bound < entries.len() { bound + 1 } else { entries.len() };
        }
    }
}

/// English “linking ɹ” after **or** before a vowel-initial word (same condition as IPA post-pass).
pub(crate) fn apply_en_or_linking_r(entries: &mut [TranslateEntry], phdata: &PhonemeData) {
    let r_code = phdata.lookup_phoneme("r");
    if r_code == 0 {
        return;
    }

    fn next_word_vowel_initial(entries: &[TranslateEntry], from: usize, phdata: &PhonemeData) -> bool {
        entries[from + 1..]
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
        Ok(Translator { options, data_dir: dir })
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
        self.text_to_ipa_with_options(text, false, false, false, true)
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
                        &seg_text, preserve_punctuation, flatten_clauses, false, allow_switch,
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
        let lang = &self.options.lang;
        let dict_stem = resolve_dict_stem(&self.data_dir, lang).ok_or_else(|| {
            Error::NotImplemented("text_to_ipa: dict not found")
        })?;
        let dict_path = self.data_dir.join(format!("{dict_stem}_dict"));
        let phontab_path = self.data_dir.join("phontab");

        // Load dictionary
        let dict_bytes = std::fs::read(&dict_path)
            .map_err(Error::Io)?;
        let dict = Dictionary::from_bytes(&dict_stem, dict_bytes)?;

        // Load phoneme data
        if !phontab_path.exists() {
            return Err(Error::NotImplemented("text_to_ipa: phontab not found"));
        }
        let mut phdata = PhonemeData::load(&self.data_dir)?;
        // The table is usually named after the language; a few voices redirect
        // it (`phonemes <name>`, e.g. Norwegian Bokmål `nb` → `no`).
        select_phoneme_table(&mut phdata, &self.data_dir, lang)?;

        // Build stress options for this language
        let stress_opts = StressOpts::for_lang(lang);

        // Tokenize
        let mut tokens = tokenize_opts(text, &self.options.number_grammar);
        apply_roman_numerals(&mut tokens, lang);
        suppress_abbreviation_periods(&mut tokens, lang);
        apply_time_reading(&mut tokens, lang);
        apply_dash_reading(&mut tokens, lang);
        apply_decades(&mut tokens, lang);
        apply_units(&mut tokens, lang);
        apply_temperature(&mut tokens, lang);
        apply_abbreviations(&mut tokens, lang);
        apply_dimensions(&mut tokens, lang);
        apply_number_abbrev(&mut tokens, lang);
        apply_space_grouping(&mut tokens, &self.options.number_grammar);
        apply_currency(&mut tokens, lang);
        apply_fractions(&mut tokens, lang);
        apply_exponents(&mut tokens, lang);

        let mut entries = build_translate_entries(
            &tokens,
            &dict,
            &phdata,
            &stress_opts,
            &self.options,
        );
        promote_translate_entries(&mut entries, &phdata, lang);

        // Render to IPA with clause boundary newlines
        let mut ipa_out = String::new();
        let mut first_word = true;
        let mut clause_has_output = false;
        let mut stress = PendingStress::None;
        let mut pending_clause_break = false;
        let mut suppress_next_word_space = false;
        let mut straight_quote_open = false;
        // Cache of translators for `phonSWITCH` target languages.
        let mut switch_cache: std::collections::HashMap<String, Translator> =
            std::collections::HashMap::new();

        // `-k 2`: pre-render the "capital" announcement (dict `_cap`) that is
        // spoken before each capitalised word.
        let cap_ipa: Option<String> = if self.options.capitals == 2 {
            let cap = word_to_phonemes("_cap", &dict, &phdata, &stress_opts, &self.options);
            if cap.phonemes.is_empty() {
                None
            } else {
                let use_en = primary_bcp47_subtag(lang) == "en";
                let (ipa, _) = phonemes_to_ipa_full(
                    &cap.phonemes, &phdata, PendingStress::None, false, use_en,
                    LiaisonCtx::default(),
                );
                (!ipa.is_empty()).then_some(ipa)
            }
        } else {
            None
        };

        for (ei, (token, entry)) in tokens.iter().zip(entries.iter()).enumerate() {
            if pending_clause_break
                && matches!(entry.kind, TranslateEntryKind::Word | TranslateEntryKind::LangSwitch)
            {
                ipa_out.push('\n');
                pending_clause_break = false;
            }

            match entry.kind {
                TranslateEntryKind::LangSwitch => {
                    // phonSWITCH: `[phonSWITCH, <lang bytes…>]` — re-translate the
                    // word with the target language and wrap it `(target)…(orig)`.
                    if !allow_switch { continue; }
                    let phonemes = &entry.phonemes;
                    let mut target: String = phonemes[1..]
                        .iter()
                        .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'-')
                        .map(|&b| (b as char).to_ascii_lowercase())
                        .collect();
                    // A bare `phonSWITCH` with no language name switches to the
                    // default voice; the dictionary compiler omits the name when
                    // it equals `ESPEAKNG_DEFAULT_VOICE` ("en").
                    if target.is_empty() {
                        target = "en".to_string();
                    }
                    let word = entry.word_lower.as_deref().unwrap_or("");
                    if !target.is_empty() && !word.is_empty() {
                        if !switch_cache.contains_key(&target) {
                            if let Ok(tr) = Translator::new(&target, Some(&self.data_dir)) {
                                switch_cache.insert(target.clone(), tr);
                            }
                        }
                        if let Some(sw_ipa) = switch_cache.get(&target).and_then(|tr| {
                            tr.text_to_ipa_with_options(word, false, false, false, false).ok()
                        }) {
                            if !first_word && !suppress_next_word_space {
                                ipa_out.push(' ');
                            }
                            suppress_next_word_space = false;
                            ipa_out.push_str(&format!("({target}){sw_ipa}({})", self.options.lang));
                            first_word = false;
                            clause_has_output = true;
                            stress = PendingStress::None;
                        }
                    }
                }
                TranslateEntryKind::Word => {
                    let phonemes = &entry.phonemes;
                    if phonemes.is_empty() { continue; }

                    // `-k 2`: announce "capital" before a capitalised word.
                    if let Some(cap) = &cap_ipa {
                        if matches!(token, Token::Word(w)
                            if w.chars().next().is_some_and(char::is_uppercase))
                        {
                            if !first_word && !suppress_next_word_space {
                                ipa_out.push(' ');
                            }
                            ipa_out.push_str(cap);
                            suppress_next_word_space = false;
                            first_word = false;
                            clause_has_output = true;
                            stress = PendingStress::None;
                        }
                    }

                    let use_en_overrides = primary_bcp47_subtag(lang) == "en";
                    // Look at what follows this word within its clause: the first
                    // spoken Word, or a ClauseBoundary (= a pause), whichever comes
                    // first.  This decides liaison at the end of the word.
                    let next_sig = entries[ei+1..].iter().find(|e| {
                        e.kind == TranslateEntryKind::ClauseBoundary
                            || (e.kind == TranslateEntryKind::Word && !e.phonemes.is_empty())
                    });
                    let next_starts_vowel = matches!(next_sig, Some(e)
                        if e.kind == TranslateEntryKind::Word
                            && e.phonemes.iter()
                                .find(|&&c| c > 8 && c != 15)
                                .and_then(|&c| phdata.get(c))
                                .is_some_and(|ph| ph.typ == 2)); // phVOWEL
                    // A pause follows if the next significant entry is a clause
                    // boundary, or there is nothing more (end of text).
                    let next_is_pause = !matches!(next_sig, Some(e)
                        if e.kind == TranslateEntryKind::Word);
                    // French liaison: resolve liaison phonemes by the following
                    // sound (kept before a vowel, dropped before a consonant),
                    // both within a composed token (numbers like "200" = deux·cent)
                    // and across the boundary to the next token.  A `3`-liaison
                    // (six/dix/huit) takes its citation form before a pause.
                    // French-only: other languages use "X2"/"X3" for real consonants
                    // (Hungarian "öt" = ö + "t2"), which must not be dropped.
                    let liaison = LiaisonCtx {
                        resolve: primary_bcp47_subtag(lang) == "fr",
                        next_starts_vowel,
                        next_is_pause,
                    };
                    let (mut word_ipa, new_stress) = phonemes_to_ipa_full(
                        phonemes,
                        &phdata,
                        stress,
                        !first_word && !suppress_next_word_space,
                        use_en_overrides,
                        liaison,
                    );
                    suppress_next_word_space = false;
                    stress = new_stress;
                    if use_en_overrides
                        && entry.word_lower.as_deref() == Some("or")
                        && next_starts_vowel
                        && word_ipa.ends_with("ɔː")
                        && !word_ipa.ends_with("ɔːɹ")
                    {
                        word_ipa.push('ɹ');
                    }
                    if !word_ipa.is_empty() {
                        ipa_out.push_str(&word_ipa);
                        first_word = false;
                        clause_has_output = true;
                    }
                }
                TranslateEntryKind::ClauseBoundary => {
                    if preserve_punctuation {
                        if let Token::ClauseBoundary(punctuation) = token {
                            // Never print the SSML break sentinel.
                            if matches!(*punctuation, '.' | ',' | '!' | '?' | ';' | ':') {
                                ipa_out.push(*punctuation);
                            }
                        }
                    }
                    if clause_has_output {
                        pending_clause_break = true;
                        clause_has_output = false;
                        stress = PendingStress::None;
                    }
                    // Reset word spacing for new clause
                    first_word = true;
                }
                TranslateEntryKind::Other => {
                    if !preserve_punctuation {
                        continue;
                    }
                    let Token::Punctuation(punctuation) = token else {
                        continue;
                    };

                    let is_opening = match punctuation {
                        '(' | '[' | '{' | '“' | '‘' => true,
                        '"' => !straight_quote_open,
                        _ => false,
                    };
                    if *punctuation == '"' {
                        straight_quote_open = !straight_quote_open;
                    }

                    if is_opening {
                        if pending_clause_break {
                            ipa_out.push('\n');
                            pending_clause_break = false;
                        } else if !ipa_out.is_empty()
                            && !ipa_out.ends_with([' ', '\n'])
                        {
                            ipa_out.push(' ');
                        }
                        suppress_next_word_space = true;
                    }
                    ipa_out.push(*punctuation);
                }
            }
        }

        let mut ipa_out = ipa_out.trim_end_matches('\n').to_string();
        if flatten_clauses {
            ipa_out = ipa_out.replace('\n', " ");
        }
        if primary_bcp47_subtag(lang) == "fr" {
            ipa_out = ipa_out.replace('r', "ʁ");
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
        let lang = &self.options.lang;
        let dict_stem = resolve_dict_stem(&self.data_dir, lang).ok_or_else(|| {
            Error::NotImplemented("translate_to_codes: dict not found")
        })?;
        let dict_path = self.data_dir.join(format!("{dict_stem}_dict"));
        let phontab_path = self.data_dir.join("phontab");

        let dict_bytes = std::fs::read(&dict_path).map_err(Error::Io)?;
        let dict = Dictionary::from_bytes(&dict_stem, dict_bytes)?;

        if !phontab_path.exists() {
            return Err(Error::NotImplemented("translate_to_codes: phontab not found"));
        }
        let mut phdata = PhonemeData::load(&self.data_dir)?;
        // The table is usually named after the language; a few voices redirect
        // it (`phonemes <name>`, e.g. Norwegian Bokmål `nb` → `no`).
        select_phoneme_table(&mut phdata, &self.data_dir, lang)?;
        let stress_opts = StressOpts::for_lang(lang);

        // Capital-letter indication before a capitalised word (C `option_capitals`):
        //   `-k 1` → the `phonCAPITAL` *sound* (short pause + capital phoneme),
        //   `-k 2` → the spoken "capital" announcement (dict `_cap`).
        // Empty when disabled.
        let cap_codes: Vec<u8> = match self.options.capitals {
            1 => vec![crate::phoneme::PHON_PAUSE_SHORT, crate::phoneme::PHON_CAPITAL],
            2 => word_to_phonemes("_cap", &dict, &phdata, &stress_opts, &self.options).phonemes,
            _ => Vec::new(),
        };

        let mut tokens = tokenize_opts(text, &self.options.number_grammar);
        apply_roman_numerals(&mut tokens, lang);
        suppress_abbreviation_periods(&mut tokens, lang);
        apply_time_reading(&mut tokens, lang);
        apply_dash_reading(&mut tokens, lang);
        apply_decades(&mut tokens, lang);
        apply_units(&mut tokens, lang);
        apply_temperature(&mut tokens, lang);
        apply_abbreviations(&mut tokens, lang);
        apply_dimensions(&mut tokens, lang);
        apply_number_abbrev(&mut tokens, lang);
        apply_space_grouping(&mut tokens, &self.options.number_grammar);
        apply_currency(&mut tokens, lang);
        apply_fractions(&mut tokens, lang);
        apply_exponents(&mut tokens, lang);
        let mut entries = build_translate_entries(
            &tokens,
            &dict,
            &phdata,
            &stress_opts,
            &self.options,
        );
        promote_translate_entries(&mut entries, &phdata, lang);
        if primary_bcp47_subtag(lang) == "en" {
            apply_en_or_linking_r(&mut entries, &phdata);
        }

        debug_assert_eq!(tokens.len(), entries.len());
        let mut codes: Vec<PhonemeCode> = Vec::new();
        for (token, entry) in tokens.iter().zip(entries.iter()) {
            match token {
                // Inline phonemes (`[[…]]`) carry pre-parsed phoneme codes and
                // spoken symbols (`&`→"and", `%`→"percent") carry name phonemes
                // in their entry (kind `Word`); emit them like any other word so
                // the `-x` / synthesis path renders them (parity with the
                // `text_to_ipa` / `--ipa` path).  Non-spoken punctuation has an
                // empty entry, so emitting it is a harmless no-op.
                Token::Word(_)
                | Token::Number(_)
                | Token::InlinePhonemes(_)
                | Token::Punctuation(_) => {
                    // `-k 1`/`-k 2`: capital indication before a capitalised
                    // word.  For `-k 1`, an *all-caps* multi-letter word repeats
                    // the sound (mirrors C: FLAG_ALL_UPPER + IsAlpha(word[1])).
                    if !cap_codes.is_empty() {
                        if let Token::Word(w) = token {
                            if w.chars().next().is_some_and(char::is_uppercase) {
                                let alpha = w.chars().filter(|c| c.is_alphabetic());
                                let all_upper = alpha.clone().count() > 1
                                    && alpha.clone().all(|c| c.is_uppercase());
                                let repeats = if self.options.capitals == 1 && all_upper { 2 } else { 1 };
                                for _ in 0..repeats {
                                    for &b in &cap_codes {
                                        if b != 0 {
                                            codes.push(PhonemeCode {
                                                code: b, is_boundary: false, clause_char: None,
                                            });
                                        }
                                    }
                                }
                                codes.push(PhonemeCode {
                                    code: 15, is_boundary: true, clause_char: None,
                                }); // END_WORD
                            }
                        }
                    }
                    for &b in &entry.phonemes {
                        if b != 0 {
                            codes.push(PhonemeCode {
                                code:         b,
                                is_boundary:  false,
                                clause_char:  None,
                            });
                        }
                    }
                    // `-g` word gap: a pause phoneme (`_`) after each word.
                    if self.options.word_gap > 0 && !entry.phonemes.is_empty() {
                        let pause = phdata.lookup_phoneme("_");
                        if pause != 0 {
                            codes.push(PhonemeCode {
                                code: pause, is_boundary: false, clause_char: None,
                            });
                        }
                    }
                }
                Token::Space => {
                    codes.push(PhonemeCode {
                        code:         15,
                        is_boundary:  true,
                        clause_char:  None,
                    }); // END_WORD
                }
                Token::ClauseBoundary(punct) => {
                    if entry.phonemes.is_empty() {
                        codes.push(PhonemeCode {
                            code:         0,
                            is_boundary:  true,
                            clause_char:  Some(*punct),
                        }); // pause + clause
                    } else {
                        // `--punct`: the boundary became a spoken name.  It
                        // absorbed its surrounding whitespace, so bracket it with
                        // END_WORD markers to keep the name a separate word.
                        codes.push(PhonemeCode { code: 15, is_boundary: true, clause_char: None });
                        for &b in &entry.phonemes {
                            if b != 0 {
                                codes.push(PhonemeCode {
                                    code: b, is_boundary: false, clause_char: None,
                                });
                            }
                        }
                        codes.push(PhonemeCode { code: 15, is_boundary: true, clause_char: None });
                    }
                }
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
    /// When this item ends a clause (`Token::ClauseBoundary`), the punctuation
    /// character (`,`, `.`, etc.).  Used for CLI phoneme output formatting.
    pub clause_char: Option<char>,
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
        assert!(g("en").hundreds.use_conjunction_with_remainder, "en: 'and' before remainder");
        assert_eq!(g("es").tens, TensGrammar::WithConjunction, "es: 'treinta y cuatro'");
        assert!(g("es").hundreds.omit_one_prefix);
        assert!(g("fr").hundreds.omit_one_prefix, "fr: 'cent' not 'un cent'");
        assert_eq!(g("de").tens, TensGrammar::UnitsThenConjunction, "de: 'vier und dreißig'");
        assert!(g("de").ordinals.dot_marks_ordinal, "de: '3.' is ordinal");

        // … and does NOT inherit a neighbour's config.
        assert!(!g("fr").ordinals.dot_marks_ordinal, "fr must not take de's dot-ordinal");
        assert_ne!(g("fr").tens, TensGrammar::UnitsThenConjunction, "fr must not take de's tens order");
        assert_ne!(g("en").tens, TensGrammar::WithConjunction, "en must not take es's tens conjunction");
        // English keeps the "one" before hundred/thousand; de/fr/es/ru omit it.
        assert!(!g("en").hundreds.omit_one_prefix, "en keeps 'one hundred'");
        assert!(!g("en").thousands.omit_one_prefix, "en keeps 'one thousand'");
        assert!(g("de").hundreds.omit_one_prefix && g("de").thousands.omit_one_prefix, "de: 'hundert'/'tausend'");

        // Languages sharing a match arm are intentionally identical in their
        // *number* config.  nl additionally carries the Dutch-IJ tokenizer flag
        // and the Germanic compound-ordinal flag (eenentwintigste), both
        // Dutch-only, so normalise them out before comparing to Maltese.
        assert!(g("nl").ordinals.compound_cardinal_suffix, "nl: compound ordinals suffix the cardinal");
        assert!(!g("mt").ordinals.compound_cardinal_suffix, "mt: does not");
        let mut nl = g("nl");
        nl.dutch_ij = false;
        nl.ordinals.compound_cardinal_suffix = false;
        assert_eq!(nl, g("mt"), "nl and mt share the same number arm (apart from Dutch-only flags)");
        // Danish/Faroese/Slovenian are units-first ("enogtyve"), so they differ
        // from the tens-first Finnish/Estonian they used to share an arm with.
        assert_eq!(g("da").tens, TensGrammar::UnitsThenConjunction, "da: 'enogtyve'");
        assert!(g("da").ordinals.compound_cardinal_suffix, "da: compound ordinals suffix the cardinal");
        assert_eq!(g("fi").tens, TensGrammar::Standard, "fi: tens-first 'kaksikymmentäyksi'");
        assert_ne!(g("da"), g("fi"), "da (units-first) must differ from fi (tens-first)");
        assert_eq!(g("fi"), g("et"), "fi and et still share the tens-first arm");
        // … but distinct from an unrelated arm.
        assert_ne!(g("nl"), g("de"), "nl (own arm) differs from de");
        assert_ne!(g("da"), g("nl"), "da differs from nl's fuller config");

        // Unknown / untabulated languages fall back to the default, never a
        // neighbour's config (what the missing-break bug produced in C).
        assert_eq!(g("zz"), NumberGrammar::default(), "unknown lang → default grammar");
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
    fn comma_groups_thousands_for_english() {
        let en = NumberGrammar::for_lang("en");
        // Well-formed grouping → a single number token with the commas removed.
        assert_eq!(
            tokenize_opts("1,000", &en),
            vec![Token::Number(NumberToken::Cardinal("1000".into()))]
        );
        assert_eq!(
            tokenize_opts("1,234,567", &en),
            vec![Token::Number(NumberToken::Cardinal("1234567".into()))]
        );
        // A decimal after a group still parses.
        assert_eq!(
            tokenize_opts("1,000.5", &en),
            vec![Token::Number(NumberToken::Decimal { integer: "1000".into(), fractional: "5".into() })]
        );
        // Malformed groups (not exactly three digits) keep the comma as a boundary.
        assert!(
            tokenize_opts("1,0000", &en).iter().any(|t| t == &Token::ClauseBoundary(',')),
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
    fn currency_symbol_reorders_and_pluralizes() {
        let en = NumberGrammar::for_lang("en");
        let rewrite = |s: &str, lang: &str| {
            let mut t = tokenize_opts(s, &NumberGrammar::for_lang(lang));
            apply_currency(&mut t, lang);
            t
        };
        // `$5` → "five dollars" (reordered, pluralised).
        assert_eq!(
            rewrite("$5", "en"),
            vec![Token::Number(NumberToken::Cardinal("5".into())), Token::Word("dollars".into())]
        );
        // Exactly one → singular.
        assert_eq!(
            rewrite("$1", "en"),
            vec![Token::Number(NumberToken::Cardinal("1".into())), Token::Word("dollar".into())]
        );
        // Other symbols, optional space.
        assert_eq!(
            rewrite("€ 10", "en"),
            vec![Token::Number(NumberToken::Cardinal("10".into())), Token::Word("euros".into())]
        );
        // Non-English currencies now read the word after the amount (no cents
        // split): German `$5` → "fünf Dollar" tokens, not a bare symbol.
        assert_eq!(
            rewrite("$5", "de"),
            vec![Token::Number(NumberToken::Cardinal("5".into())), Token::Word("dollar".into())]
        );
        // A currency the language has no word for is still left untouched.
        assert!(rewrite("¥5", "de").iter().any(|t| matches!(t, Token::Punctuation('¥'))));
        // A bare symbol with no amount is unchanged.
        assert_eq!(rewrite("$", "en"), vec![Token::Punctuation('$')]);

        // Decimal amounts split into dollars + cents.
        let cents = |s: &str| -> Vec<String> {
            rewrite(s, "en").iter().filter_map(|t| match t {
                Token::Word(w) => Some(w.clone()),
                Token::Number(NumberToken::Cardinal(n)) => Some(n.clone()),
                _ => None,
            }).collect()
        };
        assert_eq!(cents("$5.99"), vec!["5", "dollars", "99", "cents"]);
        assert_eq!(cents("$1.01"), vec!["1", "dollar", "01", "cent"]); // singular cent
        assert_eq!(cents("$1.00"), vec!["1", "dollar"]);              // whole → no cents
        assert_eq!(cents("$0.99"), vec!["99", "cents"]);             // no dollars
        assert_eq!(cents("$5.5"), vec!["5", "dollars", "50", "cents"]); // 0.5 → 50c

        // Symbol *after* the number (continental style) and the cent symbol.
        assert_eq!(
            rewrite("10€", "en"),
            vec![Token::Number(NumberToken::Cardinal("10".into())), Token::Word("euros".into())]
        );
        assert_eq!(cents("5$"), vec!["5", "dollars"]);
        assert_eq!(cents("99¢"), vec!["99", "cents"]);
        assert_eq!(cents("5.99€"), vec!["5", "euros", "99", "cents"]);

        // A scale word after the amount goes *before* the currency word:
        // `$5 million` → "five million dollars" (not "five dollars million").
        assert_eq!(cents("$5 million"), vec!["5", "million", "dollars"]);
        assert_eq!(cents("$1 million"), vec!["1", "million", "dollars"]); // plural
        // A non-scale word after the amount is untouched (normal currency):
        // "$5 each" → "five dollars each" (the word stays after the currency).
        assert_eq!(cents("$5 each"), vec!["5", "dollars", "each"]);
        // A decimal amount + scale is read as-is (not cents-split).
        assert_eq!(
            rewrite("$1.2 billion", "en"),
            vec![
                Token::Number(NumberToken::Decimal { integer: "1".into(), fractional: "2".into() }),
                Token::Word("billion".into()),
                Token::Word("dollars".into()),
            ]
        );
        let _ = en;
    }

    #[test]
    fn numeric_exponent_is_spoken() {
        let mut t = tokenize_opts("2^10", &NumberGrammar::for_lang("en"));
        apply_exponents(&mut t, "en");
        let words: Vec<String> = t.iter().filter_map(|t| match t {
            Token::Word(w) => Some(w.clone()),
            Token::Number(NumberToken::Cardinal(n)) => Some(n.clone()),
            _ => None,
        }).collect();
        assert_eq!(words, vec!["2", "to", "the", "power", "of", "10"]);
        // A non-numeric base (`x^2`) is left alone (no spurious rewrite).
        let mut t2 = tokenize_opts("x^2", &NumberGrammar::for_lang("en"));
        apply_exponents(&mut t2, "en");
        assert!(t2.iter().any(|t| matches!(t, Token::Punctuation('^'))));
    }

    #[test]
    fn simple_fractions_are_spoken() {
        let rewrite = |s: &str, lang: &str| {
            let mut t = tokenize_opts(s, &NumberGrammar::for_lang(lang));
            apply_fractions(&mut t, lang);
            t
        };
        let words = |s: &str| -> Vec<String> {
            rewrite(s, "en").iter().filter_map(|t| match t {
                Token::Word(w) => Some(w.clone()),
                Token::Number(NumberToken::Cardinal(n)) => Some(n.clone()),
                _ => None,
            }).collect()
        };
        assert_eq!(words("1/2"), vec!["1", "half"]);
        assert_eq!(words("3/4"), vec!["3", "quarters"]);
        assert_eq!(words("2/3"), vec!["2", "thirds"]);
        assert_eq!(words("½"), vec!["1", "half"]); // vulgar fraction, normalized then read
        // Mixed numbers: "1½" → "one and one half", "2¾" → "two and three quarters".
        assert_eq!(words("1½"), vec!["1", "and", "1", "half"]);
        assert_eq!(words("2¾"), vec!["2", "and", "3", "quarters"]);
        // A date-like `a/b/c` is not a fraction.
        assert!(rewrite("3/4/2024", "en").iter().any(|t| matches!(t, Token::Punctuation('/'))));
        // A large denominator is left as a slash.
        assert!(rewrite("1/12", "en").iter().any(|t| matches!(t, Token::Punctuation('/'))));
        // Non-English is untouched.
        assert!(rewrite("1/2", "de").iter().any(|t| matches!(t, Token::Punctuation('/'))));
    }

    #[test]
    fn leading_decimal_point_is_a_number() {
        let en = NumberGrammar::for_lang("en");
        assert_eq!(
            tokenize_opts(".5", &en),
            vec![Token::Number(NumberToken::Decimal { integer: "0".into(), fractional: "5".into() })]
        );
        assert_eq!(
            tokenize_opts(".25", &en),
            vec![Token::Number(NumberToken::Decimal { integer: "0".into(), fractional: "25".into() })]
        );
        // A dot NOT followed by a digit is still a clause boundary.
        assert!(
            tokenize_opts("5.", &en).iter().any(|t| matches!(t, Token::ClauseBoundary('.'))),
            "trailing dot must stay a sentence boundary"
        );
        // Works for the language's own decimal separator (German comma).
        let de = NumberGrammar::for_lang("de");
        assert_eq!(
            tokenize_opts(",5", &de),
            vec![Token::Number(NumberToken::Decimal { integer: "0".into(), fractional: "5".into() })]
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
        assert_eq!(tokenize_opts("٥", &g), vec![Token::Number(NumberToken::Cardinal("5".into()))]);
        assert_eq!(tokenize_opts("١٢٣", &g), vec![Token::Number(NumberToken::Cardinal("123".into()))]);

        // Arabic native separators normalise to their Latin equivalents.
        assert_eq!(normalize_number_symbols("١٫٥"), "1.5");   // decimal ٫
        assert_eq!(normalize_number_symbols("١٬٠٠٠"), "1,000"); // thousands ٬
        assert_eq!(normalize_number_symbols("٥٪"), "5%");      // percent ٪

        // Full-width (CJK-compatibility) forms → ASCII.
        assert_eq!(fullwidth_to_ascii('５'), Some('5'));
        assert_eq!(fullwidth_to_ascii('Ａ'), Some('A'));
        assert_eq!(fullwidth_to_ascii('＄'), Some('$'));
        assert_eq!(fullwidth_to_ascii('\u{3000}'), Some(' ')); // ideographic space
        assert_eq!(fullwidth_to_ascii('5'), None);
        assert_eq!(normalize_number_symbols("Ｈｅｌｌｏ"), "Hello");
        assert_eq!(normalize_number_symbols("１２３"), "123");

        // Enclosed digits and Roman-numeral codepoints → their value.
        assert_eq!(enclosed_number_value('①'), Some(1));
        assert_eq!(enclosed_number_value('⑩'), Some(10));
        assert_eq!(enclosed_number_value('Ⅳ'), Some(4));
        assert_eq!(enclosed_number_value('Ⅿ'), Some(1000));
        assert_eq!(enclosed_number_value('5'), None);
        assert!(tokenize_opts("①", &g).iter()
            .any(|t| t == &Token::Number(NumberToken::Cardinal("1".into()))));
    }

    #[test]
    fn unicode_number_symbols_are_normalized() {
        use std::borrow::Cow;
        // Superscript ²/³ read as powers; subscripts stay digits (H₂O).
        assert_eq!(normalize_number_symbols("x²"), "x squared ");
        assert_eq!(normalize_number_symbols("10³"), "10 cubed ");
        assert_eq!(normalize_number_symbols("H₂O"), "H 2O");
        // Vulgar fractions → n/m.
        assert_eq!(normalize_number_symbols("½"), " 1/2 ");
        assert_eq!(normalize_number_symbols("3½"), "3 1/2 ");
        assert_eq!(normalize_number_symbols("⅔"), " 2/3 ");
        // Plain text is returned borrowed (no allocation).
        assert!(matches!(normalize_number_symbols("hello"), Cow::Borrowed("hello")));

        // Stylized (mathematical/circled) letters and digits → plain ASCII.
        assert_eq!(normalize_number_symbols("𝐇𝐞𝐥𝐥𝐨"), "Hello"); // bold
        assert_eq!(normalize_number_symbols("𝓗𝓮𝓵𝓵𝓸"), "Hello"); // bold script
        assert_eq!(normalize_number_symbols("𝔻𝕒𝕥𝕒"), "Data");    // double-struck
        assert_eq!(normalize_number_symbols("Ⓗⓘ"), "Hi");         // circled
        assert_eq!(normalize_number_symbols("𝟏𝟐𝟑"), "123");       // bold digits
        assert_eq!(normalize_number_symbols("ℝℤℕ"), "RZN");        // letterlike holes
        assert_eq!(stylized_to_ascii('𝐀'), Some('A'));
        assert_eq!(stylized_to_ascii('𝐳'), Some('z'));
        assert_eq!(stylized_to_ascii('A'), None); // plain ASCII untouched

        // End-to-end: `x²` reads "x squared"; a subscript `H₂` still yields a
        // number token (not vanishing).
        let en = NumberGrammar::for_lang("en");
        assert!(
            tokenize_opts("x²", &en).iter().any(|t| t == &Token::Word("squared".into())),
            "superscript ² should read 'squared'"
        );
        assert!(
            tokenize_opts("H₂", &en).iter().any(|t| t == &Token::Number(NumberToken::Cardinal("2".into()))),
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
        // Genuine ordinal suffixes still parse as ordinals.
        assert!(matches!(
            tokenize_opts("3rd", &en).as_slice(),
            [Token::Number(NumberToken::Ordinal(o))] if o.digits == "3"
        ));
        assert!(matches!(
            tokenize_opts("21st", &en).as_slice(),
            [Token::Number(NumberToken::Ordinal(o))] if o.digits == "21"
        ));
        // A st/nd/rd/th suffix that does NOT match the number is not an ordinal
        // (upstream #96: "2st" must not become "secst") — the number is a
        // cardinal and the stray letters a separate word.
        for (input, num, letters) in [("2st", "2", "st"), ("1nd", "1", "nd"),
                                      ("3th", "3", "th"), ("11st", "11", "st")] {
            assert_eq!(
                tokenize_opts(input, &en),
                vec![
                    Token::Number(NumberToken::Cardinal(num.into())),
                    Token::Word(letters.into()),
                ],
                "{input} should be cardinal + word",
            );
        }
        // …but the correct suffix for the same tens still parses (11th, 111th).
        assert!(matches!(
            tokenize_opts("11th", &en).as_slice(),
            [Token::Number(NumberToken::Ordinal(o))] if o.digits == "11"
        ));
    }

    #[test]
    fn european_decimal_and_grouping_are_swapped() {
        let de = NumberGrammar::for_lang("de");
        assert_eq!((de.decimal_separator, de.group_separator), (',', Some('.')));
        // German uses `.` to group and `,` for the decimal.
        assert_eq!(
            tokenize_opts("1.000", &de),
            vec![Token::Number(NumberToken::Cardinal("1000".into()))]
        );
        assert_eq!(
            tokenize_opts("3,14", &de),
            vec![Token::Number(NumberToken::Decimal { integer: "3".into(), fractional: "14".into() })]
        );
        assert_eq!(
            tokenize_opts("1.000,5", &de),
            vec![Token::Number(NumberToken::Decimal { integer: "1000".into(), fractional: "5".into() })]
        );

        // English is the mirror image and stays as before.
        let en = NumberGrammar::for_lang("en");
        assert_eq!((en.decimal_separator, en.group_separator), ('.', Some(',')));
        assert_eq!(
            tokenize_opts("3.14", &en),
            vec![Token::Number(NumberToken::Decimal { integer: "3".into(), fractional: "14".into() })]
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
    fn text_to_ipa_en_us_falls_back_to_en_dict() {
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
        assert_eq!(ipa, t_en.text_to_ipa("hello").unwrap());
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
    fn text_to_ipa_english_rule_regressions() {
        run_ipa_table("en", "en_dict", &[
            ("sky", "skˈaɪ"),
            ("caused", "kˈɔːzd"),
            ("reflection", "ɹɪflˈɛkʃən"),
            ("droplets", "dɹˈɒplɪts"),
            ("appearing", "ɐpˈiəɹɪŋ"),
            ("meteorological", "mˌiːtɪˌɔːɹəlˈɒdʒɪkəl"),
        ]);
    }

    #[test]
    fn text_to_ipa_english_sentence_weak_forms() {
        let t = Translator::new_default("en").unwrap();
        if !Path::new("/usr/share/espeak-ng-data/en_dict").exists() { return; }
        let ipa = t.text_to_ipa("A rainbow is a meteorological phenomenon that is caused by reflection, refraction and dispersion of light in water droplets resulting in a spectrum of light appearing in the sky.").unwrap();
        assert_eq!(
            ipa,
            "ɐ ɹˈeɪnbəʊ ɪz ɐ mˌiːtɪˌɔːɹəlˈɒdʒɪkəl fɪnˈɒmɪnən ðat ɪz kˈɔːzd baɪ ɹɪflˈɛkʃən\nɹɪfɹˈakʃən and dɪspˈɜːʃən ɒv lˈaɪt ɪn wˈɔːtə dɹˈɒplɪts ɹɪzˈʌltɪŋ ɪn ɐ spˈɛktɹəm ɒv lˈaɪt ɐpˈiəɹɪŋ ɪnðə skˈaɪ"
        );
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
        // NUM_ORDINAL_DOT mechanism.  A compound ordinal takes the suffix on the
        // whole cardinal ("einundzwanzigste"), not tens-ord + units-ord (which
        // gave the wrong two-word "zwanzig erste").
        run_ipa_table("de", "de_dict", &[
            ("1.",  "ˈeːrstə"),
            ("3.",  "drˈɪtə"),
            ("20.", "tsvˈantsɪɡʰtə"),
            // "einundzwanzigste" — the unit "1" apocopates to "ein" (not "eins").
            ("21.", "ˌaɪn ʊnttsvˈantsɪɡʰtə"),
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
             "œ̃ miljɔ̃ døsɑ̃ tʁɑ̃tkatʁ mil sɛ̃ksɑ̃ swasɑ̃tsˈɛt",
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

    #[test]
    fn french_cardinal_text_vigesimal() {
        // The vigesimal speller used to build ordinals (data-free).
        let cases = [
            (0, "zéro"), (7, "sept"), (16, "seize"), (17, "dix-sept"),
            (20, "vingt"), (21, "vingt et un"), (22, "vingt-deux"),
            (31, "trente et un"), (60, "soixante"), (61, "soixante et un"),
            (70, "soixante-dix"), (71, "soixante et onze"), (77, "soixante-dix-sept"),
            (80, "quatre-vingts"), (81, "quatre-vingt-un"), (90, "quatre-vingt-dix"),
            (91, "quatre-vingt-onze"), (99, "quatre-vingt-dix-neuf"),
            (100, "cent"), (101, "cent un"), (200, "deux cents"),
            (234, "deux cent trente-quatre"), (999, "neuf cent quatre-vingt-dix-neuf"),
        ];
        for (n, want) in cases {
            assert_eq!(fr_cardinal_text(n), want, "fr_cardinal_text({n})");
        }
    }

    #[test]
    fn french_ordinal_word_generation() {
        // digits + written suffix → ordinal word (data-free).
        let cases = [
            ("1", "er", "premier"), ("1", "re", "première"), ("1", "ère", "première"),
            ("2", "e", "deuxième"), ("2", "nd", "second"), ("2", "nde", "seconde"),
            ("3", "e", "troisième"), ("4", "e", "quatrième"), ("5", "e", "cinquième"),
            ("9", "e", "neuvième"), ("11", "e", "onzième"), ("17", "e", "dix-septième"),
            ("20", "e", "vingtième"), ("21", "e", "vingt et unième"),
            ("22", "e", "vingt-deuxième"), ("70", "e", "soixante-dixième"),
            ("71", "e", "soixante et onzième"), ("80", "e", "quatre-vingtième"),
            ("81", "e", "quatre-vingt-unième"), ("90", "e", "quatre-vingt-dixième"),
            ("99", "e", "quatre-vingt-dix-neuvième"), ("100", "e", "centième"),
            ("200", "e", "deux centième"), ("234", "e", "deux cent trente-quatrième"),
            ("1000", "e", "millième"),
        ];
        for (d, suf, want) in cases {
            assert_eq!(french_ordinal_word(d, suf).as_deref(), Some(want), "{d}{suf}");
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
            ("0", "zero"), ("1", "um"), ("3", "três"), ("5", "cinco"),
            ("15", "quinze"), ("16", "dezasseis"), ("21", "vinte e um"),
            ("100", "cem"), ("101", "cento e um"), ("123", "cento e vinte e três"),
            ("200", "duzentos"), ("234", "duzentos e trinta e quatro"),
            ("999", "novecentos e noventa e nove"),
            ("1000", "mil"), ("1001", "mil e um"), ("1100", "mil e cem"),
            ("1120", "mil cento e vinte"),        // no "e" (120 not <100/round)
            ("1500", "mil e quinhentos"),         // "e" (round hundred)
            ("2000", "dois mil"), ("2020", "dois mil e vinte"),
            ("2234", "dois mil duzentos e trinta e quatro"),
            ("100000", "cem mil"),
            ("1000000", "um milhão"), ("2000000", "dois milhões"),
            ("1500000", "um milhão e quinhentos mil"),
            ("2000001", "dois milhões e um"),
        ];
        for (digits, want) in cases {
            assert_eq!(portuguese_cardinal_word(digits).as_deref(), Some(want), "pt {digits}");
        }
        // Above the covered range → None (caller keeps the existing path).
        assert_eq!(portuguese_cardinal_word("1000000000"), None);
    }

    #[test]
    fn comma_decimal_languages_are_configured() {
        // These use a comma decimal (period/space thousands); without it, "3,14"
        // read as "three [pause] fourteen".
        for lang in ["sv", "pl", "cs", "ro", "el", "lv", "is", "ca", "eu", "sq", "af",
                     "bs", "id", "vi", "az", "ka", "hy", "kk"] {
            let g = NumberGrammar::for_lang(lang);
            assert_eq!(g.decimal_separator, ',', "{lang} decimal");
            assert_eq!(g.group_separator, Some('.'), "{lang} group");
            assert!(g.space_group, "{lang} should allow space grouping too");
        }
        // Period-decimal languages are unchanged.
        assert_eq!(NumberGrammar::for_lang("en").decimal_separator, '.');
        assert_eq!(NumberGrammar::for_lang("en").group_separator, Some(','));
    }

    #[test]
    fn roman_numeral_parsing() {
        // Canonical Roman numerals parse; non-canonical / non-Roman reject.
        for (s, n) in [("I", 1), ("IV", 4), ("IX", 9), ("XIV", 14), ("XL", 40),
                       ("XCIX", 99), ("MCMLXXXIV", 1984), ("MMXXIV", 2024)] {
            assert_eq!(roman_value(s), Some(n), "{s}");
            assert_eq!(to_roman(n), s, "to_roman({n})");
        }
        // Real words / malformed numerals must NOT parse (round-trip guard).
        for s in ["IIII", "VX", "IC", "MIX", "DID", "MIMIC", "CIVIC", "", "A", "mix"] {
            // "MIX" *is* canonical (1009) but only converts after a keyword; the
            // parser itself accepts it — the guard is `to_roman` round-tripping.
            if s == "MIX" {
                assert_eq!(roman_value(s), Some(1009));
            } else {
                assert_eq!(roman_value(s), None, "{s} should not be a valid numeral");
            }
        }
    }
}
