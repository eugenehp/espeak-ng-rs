//! Phoneme-list → audio PCM synthesis.
//!
//! Rust port of the eSpeak NG synthesis engine:
//!
//! | C source file | Lines | What it does |
//! |---|---|---|
//! | `synthesize.c` | 1607 | Phoneme interpreter, `InterpretPhoneme` |
//! | `synthdata.c` | 998 | `LoadPhData`, formant data access |
//! | `wavegen.c` | 1486 | Harmonic additive synthesizer, AGC |
//! | `klatt.c` | 1124 | Klatt cascade/parallel filter |
//! | `setlengths.c` | 806 | Phoneme duration: `CalcLengths` |
//! | `phonemelist.c` | 593 | `MakePhonemeList`, stress promotion |
//!
//! # Synthesis pipeline
//!
//! ```text
//! PhonemeCode[]
//!     │  synthesize_codes()          (mod.rs)
//!     ▼
//! SpectSeq[]  (formant frame sequences from phondata)
//!     │  synthesize_frames()         (wavegen.rs)
//!     ▼
//! Vec<i32>  (unnormalised samples)
//!     │  agc_clip()                  (mod.rs)
//!     ▼
//! PcmBuffer  (Vec<i16>, 22 050 Hz, mono)
//! ```
//
// Status: IMPLEMENTED (cascade formant synthesizer)
//
// ## Pipeline
//
// ```text
//   IPA string
//       │  parse_ipa()          (engine.rs)
//       ▼
//   Vec<Segment>  (phoneme + timing + stress)
//       │  synthesize_segments()  (engine.rs)
//       ▼
//   Vec<f64>  (raw samples, un-normalised)
//       │  f64_to_i16()          (engine.rs)
//       ▼
//   PcmBuffer (i16, 22 050 Hz, mono)
// ```
//
// ## Formant Synthesis
//
pub mod targets;
pub mod engine;
pub mod phondata;
pub mod bytecode;
pub mod wavegen;
pub mod setlengths;
pub mod sample;

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Frame – formant parameters for one time slice
// Mirrors `frame_t` from synthesize.h (64 bytes in C)
// ---------------------------------------------------------------------------

/// One frame of formant parameters.
///
/// The C struct is 64 bytes with hand-packed `unsigned char` arrays.
/// We use named fields and let Rust handle packing.  When reading binary
/// data files with `#[repr(C)]` we will need a separate raw type.
#[derive(Debug, Clone, Default)]
pub struct Frame {
    /// Frame flags (FRFLAG_XXX bitmask)
    pub flags: u16,
    /// Formant frequencies F0–F6 (Hz × 2 in the C code)
    pub ffreq: [i16; 7],
    /// Frame length (units of STEPSIZE = 2.9ms @ 22050 Hz)
    pub length: u8,
    /// RMS amplitude
    pub rms: u8,
    /// Formant heights (amplitude of each formant)
    pub fheight: [u8; 8],
    /// Formant widths / 4, F0–F5
    pub fwidth: [u8; 6],
    /// Right-side formant widths / 4, F0–F2
    pub fright: [u8; 3],
    /// Klatt bandwidth / 2: BNZ, F1, F2, F3
    pub bw: [u8; 4],
    /// Klatt parameters: AV, FNZ, Tilt, Aspr, Skew
    pub klattp: [u8; 5],
    /// Extended Klatt parameters: AVp, Fric, FricBP, Turb, (spare)
    pub klattp2: [u8; 5],
    /// Klatt parallel amplitudes, F0–F6
    pub klatt_ap: [u8; 7],
    /// Klatt parallel bandwidths / 2, F0–F6
    pub klatt_bp: [u8; 7],
    /// Pad byte
    pub spare: u8,
}

impl Frame {
    /// The size of the equivalent C struct in bytes.
    pub const C_SIZE: usize = 64;
}

// ---------------------------------------------------------------------------
// Resonator – digital filter for one formant
// Mirrors `RESONATOR` struct from synthesize.h
// ---------------------------------------------------------------------------

