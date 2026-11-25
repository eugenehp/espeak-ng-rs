//! The length, loudness and lead-in pause of each phoneme: `CalcLengths` in
//! `setlengths.c`.

use super::lang_prosody::LangProsody;
use super::setlengths::{vowel_length_mod, VowelContext, MAXAMP_EOC_EN};
use super::AnnCode;
use crate::bytes::Bounded;
use crate::phoneme::{
    PhonemeData, PH_FRICATIVE, PH_LIQUID, PH_NASAL, PH_PAUSE, PH_STOP, PH_VFRICATIVE, PH_VOWEL,
    PH_VSTOP,
};

const PH_SIBILANT: u32 = 1 << 5;
const PH_VOICELESS: u32 = 1 << 3;
const PH_BRKAFTER: u32 = 1 << 14;
const PH_LENGTHENSTOP: u32 = 1 << 22;
const PH_NOPAUSE: u32 = 1 << 24;
pub(super) const PH_PREVOICE: u32 = 1 << 25;

/// What [`calc_lengths`] works out for a phoneme.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Timing {
    /// Length modifier, 256 = 100%.
    pub length: u32,
    /// `stress_amps` value, 22 = a primary stress.
    pub amp: u8,
    /// Pause before it, in nominal ms.
    pub prepause: u32,
}

/// The neighbour of a phoneme, as far as its timing depends on it.
#[derive(Clone, Copy, Default)]
struct Neighbour {
    typ: u8,
    flags: u32,
    newword: bool,
    length: u32,
}

