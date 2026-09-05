//! Clause intonation — a port of upstream `intonation.c`'s `CalcPitches()`.
//!
//! espeak builds a **syllable table** for the clause (one entry per syllable —
//! i.e. per vowel — with its stress level), then assigns each syllable a pitch
//! *span* (`pitch1` → `pitch2`) and an *envelope* (one of the 128-sample curves
//! in [`super::envelopes`]) describing how F0 moves across it.  The clause is
//! divided into four parts:
//!
//! * **pre-head** — syllables before the first primary stress: a linear glide
//!   between the tune's `prehead_start` / `prehead_end`;
//! * **head/body** — up to the tonic syllable: the pitch steps down (or up) once
//!   per primary stress, with an overflow table when the steps run out;
//! * **nucleus** — the tonic syllable, which carries the tune's characteristic
//!   contour (fall for a statement, fall-rise for a comma, rise for a question);
//! * **tail** — a linear glide after the tonic syllable.
//!
//! Which tune is used depends on the clause type (`.`, `,`, `?`, `!`) and the
//! language's `intonation` group, read from the `intonations` data file.
//!
//! This replaces the port's earlier approximation — a single falling
//! declination across the whole utterance with a rising tail for questions —
//! which had no notion of syllables, stress or tunes.

use super::envelopes::ENVELOPE_DATA;

/// Stress levels, as `intonation.c` uses them.
const SECONDARY: u8 = 3;
const PRIMARY: u8 = 4;
const PRIMARY_STRESSED: u8 = 6;
const PRIMARY_LAST: u8 = 7;

/// `syl->flags` bits.
pub const SYL_RISE: u8 = 1;
pub const SYL_EMPHASIS: u8 = 2;
pub const SYL_END_CLAUSE: u8 = 4;

/// Minimum pitch drop for a syllable, indexed by stress.
const MIN_DROP: [i32; 8] = [6, 7, 9, 9, 20, 20, 20, 25];
/// Pitch change during the main part of the clause, indexed by stress.
const DROPS_0: [i32; 8] = [9, 9, 16, 16, 16, 23, 55, 32];

// Overflow tables: 64ths of the body pitch range.
const OFLOW: [i8; 5] = [0, 40, 24, 8, 0];
const OFLOW_EMF: [i8; 5] = [10, 52, 32, 20, 10];
const OFLOW_LESS: [i8; 5] = [6, 38, 24, 14, 4];

/// Envelope indices (`PITCH*` in `synthesize.h`).
const PITCH_FALL: u8 = 0;
const PITCH_RISE: u8 = 2;
const PITCH_FRISE: u8 = 4; // and 5 = rising variant
const PITCH_FRISE2: u8 = 6;

const T_EMPH: u8 = 1;

/// One entry of `tone_head_table[]` — the clause body's pitch behaviour.
#[derive(Clone, Copy)]
struct ToneHead {
    pre_start: u8,
    pre_end: u8,
    body_start: u8,
    body_end: u8,
    body_max_steps: u8,
    body_lower_u: i32,
    n_overflow: usize,
    overflow: &'static [i8; 5],
}

/// One entry of `tone_nucleus_table[]` — the tonic syllable and tail.
#[derive(Clone, Copy)]
struct ToneNucleus {
    pitch_env0: u8,
    tonic_max0: i32,
    tonic_min0: i32,
    pitch_env1: u8,
    tonic_max1: i32,
    tonic_min1: i32,
    tail_start: u8,
    tail_end: u8,
    flags: u8,
}