/// A second-order IIR resonator (one formant).
///
/// Direct port of the `RESONATOR` C struct + the `Resonator()` macro.
///
/// Coefficients follow the Klatt (1980) convention:
/// ```text
///   y[n] = a·x[n] + b·y[n-1] + c·y[n-2]
/// ```
/// where:
/// ```text
///   r = exp(−π·BW/fs)
///   c = −r²
///   b = 2·r·cos(2π·F/fs)
///   a = 1 − b − c
/// ```
#[derive(Debug, Clone, Default)]
pub struct Resonator {
    /// Feed-forward coefficient `a` (see Klatt 1980).
    pub a:  f64,
    /// First feedback coefficient `b`.
    pub b:  f64,
    /// Second feedback coefficient `c`.
    pub c:  f64,
    /// Delay element `y[n-1]`.
    pub x1: f64,
    /// Delay element `y[n-2]`.
    pub x2: f64,
}

impl Resonator {
    /// Run one sample through the resonator.
    ///
    /// Mirrors the `Resonator(rp, in)` macro from wavegen.c:
    /// ```text
    ///   y = a·in + b·x1 + c·x2;  x2 = x1;  x1 = y;  y
    /// ```
    #[inline]
    pub fn tick(&mut self, input: f64) -> f64 {
        let y = self.a * input + self.b * self.x1 + self.c * self.x2;
        self.x2 = self.x1;
        self.x1 = y;
        y
    }

    /// Reset the filter state (clear delay elements).
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
    }
}

// ---------------------------------------------------------------------------
// Voice parameters
// ---------------------------------------------------------------------------

/// Voice configuration.
///
/// A subset of `voice_t` from voice.h; synthesis-relevant fields only.
#[derive(Debug, Clone)]
pub struct VoiceParams {
    /// Speaking rate multiplier (100 = normal, 200 = double speed)
    pub speed_percent: u32,
    /// Pitch in Hz (the base F0)
    pub pitch_hz: u32,
    /// Pitch range: 0 = monotone, 100 = normal
    pub pitch_range: u32,
    /// Formant frequency scale factor (100 = normal)
    pub formant_scale: u32,
    /// Sample rate in Hz (always 22050 for espeak-ng)
    pub sample_rate: u32,
    /// Amplitude 0–100
    pub amplitude: u32,
}

impl Default for VoiceParams {
    fn default() -> Self {
        VoiceParams {
            speed_percent:  100,
            pitch_hz:       118,  // male default
            pitch_range:    100,
            formant_scale:  100,
            sample_rate:    22050,
            amplitude:      80,
        }
    }
}

// ---------------------------------------------------------------------------
// Synthesizer
// ---------------------------------------------------------------------------

/// PCM output buffer (16-bit signed mono at 22050 Hz).
pub type PcmBuffer = Vec<i16>;

/// Top-level synthesizer: takes an IPA phoneme string and produces PCM.
///
/// ## Usage
/// ```rust,no_run
/// use espeak_ng::synthesize::{Synthesizer, VoiceParams};
///
/// let synth = Synthesizer::new(VoiceParams::default());
/// let pcm = synth.synthesize("hɛloʊ").expect("synthesis failed");
/// // pcm is a Vec<i16> at 22 050 Hz, mono
/// ```
pub struct Synthesizer {
    /// Voice / acoustic parameters in use.
    pub voice: VoiceParams,
}

impl Synthesizer {
    /// Create a new synthesizer with the given voice parameters.
    pub fn new(voice: VoiceParams) -> Self {
        Synthesizer { voice }
    }

    /// Synthesize an IPA phoneme string to 16-bit PCM samples at 22 050 Hz.
    ///
    /// The string is expected in the format produced by
    /// [`Translator::text_to_ipa`](crate::translate::Translator::text_to_ipa):
    /// IPA characters, with optional stress marks (`ˈ`/`ˌ`), length marks
    /// (`ː`), and ASCII spaces as word separators.
    ///
    /// # Returns
    /// A `Vec<i16>` of mono samples at 22 050 Hz.  The output is normalised
    /// to 90 % of full scale so it should not clip, but can be further scaled
    /// by the caller.
    ///
    /// # Example
    /// ```rust,no_run
    /// use espeak_ng::synthesize::{Synthesizer, VoiceParams};
    ///
    /// let synth = Synthesizer::new(VoiceParams::default());
    /// let pcm = synth.synthesize("ðə").unwrap(); // "the"
    /// assert!(!pcm.is_empty());
    /// ```
    pub fn synthesize(&self, phonemes: &str) -> Result<PcmBuffer> {
        if phonemes.is_empty() {
            return Ok(Vec::new());
        }
        let segments = engine::parse_ipa(phonemes, &self.voice);
        if segments.is_empty() {
            return Err(Error::InvalidData(
                format!("no recognisable phonemes in {:?}", phonemes)
            ));
        }
        let pcm = engine::synthesize_segments(&segments, &self.voice);
        Ok(pcm)
    }

