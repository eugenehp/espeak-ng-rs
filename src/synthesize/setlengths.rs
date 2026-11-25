//! Phoneme duration calculation — simplified port of `setlengths.c`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::indexing_slicing))]

// src/synthesize/setlengths.rs
//
// Simplified port of setlengths.c — computes phoneme durations.
//
// For 100% parity we implement the English default vowel-length calculation,
// which is the most impactful source of timing error.
//
// References:
//   setlengths.c  CalcLengths()
//   tr_languages.c  NewTranslator() stress_lengths2, stress_amps2

// ---------------------------------------------------------------------------
// English default stress parameters  (tr_languages.c, NewTranslator)
// ---------------------------------------------------------------------------

/// `speed_lookup` — words-per-minute (80–359) to eSpeak's internal speed
/// factor, transcribed from `setlengths.c`.  A *larger* value means *longer*
/// phonemes; 175 wpm (the default) maps to 87.
use super::lang_prosody::LangProsody;
use crate::bytes::Bounded;
pub const SPEED_LOOKUP: [u8; 280] = [
    255, 255, 255, 255, 255, 253, 249, 245, 242, 238, // 80
    235, 232, 228, 225, 222, 218, 216, 213, 210, 207, // 90
    204, 201, 198, 196, 193, 191, 188, 186, 183, 181, // 100
    179, 176, 174, 172, 169, 168, 165, 163, 161, 159, // 110
    158, 155, 153, 152, 150, 148, 146, 145, 143, 141, // 120
    139, 137, 136, 135, 133, 131, 130, 129, 127, 126, // 130
    124, 123, 122, 120, 119, 118, 117, 115, 114, 113, // 140
    112, 111, 110, 109, 107, 106, 105, 104, 103, 102, // 150
    101, 100, 99, 98, 97, 96, 95, 94, 93, 92, // 160
    91, 90, 89, 89, 88, 87, 86, 85, 84, 83, // 170
    82, 82, 81, 80, 80, 79, 78, 77, 76, 76, // 180
    75, 75, 74, 73, 72, 71, 71, 70, 69, 69, // 190
    68, 67, 67, 66, 66, 65, 64, 64, 63, 62, // 200
    62, 61, 61, 60, 59, 59, 58, 58, 57, 57, // 210
    56, 56, 55, 54, 54, 53, 53, 52, 52, 52, // 220
    51, 50, 50, 49, 49, 48, 48, 47, 47, 46, // 230
    46, 46, 45, 45, 44, 44, 44, 43, 43, 42, // 240
    41, 40, 40, 40, 39, 39, 39, 38, 38, 38, // 250
    37, 37, 37, 36, 36, 35, 35, 35, 35, 34, // 260
    34, 34, 33, 33, 33, 32, 32, 31, 31, 31, // 270
    30, 30, 30, 29, 29, 29, 29, 28, 28, 27, // 280
    27, 27, 27, 26, 26, 26, 26, 25, 25, 25, // 290
    24, 24, 24, 24, 23, 23, 23, 23, 22, 22, // 300
    22, 21, 21, 21, 21, 20, 20, 20, 20, 19, // 310
    19, 19, 18, 18, 17, 17, 17, 16, 16, 16, // 320
    16, 16, 16, 15, 15, 15, 15, 14, 14, 14, // 330
    13, 13, 13, 12, 12, 12, 12, 11, 11, 11, // 340
    11, 10, 10, 10, 9, 9, 9, 8, 8, 8, // 350
];

/// `SetSpeedMultiplier` — the internal speed factor for a rate in wpm.
pub fn speed_multiplier(wpm: u32) -> u32 {
    const MIN: u32 = 80;
    let wpm = wpm.min(450);
    let ix = wpm.clamp(MIN, 359) - MIN;
    let mut x = SPEED_LOOKUP.get(ix as usize).copied().unwrap_or_default() as u32;
    if wpm >= 380 {
        x = 7;
    }
    if wpm >= 400 {
        x = 6;
    }
    x.max(1)
}

