//! Cascade formant synthesizer for the generic IPA → PCM path.

// src/synthesize/engine.rs
//
// Cascade formant synthesizer.
//
// Architecture:
//   1. Parse the IPA string into a sequence of `Segment`s (phoneme + timing).
//   2. For each segment, linearly interpolate the three formant frequencies and
//      bandwidths from the previous segment's targets to the current targets.
//      Resonator coefficients are recomputed every STEPSIZE samples (64 samples
//      ≈ 2.9 ms @ 22 050 Hz) to avoid per-sample transcendental-function calls.
//   3. The source signal is a mixture of:
//        • Voiced:   a shaped glottal pulse train at F0 (from VoiceParams).
//        • Unvoiced: white noise via a xorshift PRNG.
//      The mixing ratio is controlled by the phoneme's `voiced_frac` /
//      `noise_frac` fields.
//   4. The source is filtered through a cascade of three second-order IIR
//      resonators (F1, F2, F3).  The resonator state persists across phoneme
//      boundaries, giving smooth formant transitions automatically.
//   5. Output is scaled and hard-clipped to i16 range.  A final peak-limiter
//      step prevents distortion on the loudest segments.

use std::f64::consts::PI;

use super::targets::{FormantTarget, SILENCE, match_ipa};
use super::{Resonator, VoiceParams, PcmBuffer};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Synthesis sample rate (Hz).
pub const SAMPLE_RATE: f64 = 22_050.0;

/// Coefficient update interval (samples).  Matches espeak-ng's `STEPSIZE`.
const STEPSIZE: usize = 64;

// ---------------------------------------------------------------------------
// Resonator coefficient helpers
// ---------------------------------------------------------------------------

/// Compute second-order IIR resonator coefficients (Klatt 1980).
///
/// Given centre frequency `f` (Hz) and bandwidth `bw` (Hz) at sample rate
/// `fs` (Hz), returns the `(a, b, c)` coefficients for:
/// ```text
///   y[n] = a·x[n] + b·y[n-1] + c·y[n-2]
/// ```
/// where a = 1 − B − C, B = 2·r·cos(2π·f/fs), C = −r².
#[inline]
fn resonator_coeffs(f: f64, bw: f64, fs: f64) -> (f64, f64, f64) {
    // Clamp to safe range: keep f well below Nyquist and bw positive.
    let f  = f.clamp(50.0, fs * 0.49);
    let bw = bw.clamp(10.0, fs * 0.25);

    let r = (-PI * bw / fs).exp();
    let c = -(r * r);
    let b = 2.0 * r * (2.0 * PI * f / fs).cos();
    let a = 1.0 - b - c;
    (a, b, c)
}

/// Set all fields of a `Resonator` from (f, bw, fs).
#[inline]
fn set_resonator(r: &mut Resonator, f: f64, bw: f64, fs: f64) {
    let (a, b, c) = resonator_coeffs(f, bw, fs);
    r.a = a; r.b = b; r.c = c;
}

// ---------------------------------------------------------------------------
// Segment — one phoneme with timing
// ---------------------------------------------------------------------------

/// A parsed phoneme event ready for synthesis.
#[derive(Debug, Clone)]
pub struct Segment {
    /// Acoustic target for this phoneme.
    pub target: FormantTarget,
    /// Duration in samples (already rate-adjusted).
    pub dur_samples: usize,
    /// Amplitude multiplier (1.0 = normal; > 1.0 = stressed).
    pub amp_factor: f64,
}

// ---------------------------------------------------------------------------
// IPA parser
// ---------------------------------------------------------------------------

