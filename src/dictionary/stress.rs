//! Stress assignment: `GetVowelStress` + `SetWordStress`.
//!
//! Port of `GetVowelStress()` (dictionary.c:804) and
//! `SetWordStress()` (dictionary.c:921).
//!
//! These functions assign stress markers to phoneme sequences produced by
//! dictionary lookup or rule translation.

use crate::phoneme::load::PhonemeData;

// ──────────────────────────────────────────────────────────────────────────────
// Constants — mirror synthesize.h / phoneme.h
// ──────────────────────────────────────────────────────────────────────────────

pub const N_WORD_PHONEMES: usize = 200;

// Stress level values (signed byte in C, i8 here)
const STRESS_IS_DIMINISHED:   i8  = 0;
const STRESS_IS_UNSTRESSED:   i8  = 1;
const STRESS_IS_NOT_STRESSED: i8  = 2;
const STRESS_IS_SECONDARY:    i8  = 3;
const STRESS_IS_PRIMARY:      i8  = 4;
const STRESS_IS_PRIORITY:     i8  = 5;

// Phoneme type codes (phoneme.h)
const PH_PAUSE:   u8 = 0;
const PH_STRESS:  u8 = 1;
const PH_VOWEL:   u8 = 2;

// Phoneme flag bits (phoneme.h)
const PH_UNSTRESSED:   u32 = 1 << 1;   // phFLAGBIT_UNSTRESSED = 1
const PH_NONSYLLABIC:  u32 = 1 << 20;  // phFLAGBIT_NONSYLLABIC = 20
const PH_LONG:         u32 = 1 << 21;  // phFLAGBIT_LONG = 21

// Special phoneme codes (phoneme.h)
const PHON_STRESS_D:    u8 = 3;   // %% diminished
const PHON_STRESS_U:    u8 = 2;   // %  unstressed
const PHON_STRESS_2:    u8 = 4;   // ,  secondary
const PHON_STRESS_3:    u8 = 5;   // ,, secondary2
const PHON_STRESS_P:    u8 = 6;   // '  primary
const PHON_STRESS_P2:   u8 = 7;   // '' priority primary
const PHON_STRESS_PREV: u8 = 8;   // =  stress on preceding vowel
const PHON_STRESS_TONIC:u8 = 26;  // '! tonic
const PHON_LENGTHENED:  u8 = 12;  // :  length mark
// Dead code kept for docs: const PHON_END_WORD:    u8 = 15;  // || end-of-word
const PHON_SYLLABIC:    u8 = 20;  // syllabic consonant marker
// Dead code: const PHON_SCHWA:       u8 = 13;  // @ schwa

// stress_phonemes[] array indexed by stress level 0-6
// { phonSTRESS_D, phonSTRESS_U, phonSTRESS_2, phonSTRESS_3, phonSTRESS_P, phonSTRESS_P2, phonSTRESS_TONIC }
const STRESS_PHONEMES: [u8; 7] = [
    PHON_STRESS_D,    // 0 = DIMINISHED
    PHON_STRESS_U,    // 1 = UNSTRESSED  (not normally emitted)
    PHON_STRESS_2,    // 2 = NOT_STRESSED (secondary in some contexts)
    PHON_STRESS_3,    // 3 = SECONDARY
    PHON_STRESS_P,    // 4 = PRIMARY
    PHON_STRESS_P2,   // 5 = PRIORITY
    PHON_STRESS_TONIC,// 6 = TONIC
];

// Stress rule constants (translate.h)
pub const STRESSPOSN_1L:          u8 = 0;  // 1st syllable (default/trochaic)
pub const STRESSPOSN_2L:          u8 = 1;  // 2nd syllable
pub const STRESSPOSN_2R:          u8 = 2;  // penultimate
pub const STRESSPOSN_1R:          u8 = 3;  // final syllable
pub const STRESSPOSN_3R:          u8 = 4;  // antipenultimate
pub const STRESSPOSN_SYLCOUNT:    u8 = 5;  // Russian-style
pub const STRESSPOSN_1RH:         u8 = 6;  // heaviest (Hindi)
pub const STRESSPOSN_1RU:         u8 = 7;  // Turkish
pub const STRESSPOSN_2LLH:        u8 = 8;  // 1st unless light+heavy follows
pub const STRESSPOSN_ALL:         u8 = 9;  // all stressed
pub const STRESSPOSN_GREENLANDIC: u8 = 12;
pub const STRESSPOSN_1SL:         u8 = 13; // Malay
pub const STRESSPOSN_EU:          u8 = 15; // Basque

// Stress flags (translate.h)
const S_NO_DIM:               u32 = 0x02;
const S_FINAL_DIM:            u32 = 0x04;
const S_FINAL_NO_2:           u32 = 0x10;
const S_NO_AUTO_2:            u32 = 0x20;
const S_2_TO_HEAVY:           u32 = 0x40;
const S_FIRST_PRIMARY:        u32 = 0x80;
const S_FINAL_VOWEL_UNSTRESSED: u32 = 0x100;
const S_FINAL_SPANISH:        u32 = 0x200;
const S_2_SYL_2:              u32 = 0x1000;
const S_INITIAL_2:            u32 = 0x2000;
const S_MID_DIM:              u32 = 0x10000;
// Dead code: const 0x20000:      u32 = 0x20000;
const S_FINAL_LONG:           u32 = 0x80000;

// ──────────────────────────────────────────────────────────────────────────────
// Language options needed by stress computation
// ──────────────────────────────────────────────────────────────────────────────