const TONE_HEAD_TABLE: [ToneHead; 13] = [
    ToneHead { pre_start: 46, pre_end: 57, body_start: 78, body_end: 50, body_max_steps: 3, body_lower_u: 7, n_overflow: 5, overflow: &OFLOW },      // 0 statement
    ToneHead { pre_start: 46, pre_end: 57, body_start: 78, body_end: 46, body_max_steps: 3, body_lower_u: 7, n_overflow: 5, overflow: &OFLOW },      // 1 comma
    ToneHead { pre_start: 46, pre_end: 57, body_start: 78, body_end: 46, body_max_steps: 3, body_lower_u: 7, n_overflow: 5, overflow: &OFLOW },      // 2 question
    ToneHead { pre_start: 46, pre_end: 57, body_start: 90, body_end: 50, body_max_steps: 3, body_lower_u: 9, n_overflow: 5, overflow: &OFLOW_EMF },  // 3 exclamation
    ToneHead { pre_start: 46, pre_end: 57, body_start: 78, body_end: 50, body_max_steps: 3, body_lower_u: 7, n_overflow: 5, overflow: &OFLOW },      // 4 statement, emphatic
    ToneHead { pre_start: 46, pre_end: 57, body_start: 74, body_end: 55, body_max_steps: 4, body_lower_u: 7, n_overflow: 5, overflow: &OFLOW_LESS }, // 5 statement, less intonation
    ToneHead { pre_start: 46, pre_end: 57, body_start: 74, body_end: 55, body_max_steps: 4, body_lower_u: 7, n_overflow: 5, overflow: &OFLOW_LESS }, // 6 comma, less intonation
    ToneHead { pre_start: 46, pre_end: 57, body_start: 74, body_end: 55, body_max_steps: 4, body_lower_u: 7, n_overflow: 5, overflow: &OFLOW_LESS }, // 7 comma, less rise
    ToneHead { pre_start: 46, pre_end: 57, body_start: 78, body_end: 50, body_max_steps: 3, body_lower_u: 7, n_overflow: 5, overflow: &OFLOW },      // 8 raises at end
    ToneHead { pre_start: 46, pre_end: 57, body_start: 78, body_end: 46, body_max_steps: 3, body_lower_u: 7, n_overflow: 5, overflow: &OFLOW },      // 9 comma
    ToneHead { pre_start: 46, pre_end: 57, body_start: 78, body_end: 50, body_max_steps: 3, body_lower_u: 7, n_overflow: 5, overflow: &OFLOW },      // 10 question
    ToneHead { pre_start: 34, pre_end: 41, body_start: 41, body_end: 32, body_max_steps: 3, body_lower_u: 7, n_overflow: 5, overflow: &OFLOW_LESS }, // 11 test
    ToneHead { pre_start: 46, pre_end: 57, body_start: 55, body_end: 50, body_max_steps: 3, body_lower_u: 7, n_overflow: 5, overflow: &OFLOW_LESS }, // 12 test
];

const TONE_NUCLEUS_TABLE: [ToneNucleus; 13] = [
    ToneNucleus { pitch_env0: PITCH_FALL,   tonic_max0: 64, tonic_min0:  8, pitch_env1: PITCH_FALL,   tonic_max1: 70, tonic_min1: 18, tail_start: 24, tail_end: 12, flags: 0 },
    ToneNucleus { pitch_env0: PITCH_FRISE,  tonic_max0: 80, tonic_min0: 18, pitch_env1: PITCH_FRISE2, tonic_max1: 78, tonic_min1: 22, tail_start: 34, tail_end: 52, flags: 0 },
    ToneNucleus { pitch_env0: PITCH_FRISE,  tonic_max0: 88, tonic_min0: 22, pitch_env1: PITCH_FRISE2, tonic_max1: 82, tonic_min1: 22, tail_start: 34, tail_end: 64, flags: 0 },
    ToneNucleus { pitch_env0: PITCH_FALL,   tonic_max0: 92, tonic_min0:  8, pitch_env1: PITCH_FALL,   tonic_max1: 92, tonic_min1: 80, tail_start: 76, tail_end:  8, flags: T_EMPH },
    ToneNucleus { pitch_env0: PITCH_FALL,   tonic_max0: 86, tonic_min0:  4, pitch_env1: PITCH_FALL,   tonic_max1: 94, tonic_min1: 66, tail_start: 34, tail_end: 10, flags: 0 },
    ToneNucleus { pitch_env0: PITCH_FALL,   tonic_max0: 62, tonic_min0: 10, pitch_env1: PITCH_FALL,   tonic_max1: 62, tonic_min1: 20, tail_start: 28, tail_end: 16, flags: 0 },
    ToneNucleus { pitch_env0: PITCH_FRISE,  tonic_max0: 68, tonic_min0: 18, pitch_env1: PITCH_FRISE2, tonic_max1: 68, tonic_min1: 22, tail_start: 30, tail_end: 44, flags: 0 },
    ToneNucleus { pitch_env0: PITCH_FRISE2, tonic_max0: 64, tonic_min0: 16, pitch_env1: PITCH_FALL,   tonic_max1: 66, tonic_min1: 32, tail_start: 32, tail_end: 18, flags: 0 },
    ToneNucleus { pitch_env0: PITCH_RISE,   tonic_max0: 68, tonic_min0: 46, pitch_env1: PITCH_FALL,   tonic_max1: 42, tonic_min1: 32, tail_start: 46, tail_end: 58, flags: 0 },
    ToneNucleus { pitch_env0: PITCH_FRISE,  tonic_max0: 78, tonic_min0: 24, pitch_env1: PITCH_FRISE2, tonic_max1: 72, tonic_min1: 22, tail_start: 42, tail_end: 52, flags: 0 },
    ToneNucleus { pitch_env0: PITCH_FRISE,  tonic_max0: 88, tonic_min0: 34, pitch_env1: PITCH_FALL,   tonic_max1: 64, tonic_min1: 32, tail_start: 46, tail_end: 82, flags: 0 },
    ToneNucleus { pitch_env0: PITCH_FALL,   tonic_max0: 56, tonic_min0: 12, pitch_env1: PITCH_FALL,   tonic_max1: 56, tonic_min1: 20, tail_start: 24, tail_end: 12, flags: 0 },
    ToneNucleus { pitch_env0: PITCH_FALL,   tonic_max0: 70, tonic_min0: 18, pitch_env1: PITCH_FALL,   tonic_max1: 70, tonic_min1: 24, tail_start: 32, tail_end: 20, flags: 0 },
];