/// The duration multiplier for `wpm` relative to the 175 wpm default.
///
/// This is the port's whole rate model: `1.0` would be "no scaling at all", and
/// the default 175 wpm returns ~0.808.  It replaces the old linear `175 / wpm`
/// (times a fixed calibration), which over-stretched below ~100 wpm — upstream's
/// curve flattens towards its 80 wpm floor.
///
/// Upstream's `speed_lookup` scales only the *length-modifier* part of an
/// utterance; recorded consonants and the fixed pauses don't follow it, so the
/// total duration is affine in `x`, not proportional to it.  Fitting the C
/// binary's measured durations over 80–400 wpm gives a rate-independent share
/// of about 40 %:
///
/// ```text
/// duration(wpm) ∝ 0.47 + 0.33 · x(wpm)/x(175)
/// ```
///
/// which lands within a few percent of upstream from 80 wpm to 300 (400 wpm,
/// upstream's own extreme, is ~19 % long).
pub fn speed_duration_factor(wpm: u32) -> f64 {
    // Fitted against the C binary over 80–400 wpm: a rate-independent offset
    // plus a term proportional to upstream's speed factor.  The constants
    // subsume the old `RATE_CALIBRATION` — at 175 wpm this returns 0.808, where
    // the port used to use 0.836 with a second, separate scaling in `wavegen`.
    const FIXED: f64 = 0.502;
    const SCALED: f64 = 0.3546;
    let ratio = speed_multiplier(wpm) as f64 / speed_multiplier(175) as f64;
    FIXED + SCALED * ratio
}
/// stress_lengths2[stress] — stress-level length multiplier, units 1/128.
/// Indexed 0..7.  After the CalcLengths swap (stress 0↔1), stress=0 means
/// "diminished" and stress=1 means "unstressed".
pub const STRESS_LENGTHS_EN: [u32; 8] = [182, 140, 220, 220, 220, 240, 260, 280];

/// The language's `stress_lengths`, transcribed from `tr_languages.c`; a zero
/// stands for the primary-stress entry (index 6).
pub fn stress_lengths_for(lang: &str) -> Option<[u32; 8]> {
    let primary = lang.split('-').next().unwrap_or(lang);
    Some(match primary {
        "af" => [170, 140, 220, 220, 0, 0, 250, 270],
        "am" | "fr" | "tt" => [190, 170, 190, 200, 0, 0, 190, 240],
        "ro" | "rup" => [170, 170, 180, 180, 0, 0, 240, 260],
        "be" => [160, 140, 200, 140, 0, 0, 240, 160],
        "as" | "bn" | "bpy" => [180, 180, 210, 210, 0, 0, 230, 240],
        "cy" => [170, 220, 180, 180, 0, 0, 250, 270],
        "da" | "fo" | "nb" | "no" => [160, 140, 200, 200, 0, 0, 220, 230],
        "de" => [150, 130, 200, 200, 0, 0, 270, 270],
        "en" => [182, 140, 220, 220, 0, 0, 248, 275],
        "el" | "grc" => [155, 180, 210, 210, 0, 0, 270, 300],
        "eo" | "io" => [150, 140, 180, 180, 0, 0, 200, 200],
        "an" | "ca" | "es" | "ia" | "pap" => [160, 145, 155, 150, 0, 0, 200, 245],
        "eu" => [200, 200, 200, 200, 0, 0, 210, 230],
        "gu" | "hi" | "mr" | "ne" | "or" | "pa" => [190, 190, 210, 210, 0, 0, 230, 250],
        "bs" | "hr" | "sr" => [160, 150, 200, 200, 0, 0, 250, 260],
        "hu" => [185, 195, 195, 190, 0, 0, 210, 220],
        "hy" => [250, 200, 250, 250, 0, 0, 250, 250],
        "id" | "ms" => [160, 200, 180, 180, 0, 0, 220, 240],
        "is" => [180, 160, 200, 200, 0, 0, 240, 250],
        "it" => [160, 140, 150, 165, 0, 0, 218, 305],
        "jbo" => [145, 145, 170, 160, 0, 0, 330, 350],
        "ka" | "si" => [200, 200, 210, 210, 0, 0, 230, 230],
        "kk" => [190, 180, 230, 230, 0, 0, 250, 250],
        "kl" => [230, 230, 230, 230, 0, 0, 230, 230],
        "ku" => [180, 180, 190, 180, 0, 0, 230, 240],
        "ltg" | "lv" => [180, 180, 180, 160, 0, 0, 230, 180],
        "mk" => [180, 160, 200, 200, 0, 0, 220, 230],
        "mn" => [190, 180, 220, 220, 0, 0, 260, 240],
        "om" => [200, 200, 200, 200, 0, 0, 200, 200],
        "pl" => [160, 190, 175, 175, 0, 0, 200, 210],
        "pt" => [170, 115, 210, 240, 0, 0, 260, 280],
        "cs" | "sk" => [190, 190, 210, 210, 0, 0, 210, 210],
        "smj" => [150, 180, 200, 200, 0, 0, 210, 250],
        "sq" => [150, 150, 180, 180, 0, 0, 300, 300],
        "sv" => [160, 135, 220, 220, 0, 0, 250, 280],
        "sw" | "tn" => [160, 170, 200, 200, 0, 0, 320, 340],
        "kn" | "ml" | "ta" | "te" => [230, 230, 240, 240, 0, 0, 260, 260],
        "az" | "tr" => [190, 180, 200, 230, 0, 0, 240, 250],
        "vi" => [150, 150, 180, 180, 210, 230, 230, 240],
        "cmn" | "yue" | "zh" => [230, 150, 230, 230, 230, 0, 240, 250],
        _ => return None,
    })
}

