//! Klatt cascade formant synthesizer — the DSP core of `klatt.c`.
//!
//! A voicing source (glottal impulse train, optionally mixed with aspiration
//! noise) is passed through a **cascade of digital resonators**, one per formant.
//! Each resonator shapes the source's harmonics into a formant peak, so the
//! output has energy concentrated at the requested formant frequencies — the
//! defining behaviour of a Klatt cascade synthesizer.
//!
//! This is the standalone, spectrally-verifiable core.  **Not yet ported / wired:**
//! the parallel resonator branch (for fricatives/plosive bursts), the nasal
//! antiresonator, the Liljencrants–Fant glottal model, and the integration that
//! drives it from `phondata` Klatt frames in place of the harmonic path
//! (`is_klatt` is still hard-`false` in `wavegen.rs`).  See GAPS §1.2.

use std::f64::consts::PI;

/// A second-order digital resonator (Klatt `setabc` / `resonator`): a pole pair
/// at `freq` with the given bandwidth.
#[derive(Clone, Copy, Default)]
struct Resonator {
    a: f64,
    b: f64,
    c: f64,
    p1: f64,
    p2: f64,
}

impl Resonator {
    /// Set the resonator to `freq` Hz with `bw` Hz bandwidth at `sr` Hz.
    fn set(&mut self, freq: f64, bw: f64, sr: f64) {
        let r = (-PI * bw / sr).exp();
        self.b = 2.0 * r * (2.0 * PI * freq / sr).cos();
        self.c = -(r * r);
        self.a = 1.0 - self.b - self.c; // unity gain at DC
    }

    #[inline]
    fn resonate(&mut self, input: f64) -> f64 {
        let out = self.a * input + self.b * self.p1 + self.c * self.p2;
        self.p2 = self.p1;
        self.p1 = out;
        out
    }
}

/// A steady Klatt frame: pitch, formants, and the mix of the two sources.
#[derive(Debug, Clone)]
pub struct KlattFrame {
    /// Fundamental frequency (Hz).  `0` = unvoiced (noise-only).
    pub f0: f64,
    /// `(frequency, bandwidth)` in Hz for each cascade formant (F1, F2, …).
    pub formants: Vec<(f64, f64)>,
    /// Voicing amplitude (0..=1).
    pub voicing: f64,
    /// Aspiration-noise amplitude (0..=1) mixed into the source (breathiness).
    pub aspiration: f64,
}

impl KlattFrame {
    /// A typical voiced vowel frame (voicing on, no aspiration).
    pub fn vowel(f0: f64, formants: &[(f64, f64)]) -> Self {
        KlattFrame { f0, formants: formants.to_vec(), voicing: 1.0, aspiration: 0.0 }
    }
}

/// Synthesize `n_samples` of a steady `frame` at `sample_rate` Hz.
///
/// The output is peak-normalized to ~0.6 full-scale (the cascade's resonance
/// gain is large and pitch-dependent), so callers get clean, non-clipping audio;
/// spectral *shape* — the point of a formant synthesizer — is unaffected.
pub fn synthesize(frame: &KlattFrame, n_samples: usize, sample_rate: u32) -> Vec<i16> {
    let sr = sample_rate as f64;
    let mut cascade: Vec<Resonator> = frame
        .formants
        .iter()
        .map(|&(f, bw)| {
            let mut r = Resonator::default();
            r.set(f, bw, sr);
            r
        })
        .collect();

    // Deterministic pseudo-noise for aspiration (no external RNG; Date/random
    // aren't needed and this keeps synthesis reproducible).
    let mut rng: u32 = 0x1234_5678;
    let mut noise = || {
        rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((rng >> 9) as f64 / (1u32 << 23) as f64) * 2.0 - 1.0
    };

    let mut phase = 0.0f64; // glottal phase in [0,1)
    let mut raw = Vec::with_capacity(n_samples);
    for _ in 0..n_samples {
        // Voicing: one glottal impulse per pitch period.
        let voiced = if frame.f0 > 0.0 {
            phase += frame.f0 / sr;
            if phase >= 1.0 {
                phase -= 1.0;
                frame.voicing
            } else {
                0.0
            }
        } else {
            0.0
        };
        let source = voiced + frame.aspiration * noise();

        // Cascade the source through every formant resonator.
        let mut s = source;
        for r in &mut cascade {
            s = r.resonate(s);
        }
        raw.push(s);
    }

    // Peak-normalize to avoid clipping.
    let peak = raw.iter().fold(0.0f64, |m, &v| m.max(v.abs())).max(1e-9);
    let scale = 0.6 * 32767.0 / peak;
    raw.iter().map(|&v| (v * scale).clamp(-32767.0, 32767.0) as i16).collect()
}