/// A tune from the `intonations` data file (68-byte `TUNE` records).
#[derive(Clone, Debug, Default)]
pub struct Tune {
    pub name: String,
    pub head_extend: [i8; 8],
    pub prehead_start: u8,
    pub prehead_end: u8,
    pub stressed_env: u8,
    pub stressed_drop: i32,
    pub onset: u8,
    pub head_start: u8,
    pub head_end: u8,
    pub head_last: u8,
    pub head_max_steps: i32,
    pub n_head_extend: usize,
    pub unstr_start: [i8; 3],
    pub unstr_end: [i8; 3],
    pub nucleus0_env: u8,
    pub nucleus0_max: i32,
    pub nucleus0_min: i32,
    pub nucleus1_env: u8,
    pub nucleus1_max: i32,
    pub nucleus1_min: i32,
    pub tail_start: u8,
    pub tail_end: u8,
}

/// Parse the `intonations` file into its `TUNE` records (68 bytes each).
pub fn parse_tunes(data: &[u8]) -> Vec<Tune> {
    data.chunks_exact(68)
        .map(|t| Tune {
            name: t[..12]
                .iter()
                .take_while(|&&b| b != 0)
                .map(|&b| b as char)
                .collect(),
            head_extend: std::array::from_fn(|i| t[16 + i] as i8),
            prehead_start: t[24],
            prehead_end: t[25],
            stressed_env: t[26],
            stressed_drop: t[27] as i32,
            onset: t[30],
            head_start: t[31],
            head_end: t[32],
            head_last: t[33],
            head_max_steps: t[34] as i32,
            n_head_extend: t[35] as usize,
            unstr_start: std::array::from_fn(|i| t[36 + i] as i8),
            unstr_end: std::array::from_fn(|i| t[39 + i] as i8),
            nucleus0_env: t[42],
            nucleus0_max: t[43] as i32,
            nucleus0_min: t[44] as i32,
            nucleus1_env: t[45],
            nucleus1_max: t[46] as i32,
            nucleus1_min: t[47] as i32,
            tail_start: t[48],
            tail_end: t[49],
        })
        .collect()
}

/// One syllable of the clause: its stress level, and the pitch span and
/// envelope assigned to it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Syllable {
    pub stress: u8,
    pub env: u8,
    pub pitch1: i32,
    pub pitch2: i32,
    pub flags: u8,
}

impl Syllable {
    pub fn new(stress: u8) -> Self {
        Syllable { stress, env: PITCH_FALL, ..Default::default() }
    }

    /// The syllable's pitch value (espeak's 0–254 scale) at `t` ∈ [0,1] through
    /// the syllable.
    ///
    /// The envelope is 128 samples scaled across the span: C orders the two
    /// endpoints low-first (`SetPitch2` swaps them) so envelope 0 is the bottom
    /// of the span and 255 the top — `env_fall` therefore starts at 255 and ends
    /// at 0, giving a falling contour.
    pub fn pitch_at(&self, t: f64) -> f64 {
        let env = ENVELOPE_DATA[(self.env as usize).min(ENVELOPE_DATA.len() - 1)];
        let ix = ((t.clamp(0.0, 1.0) * 127.0).round() as usize).min(127);
        let frac = env[ix] as f64 / 255.0;
        let lo = self.pitch1.min(self.pitch2) as f64;
        let hi = self.pitch1.max(self.pitch2) as f64;
        lo + (hi - lo) * frac
    }
}