/// stress_amps2[stress] — amplitude weight (arbitrary units, 0–25).
pub const STRESS_AMPS_EN: [u8; 8] = [18, 18, 20, 20, 20, 22, 22, 20];

/// lengthen_tonic — extra length added for tonic syllable (stress ≥ 7).
pub const LENGTHEN_TONIC_EN: u32 = 20;

/// LOPT_MAXAMP_EOC — max amplitude allowed for end-of-clause phoneme.
pub const MAXAMP_EOC_EN: u8 = 19;

// ---------------------------------------------------------------------------
// Length-modification tables  (setlengths.c)
//
// Indexed as `table[next2_lm_group * 10 + next_lm_group]`.
// Length-mod groups (ph->length_mod from the phoneme definition):
//   0 = vowel / default
//   1 = pause / clause boundary
//   2 = voiceless stop (t, p, k)
//   3 = voiceless fricative (s, f, sh, …)
//   4 = nasal (n, m, ŋ)
//   5 = voiced stop (d, b, g)
//   6 = voiced fricative (z, v, …)
//   7 = sonorant / approximant (l, r, w, j)
//   8 = nasal-stop (N: n before k/g)
//   9 = (default, same as 0)
// ---------------------------------------------------------------------------

/// length_mods_en — for vowels that are NOT the last syllable in their word.
pub const LENGTH_MODS_EN: [u8; 100] = [
    //   a    ,    t    s    n    d    z    r    N    (←next)
    100, 120, 100, 105, 100, 110, 110, 100, 95, 100, // a  ← next2
    105, 120, 105, 110, 125, 130, 135, 115, 125, 100, // ,
    105, 120, 75, 100, 75, 105, 120, 85, 75, 100, // t
    105, 120, 85, 105, 95, 115, 120, 100, 95, 100, // s
    110, 120, 95, 105, 100, 115, 120, 100, 100, 100, // n
    105, 120, 100, 105, 95, 115, 120, 110, 95, 100, // d
    105, 120, 100, 105, 105, 122, 125, 110, 105, 100, // z
    105, 120, 100, 105, 105, 122, 125, 110, 105, 100, // r
    105, 120, 95, 105, 100, 115, 120, 110, 100, 100, // N
    100, 120, 100, 100, 100, 100, 100, 100, 100, 100, // default
];