/// The timing of every entry of `annotated` that is a phoneme.
pub(super) fn calc_lengths(
    annotated: &[AnnCode],
    phdata: &PhonemeData,
    prosody: &LangProsody,
    speeds: [u32; 3],
) -> Vec<Timing> {
    let mut out = vec![Timing::default(); annotated.len()];
    let flags = |code: u8| phdata.get(code).map_or(0, |ph| ph.phflags);
    let items: Vec<(usize, &AnnCode)> = annotated
        .iter()
        .enumerate()
        .filter(|(_, ann)| !matches!(ann, AnnCode::WordBoundary | AnnCode::Embedded(_)))
        .collect();
    let neighbour = |k: usize| match items.get(k) {
        Some((_, AnnCode::Phoneme(info))) => Neighbour {
            typ: info.ph_type,
            flags: flags(info.code),
            newword: info.this_wordstart,
            length: 0,
        },
        _ => Neighbour {
            typ: PH_PAUSE,
            ..Neighbour::default()
        },
    };

    let mut prev = Neighbour::default();
    // The last vowel's: how many syllables followed it, and its stress.
    let mut more_syllables = 0;
    let mut stressed = false;
    for (k, &(ix, ann)) in items.iter().enumerate() {
        let AnnCode::Phoneme(info) = ann else {
            prev = Neighbour::default();
            continue;
        };
        let this = neighbour(k);
        let next = neighbour(k + 1);
        stressed &= !this.newword;
        let mut timing = Timing {
            length: 128,
            amp: prosody.stress_amps[0],
            prepause: 0,
        };
        match info.ph_type {
            PH_STOP => {
                // A consonant before a vowel has that vowel's stress, any other
                // the last one's in its word.
                let stressed = match items.get(k + 1) {
                    Some((_, AnnCode::Phoneme(next))) if next.ph_type == PH_VOWEL => {
                        next.length_stress >= 6
                    }
                    _ => stressed,
                };
                timing.prepause = match prev.typ {
                    PH_FRICATIVE => 25,
                    PH_STOP => 60,
                    _ if more_syllables > 0 || !stressed => 48,
                    _ => 60,
                };
                if this.flags & PH_LENGTHENSTOP != 0 {
                    timing.prepause += 30;
                }
                if info.lengthen {
                    timing.prepause += prosody.long_stop;
                }
            }
            PH_FRICATIVE | PH_VFRICATIVE => {
                if this.newword && !(prev.typ == PH_VOWEL && this.flags & PH_NOPAUSE != 0) {
                    timing.prepause = 15;
                }
                if next.typ == PH_PAUSE && prev.typ == PH_NASAL && this.flags & PH_VOICELESS == 0 {
                    timing.prepause = 25;
                }
                if prev.flags & PH_BRKAFTER != 0 {
                    timing.prepause = 30;
                }
                timing.length = 256;
                if this.flags & PH_SIBILANT != 0 && next.typ == PH_STOP && !next.newword {
                    timing.length = if prev.typ == PH_VOWEL { 200 } else { 150 };
                }
                if info.ph_type == PH_VFRICATIVE && matches!(prev.typ, PH_VOWEL | PH_LIQUID) {
                    timing.length = (255 + prev.length) / 2;
                }
            }
            PH_VSTOP => {
                if matches!(prev.typ, PH_VFRICATIVE | PH_FRICATIVE | PH_LIQUID)
                    || prev.flags & PH_SIBILANT != 0
                {
                    timing.prepause = 30;
                }
                if matches!(next.typ, PH_VOWEL | PH_LIQUID) {
                    timing.prepause = match prev.typ {
                        // A murmur links it to the vowel instead.
                        PH_VOWEL => 0,
                        // (Shortened by the pause before it, which is longer.)
                        PH_PAUSE => 0,
                        PH_LIQUID if !this.newword => 20,
                        PH_NASAL if !this.newword => 12,
                        PH_STOP if !this.newword && prev.flags & PH_VOICELESS == 0 => 0,
                        _ => 40,
                    };
                }
            }
            PH_LIQUID | PH_NASAL => {
                timing.length = 256;
                if this.newword {
                    match prev.typ {
                        PH_LIQUID => timing.prepause = 25,
                        PH_VOWEL if this.flags & PH_NOPAUSE == 0 => timing.prepause = 12,
                        _ => {}
                    }
                }
                if next.typ != PH_VOWEL && matches!(prev.typ, PH_VOWEL | PH_LIQUID) {
                    timing.length = if info.ph_type == PH_LIQUID {
                        speeds[0]
                    } else {
                        prev.length
                    };
                    match next.typ {
                        PH_VSTOP => timing.length = timing.length * 160 / 100,
                        PH_VFRICATIVE => timing.length = timing.length * 120 / 100,
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        let vowel = if info.ph_type == PH_VOWEL {
            Some(info.vowel_length())
        } else {
            info.sonorant_vowel
        };
        if let Some(vowel) = vowel {
            let next_newword = if info.ph_type == PH_VOWEL {
                next.newword
            } else {
                neighbour(k + 2).newword
            };
            let length = vowel_length_mod(
                &VowelContext {
                    stress: vowel.stress,
                    next_lm: vowel.next_lm,
                    next2_lm: vowel.next2_lm,
                    more_syllables: vowel.more_syllables,
                    end_of_clause: vowel.end_of_clause,
                    std_length: vowel.std_length,
                    next_newword,
                    tone_percent: vowel.tone_percent,
                },
                prosody,
                speeds,
            );
            let mut amp = prosody.stress_amps.at(usize::from(vowel.stress.min(7)));
            if vowel.end_of_clause && vowel.next_lm == 1 {
                amp = amp.min(MAXAMP_EOC_EN);
            }
            if info.ph_type == PH_VOWEL {
                timing.length = length;
                // `pre_sonorant`
                if matches!(prev.typ, PH_LIQUID | PH_NASAL) {
                    amp = amp.saturating_sub(1);
                }
            } else {
                timing.length = length;
                if info.ph_type != PH_LIQUID {
                    amp = amp.min(18);
                }
            }
            timing.amp = amp;
        }
        if info.ph_type == PH_VOWEL {
            more_syllables = info.more_syllables;
            stressed = info.length_stress >= 6;
        }
        prev = Neighbour {
            length: timing.length,
            ..this
        };
        out.put(ix, timing);
    }
    out
}
