//! Phoneme duration calculation — simplified port of `setlengths.c`.

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

/// stress_lengths2[stress] — stress-level length multiplier, units 1/128.
/// Indexed 0..7.  After the CalcLengths swap (stress 0↔1), stress=0 means
/// "diminished" and stress=1 means "unstressed".
pub const STRESS_LENGTHS_EN: [u32; 8] = [182, 140, 220, 220, 220, 240, 260, 280];

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
    100, 120, 100, 105, 100, 110, 110, 100,  95, 100, // a  ← next2
    105, 120, 105, 110, 125, 130, 135, 115, 125, 100, // ,
    105, 120,  75, 100,  75, 105, 120,  85,  75, 100, // t
    105, 120,  85, 105,  95, 115, 120, 100,  95, 100, // s
    110, 120,  95, 105, 100, 115, 120, 100, 100, 100, // n
    105, 120, 100, 105,  95, 115, 120, 110,  95, 100, // d
    105, 120, 100, 105, 105, 122, 125, 110, 105, 100, // z
    105, 120, 100, 105, 105, 122, 125, 110, 105, 100, // r
    105, 120,  95, 105, 100, 115, 120, 110, 100, 100, // N
    100, 120, 100, 100, 100, 100, 100, 100, 100, 100, // default
];

/// length_mods_en0 — for vowels that ARE the last syllable in their word.
pub const LENGTH_MODS_EN0: [u8; 100] = [
//   a    ,    t    s    n    d    z    r    N    (←next)
    100, 150, 100, 105, 110, 115, 110, 110, 110, 100, // a
    105, 150, 105, 110, 125, 135, 140, 115, 135, 100, // ,
    105, 150,  90, 105,  90, 122, 135, 100,  90, 100, // t
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
/// This mirrors the `phVOWEL` case in `CalcLengths()` from setlengths.c.
///
/// # Parameters
/// * `stress`        — phoneme stress level (0-7).  **Already after the
///                     0↔1 swap** that CalcLengths applies.
/// * `next_lm`       — `ph->length_mod` of the **next** phoneme (0-9).
/// * `next2_lm`      — `ph->length_mod` of the phoneme after that (0-9).
/// * `more_syllables`— `false` if this is the **last syllable** of its word.
/// * `end_of_clause` — `true` if this is the last syllable before the
///                     clause boundary.
/// * `std_length`    — `ph->std_length` of this vowel (in mS/2 units).
///
/// Returns `length_mod`: a 256-based scale factor (256 = no change).
pub fn calc_vowel_length_mod(
    stress: u8,
    next_lm: u8,
    next2_lm: u8,
    more_syllables: bool,
    end_of_clause: bool,
    std_length: u8,
) -> u32 {
    let stress = (stress as usize).min(7);

    // Choose table based on syllable position
    let table = if more_syllables { &LENGTH_MODS_EN } else { &LENGTH_MODS_EN0 };
    let n2 = (next2_lm as usize).min(9);
    let n1 = (next_lm as usize).min(9);
    let base_mod = table[n2 * 10 + n1] as u32;

    // Apply stress-length factor
    let stress_len = STRESS_LENGTHS_EN[stress];
    let mut length_mod = base_mod * stress_len / 128;

    // Tonic syllable: add constant component (lengthen_tonic)
    if stress >= 7 {
        length_mod += LENGTHEN_TONIC_EN;
    }

    // End-of-clause lengthening
    if end_of_clause {
        let len = (std_length as u32) * 2; // std_length is in mS/2 units
        let eoc_num = 280u32.saturating_sub(len);
        length_mod = length_mod * (256 + eoc_num / 3) / 256;
    }

    // Clamp: minimum 8, maximum 500 × speed_factor (at default speed = 1.0)
    length_mod.clamp(8, 500)
}

/// Convert a vowel length_mod to total PCM samples.
///
/// This mirrors the formula in `LookupSpect()` + `DoSpect2()`:
///   `total_samples = (length_mod - 45) * samplerate/1000 * length_mod/256`
///
/// The result is then scaled by `speed_factor` (100/speed_percent).
pub fn length_mod_to_samples(length_mod: u32, samplerate: u32, speed_factor: f64) -> usize {
    if length_mod <= 45 {
        return 0;
    }
    let length_std = length_mod - 45;
    // total = length_std_ms * samplerate/1000 * length_mod/256
    let total = (length_std as f64 / 1000.0)
        * samplerate as f64
        * (length_mod as f64 / 256.0)
        * speed_factor;
    total.round() as usize
}

// ---------------------------------------------------------------------------
// stress_to_espeak_level — map our code-stream stress codes → espeak level
// ---------------------------------------------------------------------------

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
        0 => 0,   // no marker → diminished
        2 => 0,   // % → unstressed/diminished  
        3 => 2,   // %% → secondary
        4 => 3,   // , → tertiary
        5 => 4,   // (moderate)
        6 => 6,   // ˈ → primary
        7 => 7,   // ˈˈ → tonic
        _ => 0,
    };
    // Apply 0↔1 swap (CalcLengths: `if stress <= 1: stress ^= 1`)
    if raw <= 1 { raw ^ 1 } else { raw }
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
        let length_mod = calc_vowel_length_mod(7, 1, 1, false, true, 110);
        // Expected: >= 300 (several hundred ms)
        assert!(length_mod >= 300, "tonic EOC vowel should be long: {length_mod}");
        let samp = length_mod_to_samples(length_mod, 22050, 1.0);
        assert!(samp > 5000, "tonic EOC vowel > 200ms: {samp}");
    }

    #[test]
    fn unstressed_vowel_is_short() {
        // @ schwa, stress=1 (after swap = 0), not last syllable, not EOC
        let length_mod = calc_vowel_length_mod(1, 7, 0, true, false, 60);
        let samp = length_mod_to_samples(length_mod, 22050, 1.0);
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