/// Stress-assignment options, mirroring the relevant `LANGUAGE_OPTIONS` fields
/// from `translate.h`.
///
/// Build a language-appropriate value with [`StressOpts::for_lang`].
#[derive(Debug, Clone)]
pub struct StressOpts {
    /// Which syllable receives primary stress (a `STRESSPOSN_*` constant).
    pub stress_rule:    u8,
    /// Bitmask of `S_*` stress flags.
    pub stress_flags:   u32,
    /// `LOPT_VOWEL_PAUSE` — pause insertion around certain vowels.
    pub vowel_pause:    u32,
    /// Stress level to assign to monosyllabic words (rule 1).
    pub unstressed_wd1: i32,
    /// Stress level to assign to monosyllabic words (rule 2).
    pub unstressed_wd2: i32,
    /// Language name packed as a `u32` (used for language-specific branches).
    pub translator_name: u32,
    /// `LOPT_IT_LENGTHEN` — lengthening parameter (Italian etc.).
    pub opt_length: u8,
    /// True for languages with word-final obstruent devoicing
    /// (German, Dutch, Slovak, …).
    pub word_final_devoicing: bool,
    /// `LOPT_ALT & 2` — when true, the phoneme immediately after a primary
    /// stress marker is upgraded from its "plain" vowel form to its
    /// "stressed" vowel form.  Mirrors `ApplySpecialAttribute2()` in
    /// translateword.c.  Used by Turkish (e→E, o→O under primary stress).
    pub alt_stress_upgrade: bool,
}

impl Default for StressOpts {
    fn default() -> Self {
        StressOpts {
            stress_rule:     STRESSPOSN_1L,
            stress_flags:    0,
            vowel_pause:     0,
            unstressed_wd1:  STRESS_IS_UNSTRESSED as i32,
            unstressed_wd2:  STRESS_IS_NOT_STRESSED as i32,
            translator_name: 0,
            opt_length:      0,
            word_final_devoicing: false,
            alt_stress_upgrade: false,
        }
    }
}

impl StressOpts {
    pub fn for_lang(lang: &str) -> Self {
        let mut opts = StressOpts::default();
        let name = lang_name(lang);
        opts.translator_name = name;

        match lang {
            "en" => {
                opts.stress_rule  = STRESSPOSN_1L;
                opts.stress_flags = 0x08;
            }
            // Languages with LOPT_REGRESSIVE_VOICING & 0x100: word-final devoicing
            "de" | "nl" | "af" | "sk" | "sl" | "sq" => {
                opts.stress_rule  = STRESSPOSN_1L;
                opts.stress_flags = 0;
                opts.word_final_devoicing = true;
            }
            "am" | "az" | "bs" | "bg" | "hr" | "cs"
            | "eo" | "fi" | "hu" | "id" | "ms" | "ka" | "mk" | "nb"
            | "ro" | "sr" | "sw" => {
                opts.stress_rule  = STRESSPOSN_1L;
                opts.stress_flags = 0;
            }
            "fr" => {
                opts.stress_rule  = STRESSPOSN_1R;
                opts.stress_flags = S_NO_AUTO_2 | S_FINAL_DIM;
                opts.opt_length   = 1;
            }
            "es" | "an" | "ca" | "ia" => {
                opts.stress_rule  = STRESSPOSN_2R;
                opts.stress_flags = S_FINAL_DIM | S_FINAL_SPANISH;
                // Note: ca and ia have sub-variants, this is a simplification
            }
            "it" => {
                opts.stress_rule  = STRESSPOSN_2R;
                opts.stress_flags = S_FINAL_DIM | S_NO_AUTO_2;
            }
            "pt" => {
                opts.stress_rule  = STRESSPOSN_2R;
                opts.stress_flags = S_FINAL_DIM | S_FINAL_SPANISH | S_NO_AUTO_2;
            }
            "ru" | "uk" => {
                opts.stress_rule  = STRESSPOSN_SYLCOUNT;
                opts.stress_flags = S_NO_AUTO_2;
            }
            "tr" => {
                opts.stress_rule     = STRESSPOSN_1RU;
                opts.stress_flags    = S_NO_AUTO_2 | S_FINAL_DIM;
                opts.alt_stress_upgrade = true; // LOPT_ALT & 2: upgrade e→E, o→O under primary stress
            }
            "hi" | "ur" => {
                opts.stress_rule  = STRESSPOSN_1RH;
                opts.stress_flags = 0;
            }
            "kl" => {
                opts.stress_rule  = STRESSPOSN_GREENLANDIC;
            }
            "ml" => {
                opts.stress_rule  = STRESSPOSN_1SL;
            }
            "eu" => {
                opts.stress_rule  = STRESSPOSN_EU;
            }
            "el" | "grc" => {
                opts.stress_rule  = STRESSPOSN_2R;
                opts.stress_flags = S_FINAL_DIM | S_NO_AUTO_2;
            }
            "ja" => {
                opts.stress_rule  = STRESSPOSN_1L;
                opts.stress_flags = S_NO_AUTO_2;
            }
            "zh" | "cmn" | "yue" => {
                opts.stress_rule  = STRESSPOSN_1L;
                opts.stress_flags = 0;
            }
            _ => {
                // Default: penultimate
                opts.stress_rule  = STRESSPOSN_2R;
                opts.stress_flags = 0;
            }
        }
        opts
    }
}

fn lang_name(lang: &str) -> u32 {
    let bytes = lang.as_bytes();
    let mut name = 0u32;
    for (_, &b) in bytes.iter().enumerate().take(4) {
        name = (name << 8) | (b as u32);
    }
    name
}

/// Apply word-final devoicing (Auslautverhärtung) for languages with
/// LOPT_REGRESSIVE_VOICING & 0x100.
///
/// Scans the phoneme list from the end and devoices the last consonant if it
/// is a phVSTOP (5) or phVFRICATIVE (7) with a non-zero end_type.
/// Stops if a vowel or pause is encountered before a voiced stop/fricative.
pub fn apply_word_final_devoicing(phonemes: &mut Vec<u8>, phdata: &PhonemeData) {
    // Find last voiced stop/fricative at word end (after all vowels)
    // Scan backwards, skipping null terminator and stress markers
    for i in (0..phonemes.len()).rev() {
        let code = phonemes[i];
        if code == 0 { continue; }
        if let Some(ph) = phdata.get(code) {
            match ph.typ {
                0 | 1 => continue, // pause or stress: skip and keep looking
                2 => return,       // vowel before the consonant: no devoicing
                5 | 7 => {
                    // phVSTOP or phVFRICATIVE: devoice if has unvoiced equivalent
                    if ph.end_type != 0 {
                        phonemes[i] = ph.end_type;
                    }
                    return;
                }
                _ => return, // other consonant at end: no devoicing
            }
        } else {
            // Unknown code (control code like LENGTHEN=12, END_WORD=15, etc.)
            // Skip stress/boundary markers
            if code <= 16 { continue; }
            return; // other unknown: stop
        }
    }
}