/// length_mods_en0 — for vowels that ARE the last syllable in their word.
pub const LENGTH_MODS_EN0: [u8; 100] = [
    //   a    ,    t    s    n    d    z    r    N    (←next)
    100, 150, 100, 105, 110, 115, 110, 110, 110, 100, // a
    105, 150, 105, 110, 125, 135, 140, 115, 135, 100, // ,
    105, 150, 90, 105, 90, 122, 135, 100, 90, 100, // t
    105, 150, 100, 105, 100, 122, 135, 100, 100, 100, // s
    105, 150, 100, 105, 105, 115, 135, 110, 105, 100, // n
    105, 150, 100, 105, 105, 122, 130, 120, 125, 100, // d
    105, 150, 100, 105, 110, 122, 125, 115, 110, 100, // z
    105, 150, 100, 105, 105, 122, 135, 120, 105, 100, // r
    105, 150, 100, 105, 105, 115, 135, 110, 105, 100, // N
    100, 100, 100, 100, 100, 100, 100, 100, 100, 100, // default
];

// ---------------------------------------------------------------------------
// calc_vowel_length_mod
// ---------------------------------------------------------------------------

/// Compute the length-modification factor for one vowel phoneme (English).
///
/// [`vowel_length_mod`] does it for any language.
///
/// This mirrors the `phVOWEL` case in `CalcLengths()` from setlengths.c.
///
/// # Parameters
/// * `stress`        — phoneme stress level (0-7).  **Already after the
///                     0↔1 swap** that CalcLengths applies.
/// * `next_lm`       — `ph->length_mod` of the **next** phoneme (0-9).
/// * `next2_lm`      — `ph->length_mod` of the phoneme after that (0-9).
/// * `more_syllables`— how many more syllables its word has.
/// * `end_of_clause` — `true` if this is the last syllable before the
///                     clause boundary.
/// * `std_length`    — `ph->std_length` of this vowel (in mS/2 units).
///
/// Returns `length_mod`: a 256-based scale factor (256 = no change).
pub fn calc_vowel_length_mod(
    stress: u8,
    next_lm: u8,
    next2_lm: u8,
    more_syllables: u8,
    end_of_clause: bool,
    std_length: u8,
    // The language's own `stressLength` table, when its voice file declares one
    // (upstream #1466 — twenty shipped languages do).  `None` = the built-in
    // English defaults.
    stress_lengths: Option<&[u32; 8]>,
    // `len_speeds`: the speaking rate, 128 = 100% (see [`len_speeds`]).
    speeds: [u32; 3],
) -> u32 {
    let prosody = LangProsody {
        stress_lengths: stress_lengths.copied().unwrap_or(STRESS_LENGTHS_EN),
        ..LangProsody::default()
    };
    vowel_length_mod(
        &VowelContext {
            stress,
            next_lm,
            next2_lm,
            more_syllables,
            end_of_clause,
            std_length,
            next_newword: false,
            tone_percent: 0,
        },
        &prosody,
        speeds,
    )
}

/// What a vowel's length depends on (see [`calc_vowel_length_mod`]).
#[derive(Debug, Clone, Copy)]
pub struct VowelContext {
    pub stress: u8,
    pub next_lm: u8,
    pub next2_lm: u8,
    pub more_syllables: u8,
    pub end_of_clause: bool,
    pub std_length: u8,
    /// The next phoneme starts a word.
    pub next_newword: bool,
    /// What its tone makes of the length, in percent; 0 = nothing.
    pub tone_percent: u32,
}