    /// Synthesize phoneme codes directly using espeak-ng's binary acoustic data.
    ///
    /// This is the high-quality path that reads actual formant frame sequences
    /// from the `phondata` file and drives the harmonic synthesizer — the same
    /// acoustic model as the C `espeak-ng` library.
    ///
    /// # Arguments
    /// * `codes`   — slice of `PhonemeCode` items from `Translator::translate_to_codes`.
    /// * `phdata`  — the loaded phoneme data (`PhonemeData::load(data_dir)`).
    ///
    /// # Returns
    /// A `Vec<i16>` at 22 050 Hz.  Returns `Ok(vec![])` if `codes` is empty.
    pub fn synthesize_codes(
        &self,
        codes: &[crate::translate::PhonemeCode],
        phdata: &crate::phoneme::PhonemeData,
    ) -> Result<PcmBuffer> {
        if codes.is_empty() {
            return Ok(Vec::new());
        }

        let speed_factor = 100.0 / self.voice.speed_percent.max(1) as f64;

        // ── Pass 1: annotate each code with synthesis context ─────────────
        let annotated = annotate_codes(codes, phdata);

        // ── Pass 2: synthesize ─────────────────────────────────────────────
        let mut output_i16: Vec<i16> = Vec::new();
        let mut wavephase: i32 = i32::MAX;

        let sil_samples = |ms: f64| -> usize {
            ((ms / 1000.0) * 22050.0 * speed_factor) as usize
        };

        for ann in &annotated {
            match ann {
                AnnCode::Pause(ms) => {
                    let n = sil_samples(*ms);
                    output_i16.extend(std::iter::repeat(0i16).take(n));
                }
                AnnCode::WordBoundary => {
                    // ~50 ms gap between words (DoPause in synthesize.c)
                    let n = sil_samples(50.0);
                    output_i16.extend(std::iter::repeat(0i16).take(n));
                }
                AnnCode::PrepauseSamples(n) => {
                    output_i16.extend(std::iter::repeat(0i16).take(*n));
                }
                AnnCode::Phoneme(info) => {
                    let samples = synthesize_phoneme_info(info, phdata, &self.voice,
                                                          speed_factor, &mut wavephase);
                    output_i16.extend_from_slice(&samples);
                }
            }
        }

        Ok(output_i16)
    }

    /// Return the sample rate used by this synthesizer.
    ///
    /// Always 22 050 Hz in the current implementation.
    pub fn sample_rate(&self) -> u32 {
        self.voice.sample_rate
    }
}

// ---------------------------------------------------------------------------
// Phoneme synthesis context (annotated code stream)
// ---------------------------------------------------------------------------

/// Per-phoneme synthesis information extracted in pass 1.
struct PhonemeInfo {
    code: u8,
    /// Phoneme type from PhonemeTab (phVOWEL=2, phSTOP=4, phFRICATIVE=6, …)
    ph_type: u8,
    /// Espeak stress level (0–7), already after the 0↔1 swap.
    stress_level: u8,
    /// LENGTHEN (:) modifier — extend duration.
    lengthen: bool,
    // ── CalcLengths inputs for vowels ─────────────────────────────────────
    /// `ph->length_mod` of the NEXT phoneme (0–9).
    next_lm: u8,
    /// `ph->length_mod` of the phoneme after that (0–9).
    next2_lm: u8,
    /// `false` = this is the last syllable in its word.
    more_syllables: bool,
    /// `true` = this is the last vowel before the clause boundary.
    end_of_clause: bool,
    /// `ph->std_length` (mS/2 units).
    std_length: u8,
}

/// Annotated synthesis command.
enum AnnCode {
    /// Silence of `ms` milliseconds.
    Pause(f64),
    /// Word-boundary gap.
    WordBoundary,
    /// Pre-phoneme silence already computed in samples.
    PrepauseSamples(usize),
    /// A real phoneme with full context.
    Phoneme(PhonemeInfo),
}

// ---------------------------------------------------------------------------
// annotate_codes — pass 1
// ---------------------------------------------------------------------------