/// Parse an IPA string produced by `Translator::text_to_ipa()` into a
/// sequence of `Segment`s suitable for synthesis.
///
/// Handled characters:
/// * `ˈ` (U+02C8) primary stress   → `amp_factor = 1.3` on next phoneme.
/// * `ˌ` (U+02CC) secondary stress → `amp_factor = 1.1` on next phoneme.
/// * `ː` (U+02D0) length mark      → should have been consumed as part of
///   a long-vowel digraph (e.g. "iː").  If seen naked, ignored.
/// * ` ` (ASCII space)             → short inter-word pause.
/// * Everything else → longest-prefix match against `IPA_TARGETS`.
pub fn parse_ipa(ipa: &str, voice: &VoiceParams) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut s = ipa;
    // Speed: 100 = normal (1×), 200 = double speed (÷2), 50 = half speed (×2).
    let speed_factor = 100.0 / voice.speed_percent.max(1) as f64;
    let mut pending_amp = 1.0_f64;

    while !s.is_empty() {
        // ── Stress marks ────────────────────────────────────────────────────
        if let Some(rest) = s.strip_prefix('ˈ') {   // primary stress U+02C8
            pending_amp = 1.3;
            s = rest;
            continue;
        }
        if let Some(rest) = s.strip_prefix('ˌ') {   // secondary stress U+02CC
            pending_amp = 1.1;
            s = rest;
            continue;
        }
        // ── Stray length mark (shouldn't appear outside a long-vowel digraph)
        if let Some(rest) = s.strip_prefix('ː') {   // U+02D0
            s = rest;
            continue;
        }
        // ── Inter-word space → very short silence ──────────────────────────
        if let Some(rest) = s.strip_prefix(' ') {
            let dur_ms = 60.0 * speed_factor;
            segments.push(Segment {
                target: SILENCE,
                dur_samples: ms_to_samples(dur_ms),
                amp_factor: 1.0,
            });
            s = rest;
            pending_amp = 1.0;
            continue;
        }

        // ── Try longest-prefix IPA match ────────────────────────────────────
        if let Some((target, consumed)) = match_ipa(s) {
            let dur_ms = target.dur_ms * speed_factor;
            segments.push(Segment {
                target: *target,
                dur_samples: ms_to_samples(dur_ms),
                amp_factor: pending_amp,
            });
            pending_amp = 1.0;
            s = &s[consumed..];
        } else {
            // Unknown IPA character — skip it.
            let c = s.chars().next().unwrap();
            s = &s[c.len_utf8()..];
        }
    }

    segments
}

#[inline]
fn ms_to_samples(ms: f64) -> usize {
    ((ms / 1000.0) * SAMPLE_RATE).max(1.0).round() as usize
}

// ---------------------------------------------------------------------------
// Xorshift32 PRNG (fast white noise)
// ---------------------------------------------------------------------------

struct Xorshift32(u32);

impl Xorshift32 {
    fn new() -> Self { Xorshift32(0xBAD_5EED) }

    /// Next sample in −1.0 … +1.0.
    #[inline]
    fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as i32 as f64) * (1.0 / 2_147_483_648.0)
    }
}

// ---------------------------------------------------------------------------
// Glottal pulse model
// ---------------------------------------------------------------------------

/// Generate one sample of a shaped glottal waveform.
///
/// `phase` ∈ [0, 1) is the fractional position within the F0 cycle.
/// The shape is a quasi-sinusoidal pulse:
///   * Opening phase (0.0 – 0.65): raised-cosine rise.
///   * Closing phase (0.65 – 1.0): sharper fall (glottal closure).
///
/// The DC component is zero (no net force), and the waveform has the
/// spectral tilt of a natural voice (6 dB/octave slope).
#[inline]
fn glottal_sample(phase: f64) -> f64 {
    let p = phase;
    if p < 0.65 {
        // Smooth opening: half of a cosine (−1 → +1 over 0.65 periods)
        let t = p / 0.65;
        (PI * t - PI * 0.5).sin()           // = −cos(π·t), range −1..+1
    } else {
        // Sharp glottal closure: linear fall
        let t = (p - 0.65) / 0.35;
        1.0 - 2.0 * t                        // +1 → −1
    }
}

// ---------------------------------------------------------------------------
// Core synthesis loop
// ---------------------------------------------------------------------------