/// Apply stressed-vowel upgrading after stress placement.
///
/// Mirrors `ApplySpecialAttribute2()` in translateword.c for `LOPT_ALT & 2`.
///
/// Scans `phonemes` for a primary-stress marker (code 6) and, if found,
/// upgrades the immediately following vowel from its "plain" form to its
/// "stressed" form by looking up the alternative phoneme via mnemonic:
/// - phoneme with mnemonic "e" → replace with phoneme with mnemonic "E"
/// - phoneme with mnemonic "o" → replace with phoneme with mnemonic "O"
///
/// This handles Turkish vowel quality under primary stress (e→ɛ, o→ɔ).
pub fn apply_alt_stress_upgrade(phonemes: &mut Vec<u8>, phdata: &PhonemeData) {
    // Pre-compute codes for the mnemonics we need to swap.
    const STRESS_P: u8 = 6; // phonSTRESS_P
    let code_plain_e  = phdata.lookup_phoneme("e");
    let code_stressed_e = phdata.lookup_phoneme("E");
    let code_plain_o  = phdata.lookup_phoneme("o");
    let code_stressed_o = phdata.lookup_phoneme("O");

    // Both plain and stressed variants must exist in this language's table.
    if code_plain_e == 0 || code_stressed_e == 0
        || code_plain_o == 0 || code_stressed_o == 0
    {
        return;
    }

    for i in 0..phonemes.len().saturating_sub(1) {
        if phonemes[i] == STRESS_P {
            let p = &mut phonemes[i + 1];
            if *p == code_plain_e      { *p = code_stressed_e; }
            else if *p == code_plain_o { *p = code_stressed_o; }
            break; // only the first primary stress matters
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// GetVowelStress
// ──────────────────────────────────────────────────────────────────────────────

/// Port of GetVowelStress() from dictionary.c:804.
///
/// Scans `phonemes`, strips stress-marker phonemes in-place, and builds a
/// per-vowel stress array.
///
/// Returns (max_stress, vowel_count, stressed_syllable):
/// - max_stress: the highest stress level found (-1 if none)
/// - vowel_count: number of vowels found + 1 (boundary sentinels at 0 and count)
/// - stressed_syllable: 0 if none, else the position of explicit primary stress
///
/// `control`: bit 0 = mark phUNSTRESSED vowels as STRESS_IS_UNSTRESSED (usually 1).
pub fn get_vowel_stress(
    phonemes: &mut Vec<u8>,
    phdata: &PhonemeData,
    vowel_stress: &mut [i8; 100],   // N_WORD_PHONEMES/2
    control: i32,
) -> (i32, i32, i32) {
    let mut count = 1i32;
    let mut max_stress: i32 = -1;
    let mut stress: i32 = -1;
    let mut stressed_syllable: i32 = 0;
    let mut primary_posn: i32 = 0;

    vowel_stress[0] = STRESS_IS_UNSTRESSED;

    // Compact the phoneme array, removing stress markers.
    let original = phonemes.clone();
    let mut out_pos = 0usize;
    let mut i = 0usize;
    let _phon_prev_stress = PHON_STRESS_PREV;

    while i < original.len() {
        let phcode = original[i];
        if phcode == 0 { break; }
        i += 1;

        let ph = phdata.get(phcode);

        // Stress marker?
        let is_stress_marker = ph.map(|p| p.typ == PH_STRESS && p.program == 0)
            .unwrap_or(false);

        if is_stress_marker {
            if phcode == PHON_STRESS_PREV {
                // Retroactive primary stress on the PRECEDING vowel
                let mut j = count - 1;
                while j > 0 && stressed_syllable == 0 && (vowel_stress[j as usize] as i8) < STRESS_IS_PRIMARY {
                    let vs = vowel_stress[j as usize];
                    if vs != STRESS_IS_DIMINISHED && vs != STRESS_IS_UNSTRESSED {
                        vowel_stress[j as usize] = STRESS_IS_PRIMARY;
                        if max_stress < STRESS_IS_PRIMARY as i32 {
                            max_stress = STRESS_IS_PRIMARY as i32;
                            primary_posn = j;
                        }
                        // Reduce any preceding primary stresses to secondary
                        for ix in 1..j {
                            if vowel_stress[ix as usize] == STRESS_IS_PRIMARY {
                                vowel_stress[ix as usize] = STRESS_IS_SECONDARY;
                            }
                        }
                        break;
                    }
                    j -= 1;
                }
            } else {
                // Forward stress on following vowel
                let std_len = ph.map(|p| p.std_length).unwrap_or(0) as i32;
                if std_len < 4 || stressed_syllable == 0 {
                    stress = std_len;
                    if stress > max_stress {
                        max_stress = stress;
                    }
                }
            }
            // Don't copy stress markers to output
            continue;
        }

        // Vowel?
        let is_vowel = ph.map(|p| p.typ == PH_VOWEL && p.phflags & PH_NONSYLLABIC == 0)
            .unwrap_or(false);

        let is_syllabic = phcode == PHON_SYLLABIC;

        if is_vowel {
            if count < 99 {
                vowel_stress[count as usize] = stress as i8;

                if stress >= STRESS_IS_PRIMARY as i32 && stress >= max_stress {
                    primary_posn = count;
                    max_stress = stress;
                }

                // phUNSTRESSED: force unstressed if control bit 0 set
                if stress < 0 && (control & 1) != 0 {
                    if ph.map(|p| p.phflags & PH_UNSTRESSED != 0).unwrap_or(false) {
                        vowel_stress[count as usize] = STRESS_IS_UNSTRESSED;
                    }
                }

                count += 1;
                stress = -1;
            }
        } else if is_syllabic {
            if count < 99 {
                vowel_stress[count as usize] = stress as i8;
                if stress < 0 && (control & 1) != 0 {
                    vowel_stress[count as usize] = STRESS_IS_UNSTRESSED;
                }
                count += 1;
            }
        }

        // Copy phoneme to output
        phonemes[out_pos] = phcode;
        out_pos += 1;
    }

    // Null-terminate the compacted phoneme array
    if out_pos < phonemes.len() {
        phonemes[out_pos] = 0;
        phonemes.truncate(out_pos + 1);
    }

    // Terminal boundary
    if count < 100 {
        vowel_stress[count as usize] = STRESS_IS_UNSTRESSED;
    }

    // Explicit stressed syllable from $1/$2 etc (> 0)
    if stressed_syllable > 0 {
        if stressed_syllable >= count {
            stressed_syllable = count - 1;
        }
        vowel_stress[stressed_syllable as usize] = STRESS_IS_PRIMARY;
        max_stress = STRESS_IS_PRIMARY as i32;
        primary_posn = stressed_syllable;
    }

    // Handle PRIORITY stress: replace primary markers
    if max_stress == STRESS_IS_PRIORITY as i32 {
        for ix in 1..count {
            if vowel_stress[ix as usize] == STRESS_IS_PRIMARY {
                // Replace with secondary (or unstressed if 0x20000)
                vowel_stress[ix as usize] = STRESS_IS_SECONDARY;
            }
            if vowel_stress[ix as usize] == STRESS_IS_PRIORITY {
                vowel_stress[ix as usize] = STRESS_IS_PRIMARY;
                primary_posn = ix;
            }
        }
        max_stress = STRESS_IS_PRIMARY as i32;
    }

    stressed_syllable = primary_posn;
    (max_stress, count, stressed_syllable)
}

// ──────────────────────────────────────────────────────────────────────────────
// SetWordStress
// ──────────────────────────────────────────────────────────────────────────────

/// Port of SetWordStress() from dictionary.c:921.
///
/// Modifies `phonemes` in-place, inserting stress-marker phonemes before
/// each vowel according to the language's stress rule.
///
/// Parameters:
/// - `phonemes`: the raw phoneme byte slice (may contain existing stress markers)
/// - `phdata`: phoneme data for flag lookups
/// - `opts`: language-specific stress options
/// - `dictionary_flags`: Some(flags) if a dict entry was found (bits 0-3 = explicit stress position)
/// - `tonic`: if >= 0, replace the highest stress with this level
/// - `control`: bit 0 = individual symbol; bit 1 = suffix phonemes to be added
pub fn set_word_stress(
    phonemes: &mut Vec<u8>,
    phdata: &PhonemeData,
    opts: &StressOpts,
    dictionary_flags: Option<u32>,
    tonic: i32,
    control: i32,
) {
    let stressflags = opts.stress_flags;
    let dflags = dictionary_flags.unwrap_or(0) as i32;

    // Extract explicit stress position from dict flags bits 0-2
    let mut stressed_syllable = (dflags & 0x7) as i32;
    let unstressed_word = (dflags & 0x8) != 0;
    if unstressed_word {
        stressed_syllable = (dflags & 0x3) as i32;
    }

    // Build internal phoneme copy for GetVowelStress
    let mut phonetic: Vec<u8> = phonemes.clone();
    // Ensure null-terminated
    if phonetic.last() != Some(&0) { phonetic.push(0); }

    let mut vowel_stress = [0i8; 100];
    let (mut max_stress, vowel_count, _primary_posn) = get_vowel_stress(
        &mut phonetic,
        phdata,
        &mut vowel_stress,
        1,  // control=1: mark phUNSTRESSED vowels
    );
    // After get_vowel_stress, stressed_syllable is set via primary_posn,
    // but here we override from dflags
    // Re-apply dict-specified stressed syllable
    if stressed_syllable > 0 {
        let sc = stressed_syllable.min(vowel_count - 1);
        vowel_stress[sc as usize] = STRESS_IS_PRIMARY;
        max_stress = STRESS_IS_PRIMARY as i32;
    }

    let max_stress_input = max_stress;

    // If no stress found and dictionary_flags is not None, treat as DIMINISHED
    if max_stress < 0 && dictionary_flags.is_some() {
        max_stress = STRESS_IS_DIMINISHED as i32;
    }

    // Compute syllable weights for stress algorithms that need them
    let mut syllable_weight = [0i8; 100];
    let mut vowel_length = [0i8; 100];
    compute_syllable_weights(
        &phonetic, phdata, vowel_count,
        &mut syllable_weight, &mut vowel_length,
    );

    // Final phoneme (for final-consonant checks)
    let final_ph = phonetic.iter().rev()
        .find(|&&c| c != 0)
        .copied()
        .unwrap_or(0);
    let final_ph2 = {
        let non_zero: Vec<u8> = phonetic.iter().filter(|&&c| c != 0).copied().collect();
        if non_zero.len() >= 2 { non_zero[non_zero.len()-2] } else { final_ph }
    };

    // ── Apply language stress rule ─────────────────────────────────────────
    match opts.stress_rule {
        STRESSPOSN_2LLH => {
            // 1st syllable unless 1st is light and 2nd is heavy → fall to 2L
            if syllable_weight[1] > 0 || syllable_weight[2] == 0 {
                // stay at first syllable (no change needed, trochaic will handle)
            } else {
                // Fall through to STRESSPOSN_2L behavior
                apply_2l(&mut vowel_stress, &mut max_stress, stressed_syllable, vowel_count);
            }
        }
        STRESSPOSN_2L => {
            apply_2l(&mut vowel_stress, &mut max_stress, stressed_syllable, vowel_count);
        }
        STRESSPOSN_2R => {
            // Penultimate vowel
            if stressed_syllable == 0 {
                max_stress = STRESS_IS_PRIMARY as i32;
                let mut ss = if vowel_count > 2 { vowel_count - 2 } else { 1 };

                if stressflags & S_FINAL_SPANISH != 0 {
                    // Spanish: stress last syllable if word ends in consonant ≠ n/s
                    if !phdata.get(final_ph).map(|p| p.typ == PH_VOWEL).unwrap_or(false) {
                        let mnem = phdata.get(final_ph).map(|p| p.mnemonic).unwrap_or(0);
                        let ph2_type = phdata.get(final_ph2).map(|p| p.typ).unwrap_or(0);
                        // simplified Spanish rule
                        if mnem != b'n' as u32 && mnem != b's' as u32 {
                            if ph2_type == PH_VOWEL {
                                ss = vowel_count - 1;
                            }
                        }
                    }
                }

                if stressflags & S_FINAL_LONG != 0 {
                    if vowel_length[(vowel_count-1) as usize] > vowel_length[(vowel_count-2) as usize] {
                        ss = vowel_count - 1;
                    }
                }

                // Avoid explicitly unstressed/diminished syllables
                if vowel_stress[ss as usize] == STRESS_IS_DIMINISHED
                    || vowel_stress[ss as usize] == STRESS_IS_UNSTRESSED {
                    if ss > 1 { ss -= 1; } else { ss += 1; }
                }

                if vowel_stress[ss as usize] < 0 {
                    let prev_ok = ss == 0 || vowel_stress[(ss-1) as usize] < STRESS_IS_PRIMARY;
                    let next_ok = ss as usize + 1 >= vowel_count as usize
                        || vowel_stress[(ss+1) as usize] < STRESS_IS_PRIMARY;
                    if prev_ok || next_ok {
                        vowel_stress[ss as usize] = STRESS_IS_PRIMARY;
                    }
                }
            }
        }
        STRESSPOSN_1R => {
            // Final vowel
            if stressed_syllable == 0 {
                let mut ss = vowel_count - 1;
                while ss > 0 {
                    if vowel_stress[ss as usize] < STRESS_IS_DIMINISHED {
                        vowel_stress[ss as usize] = STRESS_IS_PRIMARY;
                        break;
                    }
                    ss -= 1;
                }
                max_stress = STRESS_IS_PRIMARY as i32;
            }
        }
        STRESSPOSN_3R => {
            // Antipenultimate vowel
            if stressed_syllable == 0 {
                let mut ss = vowel_count - 3;
                if ss < 1 { ss = 1; }
                if max_stress == STRESS_IS_DIMINISHED as i32 {
                    vowel_stress[ss as usize] = STRESS_IS_PRIMARY;
                }
                max_stress = STRESS_IS_PRIMARY as i32;
            }
        }
        STRESSPOSN_SYLCOUNT => {
            // Russian-style: guess from syllable count
            if stressed_syllable == 0 {
                const GUESS_RU: [i32; 16] = [0,0,1,1,2,3,3,4,5,6,7,7,8,9,10,11];
                const GUESS_RU_V: [i32; 16] = [0,0,1,1,2,2,3,3,4,5,6,7,7,8,9,10];
                const GUESS_RU_T: [i32; 16] = [0,0,1,2,3,3,3,4,5,6,7,7,7,8,9,10];

                let ss = if (vowel_count as usize) < 16 {
                    let final_type = phdata.get(final_ph).map(|p| p.typ).unwrap_or(0);
                    if final_type == PH_VOWEL {
                        GUESS_RU_V[vowel_count as usize]
                    } else if final_type == 4 { // phSTOP
                        GUESS_RU_T[vowel_count as usize]
                    } else {
                        GUESS_RU[vowel_count as usize]
                    }
                } else {
                    vowel_count - 3
                };
                if ss > 0 && ss < vowel_count {
                    vowel_stress[ss as usize] = STRESS_IS_PRIMARY;
                }
                max_stress = STRESS_IS_PRIMARY as i32;
            }
        }
        STRESSPOSN_1RH => {
            // Heaviest syllable (Hindi)
            if stressed_syllable == 0 {
                let mut max_weight = -1i8;
                let mut ss = 1i32;
                for ix in 1..vowel_count-1 {
                    if vowel_stress[ix as usize] < STRESS_IS_DIMINISHED {
                        let wt = syllable_weight[ix as usize];
                        if wt >= max_weight {
                            max_weight = wt;
                            ss = ix;
                        }
                    }
                }
                if syllable_weight[(vowel_count-1) as usize] == 2 && max_weight < 2 {
                    ss = vowel_count - 1;
                } else if max_weight <= 0 {
                    ss = 1;
                }
                vowel_stress[ss as usize] = STRESS_IS_PRIMARY;
                max_stress = STRESS_IS_PRIMARY as i32;
            }
        }
        STRESSPOSN_1RU => {
            // Turkish: last syllable before any unstressed vowel
            if stressed_syllable == 0 {
                let mut ss = vowel_count - 1;
                for ix in 1..vowel_count {
                    if vowel_stress[ix as usize] == STRESS_IS_UNSTRESSED {
                        ss = ix - 1;
                        break;
                    }
                }
                vowel_stress[ss as usize] = STRESS_IS_PRIMARY;
                max_stress = STRESS_IS_PRIMARY as i32;
            }
        }
        STRESSPOSN_ALL => {
            for ix in 1..vowel_count {
                if vowel_stress[ix as usize] < STRESS_IS_DIMINISHED {
                    vowel_stress[ix as usize] = STRESS_IS_PRIMARY;
                }
            }
        }
        STRESSPOSN_GREENLANDIC => {
            let mut long_vowel = 0i32;
            for ix in 1..vowel_count {
                if vowel_stress[ix as usize] == STRESS_IS_PRIMARY {
                    vowel_stress[ix as usize] = STRESS_IS_SECONDARY;
                }
                if vowel_length[ix as usize] > 0 {
                    long_vowel = ix;
                    vowel_stress[ix as usize] = STRESS_IS_SECONDARY;
                }
            }
            let ss = if stressed_syllable == 0 {
                if long_vowel > 0 { long_vowel }
                else if vowel_count > 5 { vowel_count - 3 }
                else { vowel_count - 1 }
            } else { stressed_syllable };
            if ss > 0 && ss < vowel_count { vowel_stress[ss as usize] = STRESS_IS_PRIMARY; }
            max_stress = STRESS_IS_PRIMARY as i32;
        }
        STRESSPOSN_1SL => {
            if stressed_syllable == 0 {
                let ss = if vowel_length[1] == 0 && vowel_count > 2 && vowel_length[2] > 0 { 2i32 } else { 1i32 };
                vowel_stress[ss as usize] = STRESS_IS_PRIMARY;
                max_stress = STRESS_IS_PRIMARY as i32;
            }
        }
        STRESSPOSN_EU => {
            if stressed_syllable == 0 && vowel_count > 2 {
                for ix in 1..vowel_count {
                    vowel_stress[ix as usize] = STRESS_IS_DIMINISHED;
                }
                let ss = 2i32;
                if max_stress == STRESS_IS_DIMINISHED as i32 {
                    vowel_stress[ss as usize] = STRESS_IS_PRIMARY;
                }
                max_stress = STRESS_IS_PRIMARY as i32;
                if vowel_count > 3 {
                    vowel_stress[(vowel_count-1) as usize] = STRESS_IS_SECONDARY;
                }
            }
        }
        _ => {
            // STRESSPOSN_1L (0) = default: trochaic, handled below
        }
    }

    // ── Final vowel unstressed option ─────────────────────────────────────
    if stressflags & S_FINAL_VOWEL_UNSTRESSED != 0
        && (control & 2) == 0
        && vowel_count > 2
        && max_stress_input < STRESS_IS_SECONDARY as i32
        && vowel_stress[(vowel_count-1) as usize] == STRESS_IS_PRIMARY
    {
        if phdata.get(final_ph).map(|p| p.typ == PH_VOWEL).unwrap_or(false) {
            vowel_stress[(vowel_count-1) as usize] = STRESS_IS_UNSTRESSED;
            vowel_stress[(vowel_count-2) as usize] = STRESS_IS_PRIMARY;
        }
    }

    // ── Determine base stress for trochaic fill ────────────────────────────
    let mut stress: i8 = if max_stress < STRESS_IS_PRIMARY as i32 {
        STRESS_IS_PRIMARY  // no primary marked → use primary for first syllable
    } else {
        STRESS_IS_SECONDARY
    };

    // ── 2-syllable rule ────────────────────────────────────────────────────
    if !unstressed_word {
        if stressflags & S_2_SYL_2 != 0 && vowel_count == 3 {
            if vowel_stress[1] == STRESS_IS_PRIMARY { vowel_stress[2] = STRESS_IS_SECONDARY; }
            if vowel_stress[2] == STRESS_IS_PRIMARY { vowel_stress[1] = STRESS_IS_SECONDARY; }
        }
        if stressflags & S_INITIAL_2 != 0 && vowel_stress[1] < STRESS_IS_DIMINISHED {
            if vowel_count > 3 && vowel_stress[2] >= STRESS_IS_PRIMARY {
                vowel_stress[1] = STRESS_IS_SECONDARY;
            }
        }
    }

    // ── Trochaic fill ──────────────────────────────────────────────────────
    let mut done = false;
    let mut first_primary = 0i32;

    for v in 1..vowel_count {
        if vowel_stress[v as usize] < STRESS_IS_DIMINISHED {
            // Candidate for stress assignment
            if stressflags & S_FINAL_NO_2 != 0
                && (stress as i32) < STRESS_IS_PRIMARY as i32
                && v == vowel_count - 1
            {
                // Don't give secondary stress to final vowel
            } else if stressflags & 0x8000 != 0 && !done {
                // Priority: left-to-right fill
                vowel_stress[v as usize] = stress;
                done = true;
                stress = STRESS_IS_SECONDARY;
            } else {
                // Trochaic: stress a vowel surrounded by unstressed vowels
                let prev_ok = vowel_stress[(v-1) as usize] <= STRESS_IS_UNSTRESSED;
                let next_vs = vowel_stress[(v+1) as usize];
                let next_ok = next_vs <= STRESS_IS_UNSTRESSED
                    || (stress == STRESS_IS_PRIMARY && next_vs <= STRESS_IS_NOT_STRESSED);

                if prev_ok && next_ok {
                    // Check S_2_TO_HEAVY: skip light syllables if heavy exists later
                    let skip = if v > 1 && stressflags & S_2_TO_HEAVY != 0
                        && syllable_weight[v as usize] == 0 {
                        (v..vowel_count-1).any(|i| syllable_weight[i as usize] > 0)
                            || (syllable_weight[(v+1) as usize] > 0)
                    } else {
                        false
                    };

                    if !skip {
                        if stress == STRESS_IS_SECONDARY && stressflags & S_NO_AUTO_2 != 0 {
                            // Don't assign secondary stress automatically
                        } else {
                            vowel_stress[v as usize] = stress;
                            done = true;
                            stress = STRESS_IS_SECONDARY;
                        }
                    }
                }
            }
        }

        if vowel_stress[v as usize] >= STRESS_IS_PRIMARY {
            if first_primary == 0 {
                first_primary = v;
            } else if stressflags & S_FIRST_PRIMARY != 0 {
                vowel_stress[v as usize] = STRESS_IS_SECONDARY;
            }
        }
    }

    // ── Tonic / unstressed word handling ──────────────────────────────────
    let tonic = if unstressed_word && tonic < 0 {
        if vowel_count <= 2 { opts.unstressed_wd1 }
        else { opts.unstressed_wd2 }
    } else {
        tonic
    };

    // Find highest-stress position
    let mut ms = STRESS_IS_DIMINISHED as i32;
    let mut ms_posn = 0i32;
    for v in 1..vowel_count {
        if vowel_stress[v as usize] as i32 >= ms {
            ms = vowel_stress[v as usize] as i32;
            ms_posn = v;
        }
    }

    if tonic >= 0 {
        if tonic > ms || ms <= STRESS_IS_PRIMARY as i32 {
            if ms_posn > 0 && ms_posn < 100 {
                vowel_stress[ms_posn as usize] = tonic as i8;
            }
        }
        ms = tonic;
    }

    let max_stress_final = ms;

    // ── Build output phoneme string with stress markers ────────────────────
    let mut output: Vec<u8> = Vec::with_capacity(phonetic.len() + vowel_count as usize + 4);

    // Handle vowel-initial word (vowel_pause)
    // (simplified: skip for now — affects pause insertion, not IPA output)

    let mut v = 1i32;
    let phonetic_slice: &[u8] = if phonetic.last() == Some(&0) {
        &phonetic[..phonetic.len()-1]
    } else {
        &phonetic
    };

    for &phcode in phonetic_slice {
        if phcode == 0 { break; }
        let ph = phdata.get(phcode);

        let ph_type = ph.map(|p| p.typ).unwrap_or(0);
        let ph_flags = ph.map(|p| p.phflags).unwrap_or(0);

        let _is_pause = ph_type == PH_PAUSE;
        let is_vowel = ph_type == PH_VOWEL && ph_flags & PH_NONSYLLABIC == 0;

        // Check for syllabic marker (next phoneme PHON_SYLLABIC)
        // We don't have lookahead here, so skip for now

        if is_vowel {
            debug_assert!(v <= vowel_count, "v={} vowel_count={}", v, vowel_count);
            if v <= vowel_count {
                let mut v_stress = vowel_stress[v as usize] as i32;

                if v_stress <= STRESS_IS_UNSTRESSED as i32 {
                    // Decide final stress level for weak/unstressed vowels
                    if stressflags & S_FINAL_DIM != 0
                        && v > 1
                        && max_stress_final >= STRESS_IS_NOT_STRESSED as i32
                        && v == vowel_count - 1
                    {
                        v_stress = STRESS_IS_DIMINISHED as i32;
                    } else if stressflags & S_NO_DIM != 0
                        || v == 1
                        || v == vowel_count - 1
                    {
                        v_stress = STRESS_IS_UNSTRESSED as i32;
                    } else if v == vowel_count - 2
                        && vowel_stress[(vowel_count-1) as usize] <= STRESS_IS_UNSTRESSED
                    {
                        v_stress = STRESS_IS_UNSTRESSED as i32;
                    } else {
                        // Unstressed internal syllable
                        let prev_vs = vowel_stress[(v-1) as usize] as i32;
                        if prev_vs < STRESS_IS_DIMINISHED as i32
                            || stressflags & S_MID_DIM == 0 {
                            v_stress = STRESS_IS_DIMINISHED as i32;
                            vowel_stress[v as usize] = STRESS_IS_DIMINISHED;
                        }
                    }
                }

                // Emit stress marker if needed
                // Emit if DIMINISHED (0) or > UNSTRESSED (1)
                if v_stress == STRESS_IS_DIMINISHED as i32
                    || v_stress > STRESS_IS_UNSTRESSED as i32
                {
                    let idx = v_stress.max(0).min(6) as usize;
                    output.push(STRESS_PHONEMES[idx]);
                }

                v += 1;
            }
        }

        // Preserve END_WORD (||) markers — used for word-boundary spacing in numbers.
        // All other phonemes are passed through.
        output.push(phcode);
    }

    output.push(0);
    *phonemes = output;
}

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Change word stress to a given level.
///
/// Port of `ChangeWordStress()` from translateword.c.
/// Finds the highest-stressed vowel and sets it to `new_stress`.
/// If `new_stress >= STRESS_IS_PRIMARY`, promotes the vowel.
/// Otherwise, demotes all vowels above `new_stress`.
pub fn change_word_stress(
    phonemes: &mut Vec<u8>,
    phdata: &PhonemeData,
    new_stress: i32,
) {
    let mut phonetic: Vec<u8> = phonemes.clone();
    if phonetic.last() != Some(&0) { phonetic.push(0); }

    let mut vowel_stress = [0i8; 100];
    // control=0: do not strip stress markers (just read them)
    let (max_stress, vowel_count, _primary_posn) = get_vowel_stress(
        &mut phonetic,
        phdata,
        &mut vowel_stress,
        0,
    );

    if new_stress >= STRESS_IS_PRIMARY as i32 {
        // Promote: find first vowel at max_stress level and raise it
        for ix in 1..vowel_count as usize {
            if vowel_stress[ix] as i32 >= max_stress {
                vowel_stress[ix] = new_stress as i8;
                break;
            }
        }
    } else {
        // Demote: lower all vowels above new_stress
        for ix in 1..vowel_count as usize {
            if vowel_stress[ix] as i32 > new_stress {
                vowel_stress[ix] = new_stress as i8;
            }
        }
    }

    // Rebuild phoneme string with updated stress markers
    let mut output: Vec<u8> = Vec::with_capacity(phonetic.len() + vowel_count as usize + 2);
    let mut v = 1i32;
    let slice: &[u8] = if phonetic.last() == Some(&0) {
        &phonetic[..phonetic.len()-1]
    } else {
        &phonetic
    };

    for &phcode in slice {
        if phcode == 0 { break; }
        let ph = phdata.get(phcode);
        let ph_type = ph.map(|p| p.typ).unwrap_or(0);
        let ph_flags = ph.map(|p| p.phflags).unwrap_or(0);
        let is_vowel = ph_type == PH_VOWEL && ph_flags & PH_NONSYLLABIC == 0;

        if is_vowel && v < vowel_count {
            let vs = vowel_stress[v as usize] as i32;
            if vs == STRESS_IS_DIMINISHED as i32 || vs > STRESS_IS_UNSTRESSED as i32 {
                output.push(STRESS_PHONEMES[vs.max(0).min(6) as usize]);
            }
            v += 1;
        }
        output.push(phcode);
    }
    output.push(0);
    *phonemes = output;
}

/// Promote secondary stress to primary for words with $strend/$strend2 flag.
///
/// Implements the clause-level stress promotion for words marked with
/// `$strend` (bit 9 = 0x200) or `$strend2` (bit 10 = 0x400) in dictionary flags.
///
/// - `$strend`: promote if word is at clause end (is_clause_end=true)
/// - `$strend2`: promote if clause end OR only followed by unstressed words
///
/// Mirrors `ChangeWordStress(tr, word_phonemes, 4)` in translateword.c:608.
pub fn promote_strend_stress(
    phonemes: &mut Vec<u8>,
    phdata: &PhonemeData,
    dict_flags: u32,
    is_clause_end: bool,
    only_unstressed_follow: bool,
) {
    const FLAG_STREND:  u32 = 1 << 9;   // 0x200
    const FLAG_STREND2: u32 = 1 << 10;  // 0x400

    let should_promote = if dict_flags & FLAG_STREND2 != 0 {
        is_clause_end || only_unstressed_follow
    } else if dict_flags & FLAG_STREND != 0 {
        is_clause_end
    } else {
        return; // no strend flag
    };

    if !should_promote { return; }

    // Check if there's already a primary stress marker (PHON_STRESS_P=6 or PHON_STRESS_P2=7)
    let has_primary = phonemes.iter().any(|&c| c == PHON_STRESS_P || c == PHON_STRESS_P2);
    if has_primary { return; }

    // Use ChangeWordStress to promote to STRESS_IS_PRIMARY (=4)
    change_word_stress(phonemes, phdata, STRESS_IS_PRIMARY as i32);
}

fn apply_2l(
    vowel_stress: &mut [i8; 100],
    max_stress: &mut i32,
    stressed_syllable: i32,
    vowel_count: i32,
) {
    if stressed_syllable == 0 && vowel_count > 2 {
        let ss = 2usize;
        if *max_stress == STRESS_IS_DIMINISHED as i32 {
            vowel_stress[ss] = STRESS_IS_PRIMARY;
        }
        *max_stress = STRESS_IS_PRIMARY as i32;
    }
}

/// Compute syllable_weight[] and vowel_length[] arrays.
///
/// Mirrors the C code that walks phonetic[] checking for long vowels and
/// consonant clusters after each vowel.
fn compute_syllable_weights(
    phonetic: &[u8],
    phdata: &PhonemeData,
    vowel_count: i32,
    syllable_weight: &mut [i8; 100],
    vowel_length: &mut [i8; 100],
) {
    // consonant_types[]: 0 for non-consonant, 1 for consonant types 3-9
    // type 3=phFRICATIVE, 4=phSTOP, 5=phNASAL, 6=phVFRICATIVE, 7=phLIQUID, 8=phFLAP, 9=phTRILL
    let consonant_types = |typ: u8| -> bool {
        matches!(typ, 3..=9)
    };

    let mut ix = 1usize;
    let len = phonetic.len();
    let mut pos = 0usize;

    while pos < len && ix < vowel_count as usize {
        let phcode = phonetic[pos];
        if phcode == 0 { break; }
        pos += 1;

        let ph = phdata.get(phcode);
        let ph_type = ph.map(|p| p.typ).unwrap_or(0);
        let ph_flags = ph.map(|p| p.phflags).unwrap_or(0);

        if ph_type == PH_VOWEL && ph_flags & PH_NONSYLLABIC == 0 {
            let mut weight = 0i8;

            // Check if next phoneme is PHON_LENGTHENED
            let next_code = phonetic.get(pos).copied().unwrap_or(0);
            let lengthened = next_code == PHON_LENGTHENED;

            if lengthened || ph_flags & PH_LONG != 0 {
                weight += 1;
            }
            vowel_length[ix] = weight;

            if lengthened && pos < len { pos += 1; } // advance over LENGTHENED

            // Check next 2 phonemes for consonant cluster
            let next1 = phonetic.get(pos).copied().unwrap_or(0);
            let next2 = phonetic.get(pos+1).copied().unwrap_or(0);
            let next1_type = phdata.get(next1).map(|p| p.typ).unwrap_or(0);
            let next2_type = phdata.get(next2).map(|p| p.typ).unwrap_or(0);
            let next1_flags = phdata.get(next1).map(|p| p.phflags).unwrap_or(0);

            // Followed by two consonants, a long consonant, or consonant at word end
            let next1_is_cons = consonant_types(next1_type);
            let next2_is_vowel = next2_type == PH_VOWEL;
            let next1_is_long = next1_flags & PH_LONG != 0;

            if next1_is_cons && (!next2_is_vowel || next1_is_long) {
                weight += 1;
            }
            syllable_weight[ix] = weight;
            ix += 1;
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phoneme::load::PhonemeData;
    use std::path::Path;

    fn load_phdata() -> Option<PhonemeData> {
        let dir = Path::new("/usr/share/espeak-ng-data");
        if !dir.join("phontab").exists() { return None; }
        let mut pd = PhonemeData::load(dir).ok()?;
        pd.select_table_by_name("en").ok()?;
        Some(pd)
    }

    #[test]
    fn hello_gets_stress_on_second_vowel() {
        let Some(phdata) = load_phdata() else { return; };
        // hello dict phonemes: [h=65, @=13, l=55, oU=144]
        let mut phonemes = vec![65u8, 13, 55, 144, 0];
        let opts = StressOpts::for_lang("en");
        set_word_stress(&mut phonemes, &phdata, &opts, Some(0), -1, 0);
        // Expected: [h=65, @=13, l=55, STRESS_P=6, oU=144, 0]
        // i.e. stress before oU (second vowel)
        println!("hello phonemes after stress: {:?}", phonemes);
        // Find stress marker position
        let stress_pos = phonemes.iter().position(|&c| c == 6);
        let ou_pos = phonemes.iter().position(|&c| c == 144);
        assert!(stress_pos.is_some(), "should have a primary stress marker");
        assert_eq!(stress_pos.map(|p| p + 1), ou_pos, "stress marker should precede oU");
    }

    #[test]
    fn world_gets_stress_on_first_vowel() {
        let Some(phdata) = load_phdata() else { return; };
        // world phonemes: [w, 3:=136, l, d] — 3: is a full vowel, no phUNSTRESSED
        // So trochaic should give stress to vowel at position 1
        let w = phdata.lookup_phoneme("w");
        let v3c = phdata.lookup_phoneme("3:");
        let l = phdata.lookup_phoneme("l");
        let d = phdata.lookup_phoneme("d");
        if w == 0 || v3c == 0 { return; } // phoneme lookup failed
        let mut phonemes = vec![w, v3c, l, d, 0];
        let opts = StressOpts::for_lang("en");
        set_word_stress(&mut phonemes, &phdata, &opts, Some(0), -1, 0);
        println!("world phonemes after stress: {:?}", phonemes);
        // 3: is NOT phUNSTRESSED, so vowel_stress[1] = -1 initially → stress placed there
        let stress_pos = phonemes.iter().position(|&c| c == 6);
        let v3c_pos = phonemes.iter().position(|&c| c == v3c);
        assert!(stress_pos.is_some(), "should have primary stress");
        assert_eq!(stress_pos.map(|p| p + 1), v3c_pos, "stress should precede 3:");
    }
}