/// `set_pitch()` — give a syllable a pitch span of `drop` below `base`.
fn set_pitch(syl: &mut Syllable, base: i32, drop: i32) {
    let base = base.max(0);
    let mut pitch2 = base;
    let mut drop = drop;
    if drop < 0 {
        syl.flags |= SYL_RISE;
        drop = -drop;
    }
    let mut pitch1 = pitch2 + drop;
    if pitch1 < 0 {
        pitch1 = 0;
    }
    pitch1 = pitch1.min(254);
    pitch2 = pitch2.min(254);
    syl.pitch1 = pitch1;
    syl.pitch2 = pitch2;
}

/// Where the tonic syllable falls, and how many syllables precede/follow it.
struct Layout {
    number_pre: usize,
    number_tail: usize,
    tone_posn: usize,
    /// Penultimate stressed syllable — where the tonic moves under
    /// `OPTION_EMPHASIZE_PENULTIMATE`, which this port doesn't expose yet.
    #[allow(dead_code)]
    tone_posn2: usize,
}

/// `count_pitch_vowels()`.
fn count_pitch_vowels(syls: &mut [Syllable], start: usize, end: usize, clause_end: usize, no_tonic: bool) -> Layout {
    let mut max_stress = 0u8;
    let mut max_stress_posn = start;
    let mut max_stress_posn2 = start;
    let mut number_pre: Option<usize> = None;
    let mut last_primary: Option<usize> = None;

    for ix in start..end {
        let stress = syls[ix].stress;
        if stress >= max_stress {
            max_stress_posn2 = if stress > max_stress { ix } else { max_stress_posn };
            max_stress_posn = ix;
            max_stress = stress;
        }
        if stress >= PRIMARY {
            number_pre.get_or_insert(ix - start);
            last_primary = Some(ix);
        }
    }

    let number_pre = number_pre.unwrap_or(end);
    let number_tail = end.saturating_sub(max_stress_posn + 1);
    let (mut tone_posn, mut tone_posn2) = (max_stress_posn, max_stress_posn2);

    if no_tonic {
        tone_posn = end;
        tone_posn2 = end;
    } else if let Some(lp) = last_primary {
        if end == clause_end {
            syls[lp].stress = PRIMARY_LAST;
        }
    } else if tone_posn < syls.len() {
        syls[tone_posn].stress = PRIMARY_LAST;
    }

    Layout { number_pre, number_tail, tone_posn, tone_posn2 }
}

/// `count_increments()` — primary stresses up to the tonic syllable.
fn count_increments(syls: &[Syllable], mut ix: usize, end_ix: usize, min_stress: u8) -> i32 {
    let mut count = 0;
    while ix < end_ix {
        let stress = syls[ix].stress;
        ix += 1;
        if stress >= PRIMARY_LAST {
            break;
        }
        if stress >= min_stress {
            count += 1;
        }
    }
    count
}

/// `CountUnstressed()`.
fn count_unstressed(syls: &[Syllable], start: usize, end: usize, limit: u8) -> i32 {
    let mut ix = start;
    while ix <= end && ix < syls.len() {
        if syls[ix].stress >= limit {
            break;
        }
        ix += 1;
    }
    (ix - start) as i32
}

/// `SetPitchGradient()` — a linear pitch change over a run of syllables, used
/// for the pre-head, the unstressed syllables in the body, and the tail.
fn set_pitch_gradient(syls: &mut [Syllable], start_ix: usize, end_ix: usize, start_pitch: i32, end_pitch: i32) {
    let n_increments = end_ix as i32 - start_ix as i32;
    if n_increments <= 0 {
        return;
    }
    let mut increment = (end_pitch - start_pitch) * 256;
    if n_increments > 1 {
        increment /= n_increments;
    }
    let mut pitch = start_pitch * 256;

    for ix in start_ix..end_ix.min(syls.len()) {
        let stress = syls[ix].stress as usize;
        if increment > 0 {
            set_pitch(&mut syls[ix], pitch / 256, -(increment / 256));
            pitch += increment;
        } else {
            let mut drop = -(increment / 256);
            if drop < MIN_DROP[stress.min(7)] {
                drop = MIN_DROP[stress.min(7)];
            }
            pitch += increment;
            if drop > 18 {
                drop = 18;
            }
            set_pitch(&mut syls[ix], pitch / 256, drop);
        }
    }
}