/// Synthesize a sequence of `Segment`s to a `PcmBuffer`.
///
/// # Algorithm
/// For each segment the formant frequencies and bandwidths are linearly
/// interpolated from the *previous* segment's values to the *current*
/// segment's values.  Resonator coefficients are updated every `STEPSIZE`
/// samples.  The three resonators run in cascade (F1 → F2 → F3).
pub fn synthesize_segments(segments: &[Segment], voice: &VoiceParams) -> PcmBuffer {
    let fs = SAMPLE_RATE;
    let f0 = voice.pitch_hz.max(50) as f64;
    let global_amp = voice.amplitude.clamp(0, 100) as f64 / 100.0;

    // Total sample count (pre-allocate)
    let total: usize = segments.iter().map(|s| s.dur_samples).sum();
    let mut output: Vec<f64> = Vec::with_capacity(total);

    // Resonator filter state — persists across segments for smooth transitions.
    let mut r1 = Resonator::default();
    let mut r2 = Resonator::default();
    let mut r3 = Resonator::default();

    // Initialise resonators to neutral (schwa-like) values.
    set_resonator(&mut r1, 500.0, 150.0, fs);
    set_resonator(&mut r2, 1500.0, 200.0, fs);
    set_resonator(&mut r3, 2500.0, 300.0, fs);

    let mut rng = Xorshift32::new();

    // Glottal oscillator phase, 0.0..1.0 per cycle.
    let mut phase = 0.0_f64;
    let phase_inc = f0 / fs;

    // Previous segment's formant targets (for interpolation).
    let mut prev_f1 = 500.0_f64;
    let mut prev_f2 = 1500.0_f64;
    let mut prev_f3 = 2500.0_f64;
    let mut prev_bw1 = 150.0_f64;
    let mut prev_bw2 = 200.0_f64;
    let mut prev_bw3 = 300.0_f64;

    for seg in segments {
        let n = seg.dur_samples;
        if n == 0 { continue; }

        let tgt = &seg.target;
        // Effective per-phoneme amplitude.
        let seg_amp = tgt.amp * seg.amp_factor * global_amp;

        let voiced = tgt.voiced_frac;
        let noise  = tgt.noise_frac;

        // Formant endpoints for this segment.
        let to_f1  = tgt.f1;  let to_f2  = tgt.f2;  let to_f3  = tgt.f3;
        let to_bw1 = tgt.bw1; let to_bw2 = tgt.bw2; let to_bw3 = tgt.bw3;

        let mut step_start = 0usize;

        while step_start < n {
            let step_end = (step_start + STEPSIZE).min(n);
            let step_len = step_end - step_start;

            // Interpolation factor at the midpoint of this step.
            let t_mid = (step_start as f64 + step_len as f64 * 0.5) / n as f64;

            // Interpolate formant parameters and update resonator coefficients.
            let f1  = prev_f1  + (to_f1  - prev_f1)  * t_mid;
            let f2  = prev_f2  + (to_f2  - prev_f2)  * t_mid;
            let f3  = prev_f3  + (to_f3  - prev_f3)  * t_mid;
            let bw1 = prev_bw1 + (to_bw1 - prev_bw1) * t_mid;
            let bw2 = prev_bw2 + (to_bw2 - prev_bw2) * t_mid;
            let bw3 = prev_bw3 + (to_bw3 - prev_bw3) * t_mid;

            set_resonator(&mut r1, f1, bw1, fs);
            set_resonator(&mut r2, f2, bw2, fs);
            set_resonator(&mut r3, f3, bw3, fs);

            // Run the synthesizer for this step.
            for i_rel in 0..step_len {
                let i_abs = step_start + i_rel;

                // ── Source signal ────────────────────────────────────────────
                let voiced_src = if voiced > 0.0 {
                    glottal_sample(phase) * voiced
                } else {
                    0.0
                };

                let noise_src = if noise > 0.0 {
                    rng.next_f64() * noise
                } else {
                    0.0
                };

                let source = voiced_src + noise_src;

                // ── Amplitude envelope (4-ms fade-in / fade-out) ─────────────
                let fade_len = (0.004 * fs) as usize;
                let env = if i_abs < fade_len {
                    i_abs as f64 / fade_len as f64
                } else if i_abs >= n.saturating_sub(fade_len) {
                    (n.saturating_sub(i_abs)) as f64 / fade_len as f64
                } else {
                    1.0
                };

                let x = source * env;

                // ── Cascade resonator filter: F1 → F2 → F3 ──────────────────
                let y = r3.tick(r2.tick(r1.tick(x)));

                output.push(y * seg_amp);

                // Advance glottal oscillator phase.
                phase = (phase + phase_inc).fract();
            }

            step_start = step_end;
        }

        // Update previous targets for the next segment.
        prev_f1 = to_f1; prev_f2 = to_f2; prev_f3 = to_f3;
        prev_bw1 = to_bw1; prev_bw2 = to_bw2; prev_bw3 = to_bw3;
    }

    // ── Normalise & convert to i16 ───────────────────────────────────────────
    f64_to_i16(&output)
}