/// The length modifier of a vowel: the `phVOWEL` case of `CalcLengths`.
pub fn vowel_length_mod(vowel: &VowelContext, prosody: &LangProsody, speeds: [u32; 3]) -> u32 {
    let stress = usize::from(vowel.stress.min(7));
    let table = if vowel.more_syllables > 0 || prosody.same_length_mods {
        &LENGTH_MODS_EN
    } else {
        &LENGTH_MODS_EN0
    };
    let lookup = |next2: u8, next: u8| {
        let ix = usize::from(next2.min(9)) * 10 + usize::from(next.min(9));
        u32::from(table.get(ix).copied().unwrap_or(100))
    };
    let mut length_mod = lookup(vowel.next2_lm, vowel.next_lm);
    if vowel.more_syllables == 0 && vowel.next_newword && prosody.word_gap & 0x20 != 0 {
        // Treated as a pause followed by the next word.
        length_mod = (length_mod + lookup(vowel.next_lm, 1)) / 2;
    }
    let speed = match vowel.more_syllables {
        0 => speeds[0],
        1 => speeds[1],
        _ => speeds[2],
    };
    let mut length_mod = (length_mod * speed / 128).max(8);

    // Tonic syllable: a constant part, which doesn't shrink with the rate.
    if stress >= 7 {
        length_mod += prosody.lengthen_tonic;
    }
    // A zero stands for the primary-stress entry.
    length_mod *= match prosody.stress_lengths.at(stress) {
        0 => prosody.stress_lengths[6],
        n => n,
    };
    if vowel.tone_percent > 0 {
        length_mod = length_mod * vowel.tone_percent / 100;
    }
    if vowel.end_of_clause && !prosody.no_eoc_lengthen {
        // (Negative for a vowel longer than 280 ms, as in C.)
        let len = if prosody.eoc_lengthen_evenly {
            200
        } else {
            i32::from(vowel.std_length) * 2
        };
        let eoc = 256 + (280 - len) / 3;
        length_mod = length_mod * u32::try_from(eoc).unwrap_or(0) / 256;
    }
    length_mod.min(prosody.max_length_mod * speeds[0]) / 128
}

/// The length in ms of the main part of a vowel, before its length modifier.
///
/// Mirrors `LookupSpect()`: the standard length less the 45 ms its front part
/// accounts for.  `std_ms` is C's `fmt_params->std_length`, twice the phoneme's
/// `std_length`; `extra_ms` is added by a following `:`.
pub fn vowel_body_ms(std_ms: u32, extra_ms: u32) -> u32 {
    std_ms.saturating_sub(45).max(10) + extra_ms
}

// ---------------------------------------------------------------------------
// stress_to_espeak_level — map our code-stream stress codes → espeak level
// ---------------------------------------------------------------------------

/// C's `len_speeds`: the rate factors for a word's last, second-last and
/// earlier syllables (`SetSpeedFactors`, with the default voice's 256/238/232).
pub fn len_speeds(wpm: u32) -> [u32; 3] {
    let x = speed_multiplier(wpm);
    if x <= 7 {
        return [x, x - 1, x - 1];
    }
    [x, x * 238 / 256, x * 232 / 256]
}

/// `speed.wav_factor` / 256, for recorded sounds and the pauses before consonants.
pub fn wav_factor(wpm: u32) -> f64 {
    let x = speed_multiplier(wpm);
    let factor = if wpm >= 170 {
        110 + 150 * x / 128
    } else {
        128 + 128 * x / 130
    };
    f64::from(factor) / 256.0
}

/// `speed.pause_factor` / 256, for pause phonemes.
pub fn pause_factor(wpm: u32) -> f64 {
    f64::from((256 * speed_multiplier(wpm) / 115).max(12)) / 256.0
}

/// `speed.clause_pause_factor` / 256: as [`pause_factor`], but no less than 16.
pub fn clause_pause_factor(wpm: u32) -> f64 {
    pause_factor(wpm).max(16.0 / 256.0)
}