/// `SetHeadIntonation()` — the clause body, stepping the pitch once per primary
/// stress and falling back to the tune's overflow table when the steps run out.
fn set_head_intonation(syls: &mut [Syllable], tune: &Tune, mut syl_ix: usize, end_ix: usize) -> usize {
    let pitch_range = (tune.head_end as i32 - tune.head_start as i32) * 256;
    let pitch_range_abs = pitch_range.abs();
    let drops = &DROPS_0;

    let mut pitch = 0i32;
    let mut increment = 0i32;
    let mut n_steps = 0i32;
    let mut stage = if tune.onset == 255 { 1usize } else { 0 };
    let mut initial = true;
    let mut overflow_ix = 0usize;
    let mut n_unstressed = 0i32;
    let mut unstressed_ix = 0i32;
    let mut used_onset = false;
    let secondary = 2u8;

    let mut head_final = end_ix;
    if tune.head_last != 255 {
        for ix in (syl_ix..end_ix).rev() {
            if syls[ix].stress >= 4 {
                head_final = ix;
                break;
            }
        }
    }

    while syl_ix < end_ix && syl_ix < syls.len() {
        let stress = syls[syl_ix].stress;

        if initial || stress >= 4 {
            if initial || stress == 5 {
                initial = false;
                overflow_ix = 0;
                if tune.onset == 255 {
                    n_steps = count_increments(syls, syl_ix, head_final, 4);
                    pitch = tune.head_start as i32 * 256;
                } else {
                    n_steps = count_increments(syls, syl_ix + 1, head_final, 4);
                    pitch = tune.onset as i32 * 256;
                    used_onset = true;
                }
                if n_steps > tune.head_max_steps {
                    n_steps = tune.head_max_steps;
                }
                increment = if n_steps > 1 { pitch_range / (n_steps - 1) } else { 0 };
            } else if syl_ix == head_final {
                pitch = tune.head_last as i32 * 256;
                stage = 2;
            } else if used_onset {
                stage = 1;
                used_onset = false;
                pitch = tune.head_start as i32 * 256;
                n_steps += 1;
            } else if n_steps > 0 {
                pitch += increment;
            } else {
                let extend = tune.head_extend[overflow_ix.min(7)] as i32;
                pitch = (tune.head_end as i32 * 256) + (pitch_range_abs * extend) / 64;
                overflow_ix += 1;
                if tune.n_head_extend == 0 || overflow_ix >= tune.n_head_extend {
                    overflow_ix = 0;
                }
            }
            n_steps -= 1;
        }

        if stress >= PRIMARY {
            n_unstressed = count_unstressed(syls, syl_ix + 1, end_ix, secondary);
            unstressed_ix = 0;
            syls[syl_ix].stress = PRIMARY_STRESSED;
            syls[syl_ix].env = tune.stressed_env;
            set_pitch(&mut syls[syl_ix], pitch / 256, tune.stressed_drop);
        } else if stress >= secondary {
            n_unstressed = count_unstressed(syls, syl_ix + 1, end_ix, secondary);
            unstressed_ix = 0;
            set_pitch(&mut syls[syl_ix], pitch / 256, drops[stress as usize]);
        } else {
            let unstressed_inc = if n_unstressed > 1 {
                (tune.unstr_end[stage] as i32 - tune.unstr_start[stage] as i32) / (n_unstressed - 1)
            } else {
                0
            };
            let base = pitch / 256 + tune.unstr_start[stage] as i32 + unstressed_inc * unstressed_ix;
            set_pitch(&mut syls[syl_ix], base, drops[stress as usize]);
            unstressed_ix += 1;
        }

        syl_ix += 1;
    }
    syl_ix
}