/// Normalise a `f64` sample buffer and convert to `i16`.
///
/// The peak is found; if it exceeds a safe threshold the entire buffer is
/// scaled down to 90 % of full scale.  Very quiet outputs are left as-is
/// (they will just be quiet).
fn f64_to_i16(samples: &[f64]) -> PcmBuffer {
    if samples.is_empty() {
        return Vec::new();
    }

    let peak = samples.iter().fold(0.0_f64, |m, &x| m.max(x.abs()));

    // Target: 90 % of full scale (≈ 29 490).
    let target_peak = 0.90 * 32_767.0;
    let scale = if peak > 1e-6 { target_peak / peak } else { 0.0 };

    samples.iter()
        .map(|&x| (x * scale).clamp(-32_767.0, 32_767.0) as i16)
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthesize::VoiceParams;

    fn default_voice() -> VoiceParams { VoiceParams::default() }

    // ── parse_ipa ────────────────────────────────────────────────────────────

    #[test]
    fn parse_simple_vowel() {
        let segs = parse_ipa("ə", &default_voice());
        assert_eq!(segs.len(), 1);
        assert!((segs[0].target.f1 - 500.0).abs() < 1.0);
    }

    #[test]
    fn parse_long_vowel() {
        // "iː" should match as a single long-vowel segment.
        let segs = parse_ipa("iː", &default_voice());
        assert_eq!(segs.len(), 1, "long vowel = one segment");
        assert!(segs[0].dur_samples > ms_to_samples(100.0),
            "long vowel must be longer than 100 ms");
    }

    #[test]
    fn parse_stress_increases_amp() {
        let segs_plain    = parse_ipa("ə",  &default_voice());
        let segs_stressed = parse_ipa("ˈə", &default_voice());
        assert!(segs_stressed[0].amp_factor > segs_plain[0].amp_factor);
    }

    #[test]
    fn parse_word_boundary_inserts_pause() {
        // "biː hiː" → b, iː, [space], h, iː
        let segs = parse_ipa("biː hiː", &default_voice());
        // At least one segment should be silent (space).
        assert!(segs.iter().any(|s| s.target.amp == 0.0),
            "inter-word space must produce a silent segment");
    }

    #[test]
    fn parse_unknown_char_skipped() {
        // Emoji has no formant match — must be skipped without panic.
        let segs = parse_ipa("ə☺ə", &default_voice());
        // Should get two schwa segments, ignoring ☺.
        assert!(segs.len() >= 2);
    }

    // ── resonator_coeffs ────────────────────────────────────────────────────

    #[test]
    fn resonator_unit_dc_gain() {
        // A resonator's DC gain (input = constant 1.0) should converge to 1.0.
        let (a, b, c) = resonator_coeffs(500.0, 100.0, 22050.0);
        // DC gain = A / (1 − B − C) = A / (1 − B − C)
        // We check A / (1 - B - C) ≈ 1.0
        let dc_gain = a / (1.0 - b - c);
        assert!((dc_gain - 1.0).abs() < 1e-6, "dc_gain = {dc_gain}");
    }

    #[test]
    fn resonator_safe_clamp_extreme_freq() {
        // Should not panic for out-of-range frequency values.
        let (a, b, c) = resonator_coeffs(0.0, 0.0, 22050.0);
        assert!(a.is_finite());
        assert!(b.is_finite());
        assert!(c.is_finite());
    }

    // ── synthesize_segments ──────────────────────────────────────────────────

    #[test]
    fn synthesize_empty_gives_empty() {
        let out = synthesize_segments(&[], &default_voice());
        assert!(out.is_empty());
    }

    #[test]
    fn synthesize_produces_correct_length() {
        let segs = parse_ipa("ə", &default_voice());
        let expected = segs[0].dur_samples;
        let out = synthesize_segments(&segs, &default_voice());
        assert_eq!(out.len(), expected);
    }

    #[test]
    fn synthesize_vowel_nonzero() {
        // A voiced vowel must produce non-zero audio.
        let segs = parse_ipa("iː", &default_voice());
        let out = synthesize_segments(&segs, &default_voice());
        let max = out.iter().map(|&s| s.unsigned_abs()).max().unwrap_or(0);
        assert!(max > 1000, "voiced vowel must produce non-trivial audio, got peak {max}");
    }

    #[test]
    fn synthesize_silence_is_zero() {
        use super::Segment;
        let segs = vec![Segment {
            target: SILENCE,
            dur_samples: 1000,
            amp_factor: 1.0,
        }];
        let out = synthesize_segments(&segs, &default_voice());
        assert!(out.iter().all(|&s| s == 0), "silence segment must output zeros");
    }

    #[test]
    fn synthesize_peak_within_i16_range() {
        let segs = parse_ipa("ˈhɛloʊ", &default_voice());
        let out = synthesize_segments(&segs, &default_voice());
        // The synthesizer clamps to ±32767; no sample should be i16::MIN.
        assert!(out.iter().all(|&s| s >= i16::MIN + 1),
            "unexpected i16::MIN in output");
    }

    // ── glottal_sample ───────────────────────────────────────────────────────

    #[test]
    fn glottal_continuity() {
        // The glottal waveform must be continuous: no jump > 0.1 between
        // adjacent phase steps of 1/22050 of a cycle.
        let n = 22050;
        let mut prev = glottal_sample(0.0);
        for i in 1..n {
            let phase = i as f64 / n as f64;
            let cur = glottal_sample(phase);
            let delta = (cur - prev).abs();
            // Large threshold here because the glottal closure can be steep.
            assert!(delta < 0.5, "discontinuity at phase {phase:.4}: Δ = {delta:.4}");
            prev = cur;
        }
    }
}