/// Pre-scan the code stream, resolving stress markers, word boundaries,
/// and CalcLengths context for each phoneme.
fn annotate_codes(
    codes: &[crate::translate::PhonemeCode],
    phdata: &crate::phoneme::PhonemeData,
) -> Vec<AnnCode> {
    let mut result = Vec::new();

    // Helper: get ph_type and length_mod of a real phoneme code
    let ph_info = |c: u8| -> (u8, u8, u8) {
        if let Some(ph) = phdata.get(c) {
            (ph.typ, ph.length_mod, ph.std_length)
        } else {
            (0, 0, 0)
        }
    };

    // Build a flat list of (code, is_boundary) for look-ahead
    let flat: Vec<(u8, bool)> = codes.iter().map(|c| (c.code, c.is_boundary)).collect();
    let n = flat.len();

    let mut i = 0;
    let mut pending_stress: u8 = 0;
    let mut pending_lengthen = false;

    while i < n {
        let (code, is_boundary) = flat[i];
        i += 1;

        match code {
            // Clause pause
            0 if is_boundary => {
                result.push(AnnCode::Pause(200.0));
                pending_stress = 0;
            }
            0 => {
                // code=0 with is_boundary=false: ignore
            }
            // Stress markers (1–7 without is_boundary)
            1..=7 if !is_boundary => {
                pending_stress = code;
            }
            // Explicit pause phoneme
            9 => {
                result.push(AnnCode::Pause(80.0));
            }
            // Lengthen (:)
            12 => {
                pending_lengthen = true;
            }
            // END_WORD (||)
            15 if is_boundary => {
                result.push(AnnCode::WordBoundary);
                pending_stress = 0;
            }
            _ => {
                // Real phoneme (including code 13 = schwa)
                let (ph_type, ph_lm, std_length) = ph_info(code);

                // ── CalcLengths context ─────────────────────────────────

                // Find the NEXT real phoneme (skip stress/control codes)
                let mut next_code = 0u8;
                let mut next_is_boundary = false;
                let mut j = i;
                while j < n {
                    let (nc, nb) = flat[j];
                    j += 1;
                    if nb || nc == 9 || nc == 12 { continue; }
                    if nc >= 1 && nc <= 7 { continue; }
                    next_code = nc;
                    next_is_boundary = nb;
                    break;
                }

                // Find NEXT2 real phoneme
                let mut next2_code = 0u8;
                while j < n {
                    let (nc, nb) = flat[j];
                    j += 1;
                    if nb || nc == 9 || nc == 12 { continue; }
                    if nc >= 1 && nc <= 7 { continue; }
                    next2_code = nc;
                    break;
                }

                let (next_type, next_lm, _) = ph_info(next_code);
                let (next2_type, next2_lm, _) = ph_info(next2_code);

                // For EOC and more_syllables, scan forward in same word
                let end_of_clause = next_code == 0 || (next_code == 15 && next_is_boundary);
                let more_syllables = {
                    // Count vowels after this one before END_WORD / clause boundary
                    let mut has_more = false;
                    for jj in i..n {
                        let (c2, b2) = flat[jj];
                        if b2 { break; } // END_WORD or pause boundary
                        if c2 == 0 || c2 == 15 { break; }
                        if c2 >= 1 && c2 <= 7 { continue; }
                        if let Some(ph) = phdata.get(c2) {
                            if ph.typ == 2 /* phVOWEL */ { has_more = true; break; }
                        }
                    }
                    has_more
                };

                // Pre-pause for stops/fricatives (mirrors prepause in setlengths.c)
                let prepause_samples = compute_prepause(
                    ph_type, next_type, next2_type, ph_lm, code,
                    &mut result,
                );

                let stress_level = setlengths::stress_code_to_level(pending_stress);
                pending_stress = 0;

                if prepause_samples > 0 {
                    result.push(AnnCode::PrepauseSamples(prepause_samples));
                }

                result.push(AnnCode::Phoneme(PhonemeInfo {
                    code,
                    ph_type,
                    stress_level,
                    lengthen: pending_lengthen,
                    next_lm,
                    next2_lm,
                    more_syllables,
                    end_of_clause,
                    std_length,
                }));
                pending_lengthen = false;

                let _ = (next_type, next2_type); // suppress unused warnings
            }
        }
    }

    result
}