/// The speaking rate whose [`speed_duration_factor`] is closest to `factor`.
pub fn wpm_for_duration_factor(factor: f64) -> u32 {
    (80..=450)
        .min_by(|&a, &b| {
            let miss = |wpm| (speed_duration_factor(wpm) - factor).abs();
            miss(a).total_cmp(&miss(b))
        })
        .unwrap_or(175)
}

/// The `stress_lengths` index for a stress-marker code, with `CalcLengths`'
/// swap of diminished (1) and unstressed (0).  Primary stress is 6 here, as the
/// pitch pass has raised it (`PRIMARY_STRESSED`) by then.
pub fn stress_code_to_length_level(code: u8) -> u8 {
    use crate::phoneme::{
        PHON_STRESS_2, PHON_STRESS_3, PHON_STRESS_D, PHON_STRESS_P, PHON_STRESS_P2,
    };
    match code {
        PHON_STRESS_D => 1,
        PHON_STRESS_2 => 2,
        PHON_STRESS_3 => 3,
        PHON_STRESS_P | PHON_STRESS_P2 => 6,
        _ => 0,
    }
}

/// Map the stress-marker phoneme code (2–7) to the espeak-ng stress level (0–7).
///
/// In espeak-ng `phonemelist.c`, stress levels are:
///   0 = diminished (weakest)        → stress code 2 (%)
///   1 = secondary/unstressed        → stress code 2 (%)  [treated same as 0 here]
///   2 = secondary                   → stress code 3 (%%)
///   3 = secondary                   → stress code 4 (,)
///   4/5 = moderate                  → stress code 5
///   6 = primary                     → stress code 6 (ˈ)
///   7 = tonic primary               → stress code 7 (ˈˈ)
///
/// No preceding stress code → treat as unstressed (level 0, after swap → 1).
///
/// The CalcLengths "swap" (stress ≤1 → XOR 1) is applied here:
///   0 → 1  (swap: diminished treated as unstressed for length)
///   1 → 0  (swap: unstressed treated as diminished for length)
pub fn stress_code_to_level(code: u8) -> u8 {
    // Map code → raw level, then apply the 0↔1 swap
    let raw = match code {
        0 => 0, // no marker → diminished
        2 => 0, // % → unstressed/diminished
        3 => 2, // %% → secondary
        4 => 3, // , → tertiary
        5 => 4, // (moderate)
        6 => 6, // ˈ → primary
        7 => 7, // ˈˈ → tonic
        _ => 0,
    };
    // Apply 0↔1 swap (CalcLengths: `if stress <= 1: stress ^= 1`)
    if raw <= 1 {
        raw ^ 1
    } else {
        raw
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_stress_eoc_gives_long_vowel() {
        // oU at end of clause, stress=7 (tonic), std_length=110
        let length_mod = calc_vowel_length_mod(7, 1, 1, 0, true, 110, None, len_speeds(175));
        // C's own value for the last vowel of "hello" is 280.
        assert!(
            (270..=290).contains(&length_mod),
            "tonic EOC vowel should be long: {length_mod}"
        );
        assert_eq!(vowel_body_ms(220, 0), 175);
    }

    #[test]
    fn unstressed_vowel_is_short() {
        // @ schwa, stress=1 (after swap = 0), not last syllable, not EOC
        let length_mod = calc_vowel_length_mod(1, 7, 0, 1, false, 60, None, len_speeds(175));
        let samp = vowel_body_ms(120, 0) * 22050 / 1000 * length_mod / 256;
        // Should be < 100ms
        assert!(samp < 2205, "unstressed schwa should be short: {samp}");
    }

    #[test]
    fn stress_code_mapping() {
        assert_eq!(stress_code_to_level(6), 6); // primary
        assert_eq!(stress_code_to_level(7), 7); // tonic
        assert_eq!(stress_code_to_level(0), 1); // none → after swap
        assert_eq!(stress_code_to_level(2), 1); // % → after swap
    }
}