/// Render a `SpectFrame` sequence through the Klatt cascade — an opt-in
/// alternative to the harmonic `wavegen::synthesize_frames_seq`.  For each frame
/// the cascade resonators are set from its formants (`ffreq[1..7]` with
/// bandwidths `fwidth*4`) and driven by a glottal impulse train at the per-frame
/// pitch; resonator state carries across frames for continuity.  Peak-normalized
/// per run.
///
/// This makes the Klatt synthesizer *usable* (via `VoiceParams.klatt`); it is a
/// first, directionally-verified integration — intra-frame formant interpolation,
/// the parallel/nasal branches, and exact amplitude tracking are not yet ported
/// (GAPS §1.2).
pub fn synthesize_frames_klatt(
    frames: &[super::phondata::SpectFrame],
    amps: &[f64],
    pitch: &[f64],
    sample_rate: u32,
) -> Vec<i32> {
    let sr = sample_rate as f64;
    let mut cascade = [Resonator::default(); 6];
    let mut phase = 0.0f64;
    let mut raw: Vec<f64> = Vec::new();

    // A helper reading a formant's (freq, bandwidth) from a frame.
    let formant = |fr: &super::phondata::SpectFrame, k: usize| -> (f64, f64) {
        let f = fr.ffreq[k + 1] as f64;
        let bw = fr.fwidth.get(k + 1).copied().unwrap_or(10) as f64 * 4.0;
        (f, bw)
    };

    for (fi, fr) in frames.iter().enumerate() {
        // Interpolate this frame's formants toward the next frame's over the
        // frame's duration (time-varying resonators) so transitions glide
        // instead of stepping — the last frame holds steady.
        let next = frames.get(fi + 1).unwrap_or(fr);
        let f0 = pitch.get(fi).copied().unwrap_or(120.0).max(50.0);
        let amp = amps.get(fi).copied().unwrap_or(1.0) * (fr.rms as f64 / 64.0).max(0.05);
        let n = (fr.length as usize) * 64; // STEPSIZE = 64 samples/unit
        if n == 0 {
            continue;
        }

        for j in 0..n {
            let t = j as f64 / n as f64;
            for (k, res) in cascade.iter_mut().enumerate() {
                let (fa, ba) = formant(fr, k);
                let (fb, bb) = formant(next, k);
                let f = fa + (fb - fa) * t;
                if f > 20.0 {
                    let bw = (ba + (bb - ba) * t).clamp(30.0, 1000.0);
                    res.set(f, bw, sr);
                }
            }
            phase += f0 / sr;
            let src = if phase >= 1.0 {
                phase -= 1.0;
                amp
            } else {
                0.0
            };
            let mut s = src;
            for r in &mut cascade {
                s = r.resonate(s);
            }
            raw.push(s);
        }
    }

    // Return pre-AGC i32 samples (the caller runs `agc_clip`); peak-normalize
    // the run to ~0.5 full-scale so AGC has a consistent level to work from.
    let peak = raw.iter().fold(0.0f64, |m, &v| m.max(v.abs())).max(1e-9);
    let scale = 0.5 * 32767.0 / peak;
    raw.iter().map(|&v| (v * scale) as i32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_finite_and_bounded() {
        let frame = KlattFrame::vowel(120.0, &[(730.0, 50.0), (1090.0, 60.0), (2440.0, 120.0)]);
        let pcm = synthesize(&frame, 4410, 22_050); // 0.2 s
        assert_eq!(pcm.len(), 4410);
        assert!(pcm.iter().any(|&s| s != 0), "silent output");
        // Peak-normalized to ~0.6 FS → never clips at the i16 rail.
        assert!(pcm.iter().all(|&s| s.abs() < 32767), "output clipped");
    }

    #[test]
    fn unvoiced_frame_uses_noise_not_pitch() {
        // f0 = 0 with aspiration → noise-driven, still produces sound.
        let frame = KlattFrame { f0: 0.0, formants: vec![(1500.0, 200.0)], voicing: 0.0, aspiration: 1.0 };
        let pcm = synthesize(&frame, 2205, 22_050);
        assert!(pcm.iter().any(|&s| s != 0), "noise source produced silence");
    }
}