/// `calc_pitch_segment()` — the body when using the built-in tone tables.
#[allow(clippy::too_many_arguments)]
fn calc_pitch_segment(
    syls: &mut [Syllable],
    mut ix: usize,
    end_ix: usize,
    th: &ToneHead,
    min_stress: u8,
    continuing: bool,
) -> usize {
    const CONTINUE_TAB: [i8; 5] = [-26, 32, 20, 8, 0];
    let drops = &DROPS_0;
    let pitch_range = (th.body_end as i32 - th.body_start as i32) * 256;
    let pitch_range_abs = pitch_range.abs();

    let mut pitch = 0i32;
    let mut increment;
    let mut n_steps = 0i32;
    let mut overflow = 0usize;
    let (mut initial, n_overflow, mut overflow_tab): (bool, usize, &[i8; 5]) = if continuing {
        increment = pitch_range / (th.body_max_steps as i32 - 1).max(1);
        (false, 5, &CONTINUE_TAB)
    } else {
        increment = 0;
        (true, th.n_overflow, th.overflow)
    };

    while ix < end_ix && ix < syls.len() {
        let stress = syls[ix].stress;

        if initial || stress >= min_stress {
            if initial || stress == 5 {
                initial = false;
                overflow = 0;
                n_steps = count_increments(syls, ix, end_ix, min_stress);
                if n_steps > th.body_max_steps as i32 {
                    n_steps = th.body_max_steps as i32;
                }
                increment = if n_steps > 1 { pitch_range / (n_steps - 1) } else { 0 };
                pitch = th.body_start as i32 * 256;
            } else if n_steps > 0 {
                pitch += increment;
            } else {
                pitch = (th.body_end as i32 * 256)
                    + (pitch_range_abs * overflow_tab[overflow.min(4)] as i32) / 64;
                overflow += 1;
                if overflow >= n_overflow {
                    overflow = 0;
                    overflow_tab = th.overflow;
                }
            }
            n_steps -= 1;
        }

        if stress >= PRIMARY {
            syls[ix].stress = PRIMARY_STRESSED;
            set_pitch(&mut syls[ix], pitch / 256, drops[stress as usize]);
        } else if stress >= SECONDARY {
            set_pitch(&mut syls[ix], pitch / 256, drops[stress as usize]);
        } else {
            // unstressed: drop the pitch if the previous syllable was stressed
            let prev_stressed = ix > 0 && (syls[ix - 1].stress & 0x3f) >= SECONDARY;
            let base = pitch / 256 - if prev_stressed { th.body_lower_u } else { 0 };
            set_pitch(&mut syls[ix], base, drops[stress as usize]);
        }

        ix += 1;
    }
    ix
}

/// `calc_pitches2()` — assign pitches using a `TUNE` from the data file.
fn calc_pitches_tune(syls: &mut [Syllable], start: usize, end: usize, tune: &Tune, layout: &Layout, no_tonic: bool) -> u8 {
    let mut ix = start;
    set_pitch_gradient(syls, ix, ix + layout.number_pre, tune.prehead_start as i32, tune.prehead_end as i32);
    ix += layout.number_pre;

    ix = set_head_intonation(syls, tune, ix, layout.tone_posn);
    if no_tonic {
        return 0;
    }

    let tone_pitch_env = if layout.number_tail == 0 {
        let drop = tune.nucleus0_max - tune.nucleus0_min;
        if ix < syls.len() {
            set_pitch(&mut syls[ix], tune.nucleus0_min, drop);
        }
        tune.nucleus0_env
    } else {
        let drop = tune.nucleus1_max - tune.nucleus1_min;
        if ix < syls.len() {
            set_pitch(&mut syls[ix], tune.nucleus1_min, drop);
        }
        tune.nucleus1_env
    };
    ix += 1;

    if layout.tone_posn < syls.len() {
        syls[layout.tone_posn].env = tone_pitch_env;
        if syls[layout.tone_posn].stress == PRIMARY {
            syls[layout.tone_posn].stress = PRIMARY_STRESSED;
        }
    }

    set_pitch_gradient(syls, ix, end, tune.tail_start as i32, tune.tail_end as i32);
    tone_pitch_env
}