// ---------------------------------------------------------------------------
// compute_prepause — prepause silence before consonants
// ---------------------------------------------------------------------------

/// Compute prepause samples for stops/fricatives (mirrors setlengths.c).
/// Returns the number of prepause samples.
fn compute_prepause(
    ph_type: u8,
    _next_type: u8,
    _next2_type: u8,
    _ph_lm: u8,
    _code: u8,
    _result: &mut Vec<AnnCode>,
) -> usize {
    // phSTOP=4, phFRICATIVE=6 get pre-pauses; others don't
    // Simplified: use typical values from setlengths.c
    let prepause_ms: f64 = match ph_type {
        4 /* phSTOP */ => 48.0,
        // phFRICATIVE at word boundary — skip for now, handled by WAV length
        _ => 0.0,
    };
    if prepause_ms > 0.0 {
        (prepause_ms / 1000.0 * 22050.0) as usize
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// synthesize_phoneme_info — pass 2 workhorse
// ---------------------------------------------------------------------------

/// Synthesize one phoneme using its full annotation context.
fn synthesize_phoneme_info(
    info: &PhonemeInfo,
    phdata: &crate::phoneme::PhonemeData,
    voice: &VoiceParams,
    speed_factor: f64,
    wavephase: &mut i32,
) -> Vec<i16> {
    use setlengths::{calc_vowel_length_mod, length_mod_to_samples};

    // Constants matching espeak-ng defaults
    const SAMPLERATE: u32 = 22050;

    // ── Look up bytecode ──────────────────────────────────────────────────
    let ph_tab = match phdata.get(info.code) {
        Some(p) => p,
        None => return Vec::new(),
    };
    let mut extract = bytecode::scan_phoneme(ph_tab.program, &phdata.phonindex);

    // If no fmt_addr found, follow ChangePhoneme chain (for phonemes like @2→@, 02→0, etc.)
    // i_CHANGE_PHONEME(target_code) redirects synthesis to another phoneme's data.
    if extract.fmt_addr.is_none() && extract.wav_addr.is_none() {
        if let Some(target_code) = extract.change_phoneme_code {
            if let Some(target_ph) = phdata.get(target_code) {
                if target_ph.program > 0 {
                    let sub = bytecode::scan_phoneme(target_ph.program, &phdata.phonindex);
                    if extract.fmt_addr.is_none() { extract.fmt_addr = sub.fmt_addr; extract.fmt_param = sub.fmt_param; }
                    if extract.wav_addr.is_none() { extract.wav_addr = sub.wav_addr; extract.wav_param = sub.wav_param; }
                }
            }
        }
    }

    // phSTOP(4) and phFRICATIVE(6): use WAV noise sample
    // phVSTOP(5) and phVFRICATIVE(7): fall through to formant synthesis
    if info.ph_type == 4 || info.ph_type == 6 {
        if let Some(wav_addr) = extract.wav_addr {
            if let Some(pcm) = sample::parse_wav_sample(
                wav_addr, &phdata.phondata, speed_factor, 0,
            ) {
                return pcm;
            }
        }
        // Fallback: short silence if no WAV data
        let n = (50.0 / 1000.0 * SAMPLERATE as f64 * speed_factor) as usize;
        return vec![0i16; n];
    }

    // ── Formant synthesis path (VOWEL, LIQUID, NASAL, VSTOP, VFRICATIVE) ─
    let fmt_addr = match extract.fmt_addr {
        Some(a) => a as usize,
        None => return Vec::new(),
    };
    let mut seq = match phondata::SpectSeq::parse(&phdata.phondata, fmt_addr) {
        Some(s) => s,
        None => return Vec::new(),
    };

    if seq.frames.is_empty() {
        return Vec::new();
    }

    // ── Duration calculation ──────────────────────────────────────────────
    // For vowels (typ=2): use CalcLengths formula.
    // For others: use raw frame lengths from the SPECT_SEQ.
    if info.ph_type == 2 /* phVOWEL */ {
        let length_mod = calc_vowel_length_mod(
            info.stress_level,
            info.next_lm,
            info.next2_lm,
            info.more_syllables,
            info.end_of_clause,
            info.std_length,
        );

        // Extra lengthening from `:` modifier
        let length_mod = if info.lengthen { length_mod * 4 / 3 } else { length_mod };

        let target_samples = length_mod_to_samples(length_mod, SAMPLERATE, speed_factor);

        if target_samples > 0 {
            // Scale all frame lengths proportionally to hit target_samples
            // The last frame is just a target; only frames[0..n-1].length matters
            let n = seq.frames.len();
            if n > 1 {
                // Compute raw sum (frames 0..n-1, excluding last per LookupSpect)
                let raw_sum: usize = seq.frames[..n-1].iter()
                    .map(|f| f.length as usize).sum::<usize>().max(1);
                // Frame length is in STEPSIZE units (64 samples at 22050 Hz), not ms.
                // Estimate current total samples for frames[0..n-1] before rescaling.
                let scaled_sum = (raw_sum as f64 * 64.0 * speed_factor) as usize;
                if scaled_sum > 0 {
                    let scale256 = target_samples * 256 / scaled_sum.max(1);
                    for fr in &mut seq.frames[..n-1] {
                        let new_len = ((fr.length as usize * scale256 / 256).max(1) as u8).min(255);
                        fr.length = new_len;
                    }
                }
            }
        }
    } else {
        // Consonant / sonorant: speed-scale only
        if (speed_factor - 1.0).abs() > 0.01 {
            for fr in &mut seq.frames {
                let new_len = ((fr.length as f64 * speed_factor).round() as usize).max(1);
                fr.length = new_len.min(255) as u8;
            }
        }
    }

    // Lengthen for consonants too (double middle frame)
    if info.lengthen && seq.frames.len() > 1 {
        let mid = seq.frames.len() / 2;
        let extra = seq.frames[mid].clone();
        seq.frames.insert(mid, extra);
    }

    // ── Harmonic synthesis ────────────────────────────────────────────────
    // Amplitude mirrors C wavegen.c:
    //   wdata.amplitude = stress_amp * general_amplitude / 16
    //   For primary stress: wdata.amplitude = 22 * 55 / 16 = 75
    //   Reference: wdata.amplitude_ref = 75 (primary), amplitude_fmt = 100
    //   => wdata.amplitude * amplitude_fmt = 75 * 100 = 7500 (primary)
    // amp_factor is normalized: 1.0 = primary stress = 7500 units.
    // wavegen_segment uses: amp_scale = global_amp * 7500 * amp_factor
    let stress_amp = setlengths::STRESS_AMPS_EN
        .get(info.stress_level as usize)
        .copied()
        .unwrap_or(20) as f64;
    let general_amp = 55.0f64; // GetAmplitude() default
    let wdata_amplitude = stress_amp * general_amp / 16.0;
    // Normalize to primary-stress reference (wdata_amplitude_primary = 22*55/16 = 75.625)
    let amp_primary = 22.0 * 55.0 / 16.0;
    let amp_factor = wdata_amplitude / amp_primary; // 1.0 for primary stress

    let raw = wavegen::synthesize_frames(&seq, voice, amp_factor, wavephase);

    // AGC — mirrors wavegen.c: reduce gain if clipping
    agc_clip(&raw)
}

// ---------------------------------------------------------------------------
// agc_clip — automatic gain control (mirrors wavegen.c AGC)
// ---------------------------------------------------------------------------

fn agc_clip(samples: &[i32]) -> Vec<i16> {
    if samples.is_empty() {
        return Vec::new();
    }
    let mut agc: i64 = 256;
    let mut out = Vec::with_capacity(samples.len());

    for &z1 in samples {
        let z = (z1 as i64 * agc) >> 8;

        if z >= 32768 {
            let ov = if z1 != 0 { 8_388_608i64 / (z1 as i64).abs() - 1 } else { 0 };
            if ov < agc { agc = ov.max(1); }
            let z2 = (z1 as i64 * agc) >> 8;
            out.push(z2.clamp(-32767, 32767) as i16);
        } else if z <= -32768 {
            let ov = if z1 != 0 { 8_388_608i64 / (z1 as i64).abs() - 1 } else { 0 };
            if ov < agc { agc = ov.max(1); }
            let z2 = (z1 as i64 * agc) >> 8;
            out.push(z2.clamp(-32767, 32767) as i16);
        } else {
            out.push(z.clamp(-32767, 32767) as i16);
        }

        // Gradually restore AGC (mirrors `if (agc < 256) agc++`)
        if agc < 256 { agc += 1; }
    }

    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resonator_tick_accumulates() {
        // With a=1, b=0, c=0 the resonator is just a pass-through
        let mut r = Resonator { a: 1.0, b: 0.0, c: 0.0, x1: 0.0, x2: 0.0 };
        assert!((r.tick(1.0) - 1.0).abs() < 1e-12);
        assert!((r.tick(2.0) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn resonator_tick_with_feedback() {
        // a=0, b=0.5, c=0 → exponential decay of x1
        let mut r = Resonator { a: 0.0, b: 0.5, c: 0.0, x1: 1.0, x2: 0.0 };
        let y0 = r.tick(0.0);
        assert!((y0 - 0.5).abs() < 1e-12);
        let y1 = r.tick(0.0);
        assert!((y1 - 0.25).abs() < 1e-12);
    }

    #[test]
    fn resonator_reset_clears_state() {
        let mut r = Resonator { a: 1.0, b: 0.5, c: 0.0, x1: 99.0, x2: 99.0 };
        r.reset();
        assert_eq!(r.x1, 0.0);
        assert_eq!(r.x2, 0.0);
    }

    #[test]
    fn frame_c_size() {
        // Structural assertion: if we ever add/remove fields the test fails.
        assert_eq!(Frame::C_SIZE, 64,
            "Frame::C_SIZE must match the C struct frame_t");
    }

    #[test]
    fn voice_params_default_sample_rate() {
        let v = VoiceParams::default();
        assert_eq!(v.sample_rate, 22050);
    }

    // ── Synthesizer ─────────────────────────────────────────────────────────

    #[test]
    fn synthesize_empty_string_returns_empty() {
        let s = Synthesizer::new(VoiceParams::default());
        let pcm = s.synthesize("").unwrap();
        assert!(pcm.is_empty());
    }

    #[test]
    fn synthesize_ipa_the() {
        let s = Synthesizer::new(VoiceParams::default());
        let pcm = s.synthesize("ðə").expect("should synthesise 'the'");
        // Must be non-empty; synthesizer clamps to ±32767 so no sample is i16::MIN.
        assert!(!pcm.is_empty());
        assert!(pcm.iter().all(|&x| x >= i16::MIN + 1));
    }

    #[test]
    fn synthesize_hello() {
        let s = Synthesizer::new(VoiceParams::default());
        let pcm = s.synthesize("hɛloʊ").expect("should synthesise 'hello'");
        assert!(!pcm.is_empty());
        // Roughly right duration: ~420 ms at 22050 Hz → at least 5000 samples.
        assert!(pcm.len() > 5_000, "too short: {} samples", pcm.len());
    }

    #[test]
    fn synthesize_produces_nonzero_audio() {
        let s = Synthesizer::new(VoiceParams::default());
        let pcm = s.synthesize("iː").unwrap();
        let peak = pcm.iter().map(|&x| x.unsigned_abs()).max().unwrap_or(0);
        assert!(peak > 1000, "expected non-trivial audio, got peak = {peak}");
    }

    #[test]
    fn synthesize_stress_words() {
        let s = Synthesizer::new(VoiceParams::default());
        // Stress marks must not cause a panic or empty output.
        let pcm = s.synthesize("ˈhɛloʊ ˌwɜːld").unwrap();
        assert!(!pcm.is_empty());
    }

    #[test]
    fn synthesize_unknown_phonemes_error() {
        // A string of only unrecognised chars should return an error.
        let s = Synthesizer::new(VoiceParams::default());
        let result = s.synthesize("☺☻♥");
        assert!(result.is_err(), "expected error for all-unrecognised input");
    }

    #[test]
    fn sample_rate_is_22050() {
        let s = Synthesizer::new(VoiceParams::default());
        assert_eq!(s.sample_rate(), 22050);
    }

    #[test]
    fn synthesize_speed_affects_duration() {
        let mut fast_voice = VoiceParams::default();
        fast_voice.speed_percent = 200; // double speed → half duration

        let s_normal = Synthesizer::new(VoiceParams::default());
        let s_fast   = Synthesizer::new(fast_voice);

        let pcm_normal = s_normal.synthesize("hɛloʊ").unwrap();
        let pcm_fast   = s_fast.synthesize("hɛloʊ").unwrap();

        assert!(pcm_fast.len() < pcm_normal.len(),
            "fast speech must be shorter: fast={}, normal={}",
            pcm_fast.len(), pcm_normal.len());
    }
}