/// `calc_pitches()` with `control != 0` — the built-in tone head/nucleus tables.
fn calc_pitches_table(syls: &mut [Syllable], start: usize, end: usize, tune_number: usize, layout: &Layout, no_tonic: bool) -> u8 {
    let th = &TONE_HEAD_TABLE[tune_number.min(TONE_HEAD_TABLE.len() - 1)];
    let tn = &TONE_NUCLEUS_TABLE[tune_number.min(TONE_NUCLEUS_TABLE.len() - 1)];
    let continuing = start > 0;

    let mut ix = start;
    set_pitch_gradient(syls, ix, ix + layout.number_pre, th.pre_start as i32, th.pre_end as i32);
    ix += layout.number_pre;

    ix = calc_pitch_segment(syls, ix, layout.tone_posn, th, PRIMARY, continuing);
    if no_tonic {
        return 0;
    }

    if tn.flags & T_EMPH != 0 && ix < syls.len() {
        syls[ix].flags |= SYL_EMPHASIS;
    }

    let tone_pitch_env = if layout.number_tail == 0 {
        if ix < syls.len() {
            set_pitch(&mut syls[ix], tn.tonic_min0, tn.tonic_max0 - tn.tonic_min0);
        }
        tn.pitch_env0
    } else {
        if ix < syls.len() {
            set_pitch(&mut syls[ix], tn.tonic_min1, tn.tonic_max1 - tn.tonic_min1);
        }
        tn.pitch_env1
    };
    ix += 1;

    if layout.tone_posn < syls.len() {
        syls[layout.tone_posn].env = tone_pitch_env;
        if syls[layout.tone_posn].stress == PRIMARY {
            syls[layout.tone_posn].stress = PRIMARY_STRESSED;
        }
    }

    set_pitch_gradient(syls, ix, end, tn.tail_start as i32, tn.tail_end as i32);
    tone_pitch_env
}

/// Assign a pitch span and envelope to every syllable of a clause.
///
/// `clause_type`: 0 = `.`, 1 = `,`, 2 = `?`, 3 = `!`, 4 = none (an incomplete
/// clause, e.g. after an abbreviation — no tonic syllable).  `tune` selects a
/// tune from the `intonations` file; without one the built-in tone tables are
/// used, which is what C does when the language has no `intonation` group.
pub fn calc_pitches(syls: &mut [Syllable], clause_type: usize, tune: Option<&Tune>) {
    if syls.is_empty() {
        return;
    }
    let no_tonic = clause_type == 4;
    let end = syls.len();
    let layout = count_pitch_vowels(syls, 0, end, end, no_tonic);

    match tune {
        Some(t) => {
            calc_pitches_tune(syls, 0, end, t, &layout, no_tonic);
        }
        None => {
            calc_pitches_table(syls, 0, end, clause_type.min(3), &layout, no_tonic);
        }
    }
}

/// `punctuation_to_tone[][]` — which tone table a clause type uses, per
/// `intonation_group`.  Columns are `.` `,` `?` `!` none emphatic.
pub const PUNCT_TO_TONE: [[u8; 6]; 8] = [
    [0, 1, 2, 3, 0, 4],
    [0, 1, 2, 3, 0, 4],
    [5, 6, 2, 3, 0, 4],
    [5, 7, 1, 3, 0, 4],
    [8, 9, 10, 3, 0, 0],
    [8, 8, 10, 3, 0, 0],
    [11, 11, 11, 11, 0, 0],
    [12, 12, 12, 12, 0, 0],
];

/// The tone table for a clause type under an `intonation_group`.
pub fn tone_for_clause(intonation_group: usize, clause_type: usize) -> usize {
    let group = if intonation_group >= PUNCT_TO_TONE.len() { 1 } else { intonation_group };
    PUNCT_TO_TONE[group][clause_type.min(5)] as usize
}

/// `pitch_adjust_tab[]` — how the user's pitch parameter (0–100, default 50)
/// scales the voice's base pitch; 128 is unity.
pub static PITCH_ADJUST_TAB: [u8; 102] = [
    64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79,
    80, 81, 82, 83, 84, 86, 87, 88, 89, 91, 92, 93, 94, 96, 97, 98,
    100, 101, 103, 104, 105, 107, 108, 110, 111, 113, 115, 116, 118, 119, 121, 123,
    124, 126, 128, 130, 132, 133, 135, 137, 139, 141, 143, 145, 147, 149, 151, 153,
    155, 158, 160, 162, 164, 167, 169, 171, 174, 176, 179, 181, 184, 186, 189, 191,
    194, 197, 199, 202, 205, 208, 211, 214, 217, 220, 223, 226, 229, 232, 236, 239,
    242, 246, 249, 252, 254, 255,
];

/// Convert a syllable pitch value (espeak's 0–254 scale) to Hz, using the same
/// arithmetic as `SetPitch2()`: the voice's `pitch <base> <range>` line gives
/// `pitch_base = (base - 9) << 12` and `pitch_range = (range - base) * 108`,
/// and a pitch value contributes `value * range / 2` on top of the base.
pub fn pitch_to_hz(value: f64, voice_pitch_base_hz: f64, voice_pitch_range: f64) -> f64 {
    voice_pitch_base_hz + (value * voice_pitch_range) / 2.0 / 4096.0
}

/// The voice's base pitch in Hz for a user pitch setting (0–100, default 50).
///
/// C: `pitch_base = (pitch1 - 9) << 12` from the voice's `pitch <pitch1>
/// <pitch2>` line (default `82 118` → 73 Hz), scaled by `pitch_adjust_tab`.
pub fn base_pitch_hz(voice_pitch1: f64, user_pitch: u32) -> f64 {
    let ix = (user_pitch as usize).min(PITCH_ADJUST_TAB.len() - 1);
    (voice_pitch1 - 9.0) * PITCH_ADJUST_TAB[ix] as f64 / 128.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syllables(stresses: &[u8]) -> Vec<Syllable> {
        stresses.iter().map(|&s| Syllable::new(s)).collect()
    }

    #[test]
    fn statement_falls_across_the_clause() {
        // "hello world": unstressed, primary, unstressed, primary.
        let mut syls = syllables(&[1, 4, 1, 4]);
        calc_pitches(&mut syls, 0, None);
        // The tonic (last primary) ends lower than the body starts.
        assert!(syls[1].pitch2 > syls[3].pitch2, "{syls:?}");
        // Every syllable got a span.
        assert!(syls.iter().all(|s| s.pitch1 != 0 || s.pitch2 != 0));
    }

    #[test]
    fn question_ends_higher_than_a_statement() {
        let mut statement = syllables(&[1, 4, 1, 4]);
        let mut question = syllables(&[1, 4, 1, 4]);
        calc_pitches(&mut statement, 0, None);
        calc_pitches(&mut question, 2, None);
        let last = statement.len() - 1;
        assert!(
            question[last].pitch1 > statement[last].pitch1,
            "question {:?} vs statement {:?}",
            question[last],
            statement[last]
        );
    }

    #[test]
    fn the_tonic_syllable_gets_the_tune_envelope() {
        let mut syls = syllables(&[1, 4, 1, 4]);
        calc_pitches(&mut syls, 2, None); // question → fall-rise nucleus
        assert_eq!(syls[3].env, PITCH_FRISE, "tonic carries the question envelope");
        assert_eq!(syls[0].env, PITCH_FALL, "others keep the default");
    }

    #[test]
    fn an_incomplete_clause_gets_no_nucleus_envelope() {
        // `clause_type == 4` (an abbreviation such as "Dr.") has no tonic
        // syllable, so no syllable receives the tune's nucleus contour — the
        // body still runs, which is why the stresses are promoted as usual.
        let mut syls = syllables(&[1, 4, 1, 4]);
        calc_pitches(&mut syls, 4, None);
        assert!(syls.iter().all(|s| s.env == PITCH_FALL), "no nucleus envelope: {syls:?}");
    }

    #[test]
    fn envelope_interpolates_between_the_span() {
        let syl = Syllable { stress: 4, env: PITCH_FALL, pitch1: 40, pitch2: 20, flags: 0 };
        let start = syl.pitch_at(0.0);
        let end = syl.pitch_at(1.0);
        assert!(start > end, "a falling envelope starts high: {start} → {end}");
        assert!((20.0..=40.0).contains(&start) && (20.0..=40.0).contains(&end));
    }

    #[test]
    fn tunes_parse_from_the_data_file() {
        let mut raw = vec![0u8; 68];
        raw[..4].copy_from_slice(b"s1\0\0");
        raw[24] = 46; // prehead_start
        raw[25] = 57; // prehead_end
        raw[31] = 78; // head_start
        raw[32] = 50; // head_end
        raw[42] = PITCH_FALL;
        raw[43] = 64; // nucleus0_max
        raw[44] = 8; // nucleus0_min
        let tunes = parse_tunes(&raw);
        assert_eq!(tunes.len(), 1);
        assert_eq!(tunes[0].name, "s1");
        assert_eq!(tunes[0].prehead_start, 46);
        assert_eq!(tunes[0].head_start, 78);
        assert_eq!(tunes[0].nucleus0_max, 64);
    }
}
