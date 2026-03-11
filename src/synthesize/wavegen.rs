//! Harmonic additive waveform synthesizer — port of `wavegen.c`.
//!
//! Implements [`wavegen_segment`] (single inter-frame transition) and
//! [`synthesize_frames`] (full [`SpectSeq`] → raw `i32` samples).
//!
//! [`SpectSeq`]: super::phondata::SpectSeq
//
// ## Algorithm
//
// eSpeak NG uses *additive harmonic synthesis* to generate audio from formant
// data.  The key steps (mirroring `Wavegen()` + `PeaksToHarmspect()` in the C
// source) are:
//
//   1. For each pair of consecutive spectral frames (fr1, fr2) covering `length`
//      samples:
//      a. Compute formant peaks from the frame (ffreq, fheight, fwidth).
//      b. Build a harmonic spectrum table (`htab`) using `PeaksToHarmspect`:
//         for every harmonic h=1…hmax, sum the contributions from all peaks
//         using the peak-shape table.
//      c. Every STEPSIZE (64) samples, re-interpolate fr1→fr2 and rebuild htab.
//      d. For each sample: sum sin_tab[h·waveph >> 5] * htab[h] over all
//         harmonics.  `waveph` is the current oscillator phase (u16, wrapping).
//
//   2. The glottal oscillator: `wavephase` (i32) starts at i32::MAX and
//      increments by `phaseinc` each sample.  When it crosses zero from
//      positive→negative, that is the "quiet point" of the waveform cycle.
//      `waveph = (wavephase >> 16) as u16` gives the phase in [0, 65535].
//
//   3. Unvoiced phonemes use white noise instead of (or mixed with) the
//      harmonic source.
//
// References: wavegen.c (Wavegen, SetSynth, PeaksToHarmspect, WavegenInit)

use super::phondata::SpectFrame;
use super::VoiceParams;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum harmonic count.  C uses 400 but we cap at 200 for speed.
pub const MAX_HARMONIC: usize = 200;

/// Harmonics below this index are interpolated between STEPSIZE steps.
const N_LOWHARM: usize = 30;

/// Coefficient update interval (samples).  Matches espeak-ng's `STEPSIZE`.
pub const STEPSIZE: usize = 64;

/// Phase increment factor.  C: `0x8000000 / samplerate`.
/// For 22050 Hz: 134217728 / 22050 = 6088 (approx).
const PHASE_INC_FACTOR: i32 = 6088;

// ---------------------------------------------------------------------------
// sin_tab — copied verbatim from sintab.h (2048 entries)
// ---------------------------------------------------------------------------

include!("sintab_data.rs");

// ---------------------------------------------------------------------------
// Peak-shape lookup tables
// ---------------------------------------------------------------------------

/// Peak shape table 2 (used by espeak-ng by default).
/// Source: wavegen.c `pk_shape2[257]`.
static PK_SHAPE2: [u8; 257] = [
    255, 254, 254, 254, 254, 254, 254, 254, 254, 254, 253, 253, 253, 253, 252, 252,
    252, 251, 251, 251, 250, 250, 249, 249, 248, 248, 247, 247, 246, 245, 245, 244,
    243, 243, 242, 241, 239, 237, 235, 233, 231, 229, 227, 225, 223, 221, 218, 216,
    213, 211, 208, 205, 203, 200, 197, 194, 191, 187, 184, 181, 178, 174, 171, 167,
    163, 160, 156, 152, 148, 144, 140, 136, 132, 127, 123, 119, 114, 110, 105, 100,
     96,  94,  91,  88,  86,  83,  81,  78,  76,  74,  71,  69,  66,  64,  62,  60,
     57,  55,  53,  51,  49,  47,  44,  42,  40,  38,  36,  34,  32,  30,  29,  27,
     25,  23,  21,  19,  18,  16,  14,  12,  11,   9,   7,   6,   4,   3,   1,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,
];

/// Wavemult window — Hanning window for HF peak synthesis.
/// Preset for 22050 Hz sample rate; 128 entries (N_WAVEMULT=128).
/// Mirrors `wavemult[N_WAVEMULT]` in wavegen.c.
static WAVEMULT: [u8; 128] = [
      0,   0,   0,   2,   3,   5,   8,  11,  14,  18,  22,  27,  32,  37,  43,  49,
     55,  62,  69,  76,  83,  90,  98, 105, 113, 121, 128, 136, 144, 152, 159, 166,
    174, 181, 188, 194, 201, 207, 213, 218, 224, 228, 233, 237, 240, 244, 246, 249,
    251, 252, 253, 253, 253, 253, 252, 251, 249, 246, 244, 240, 237, 233, 228, 224,
    218, 213, 207, 201, 194, 188, 181, 174, 166, 159, 152, 144, 136, 128, 121, 113,
    105,  98,  90,  83,  76,  69,  62,  55,  49,  43,  37,  32,  27,  22,  18,  14,
     11,   8,   5,   3,   2,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
];

/// Compute wavemult_max for 22050 Hz rate.
/// C: wavemult_max = (samplerate * wavemult_fact) / (256 * 50); wavemult_fact = 60.
const WAVEMULT_MAX: usize = (22050 * 60) / (256 * 50); // = 103

/// Wavemult offset (center of window): wavemult_offset = wavemult_max / 2.
const WAVEMULT_OFFSET: i32 = (WAVEMULT_MAX / 2) as i32; // = 51

// ---------------------------------------------------------------------------
// Voice scale factors (default voice)
// ---------------------------------------------------------------------------

/// Default voice frequency scaling per formant (C: `voice.freq[i]`).
/// 256 = unity scaling.
const VOICE_FREQ: [i32; 9] = [256, 256, 256, 256, 256, 256, 256, 256, 256];

/// Default voice height (amplitude) scaling per formant.
/// From voices.c: default_heights = {130,128,120,116,100,100,128,128,128}, ×2 = {260,256,240,232,200,200,256,256,256}
/// HOWEVER: C's actual amplitude formula targets z1 ~ 25000 (no AGC needed).
/// With voice->height[pk]*2 the htab values are too large by ~(256/64)^2=16.
/// Verified empirically by matching C's output RMS. We use voice->height/4.
const VOICE_HEIGHT: [i64; 9] = [65, 64, 60, 58, 50, 50, 64, 64, 64]; // ÷4 of the ×2 values

/// Default voice width scaling per formant.
/// From voices.c: default_widths = {48,40,40,40,40,40,0,0,0}, ×2 = {96,80,80,80,80,80,0,0,0}
const VOICE_WIDTH: [i64; 9] = [96, 80, 80, 80, 80, 80, 80, 80, 80];

/// Number of formant peaks used for harmonic computation (n_harmonic_peaks).
const N_HARMONIC_PEAKS: usize = 5;

// ---------------------------------------------------------------------------
// Peak struct (mirrors wavegen_peaks_t)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
struct Peak {
    freq:   i64,  // Hz << 16 (fixed point)
    height: i64,  // amplitude (scaled)
    left:   i64,  // left bandwidth
    right:  i64,  // right bandwidth (= left for f3-f5)
}

/// Convert a `SpectFrame` to a peak array using the default voice scaling.
///
/// Mirrors the `SetSynth()` initialisation in wavegen.c.
fn frame_to_peaks(fr: &SpectFrame) -> [Peak; 9] {
    let mut peaks = [Peak::default(); 9];
    // Formants 0-6
    for ix in 0..7 {
        let freq = (fr.ffreq[ix] as i64 * VOICE_FREQ[ix.min(8)] as i64) << 8;
        peaks[ix].freq = freq;
        let height = (fr.fheight[ix] as i64 * VOICE_HEIGHT[ix.min(8)] as i64) << 6;
        peaks[ix].height = height;
        if ix <= 5 {
            let width = (fr.fwidth[ix] as i64 * VOICE_WIDTH[ix.min(8)] as i64) << 10;
            peaks[ix].left = width;
            if ix < 3 {
                let rw = (fr.fright[ix] as i64 * VOICE_WIDTH[ix.min(8)] as i64) << 10;
                peaks[ix].right = rw;
            } else {
                peaks[ix].right = width;
            }
        }
    }
    // Formants 7 and 8: fixed frequencies (as in C's SetSynth hardcodes)
    peaks[7].freq = 7800_i64 << 16;
    peaks[8].freq = 9000_i64 << 16;
    peaks
}

/// Interpolate two peak arrays by factor t ∈ [0, 1].
fn interpolate_peaks(p1: &[Peak; 9], p2: &[Peak; 9], t: f64) -> [Peak; 9] {
    let mut out = [Peak::default(); 9];
    for i in 0..9 {
        out[i].freq   = (p1[i].freq   as f64 + (p2[i].freq   - p1[i].freq)   as f64 * t) as i64;
        out[i].height = (p1[i].height as f64 + (p2[i].height - p1[i].height) as f64 * t) as i64;
        out[i].left   = (p1[i].left   as f64 + (p2[i].left   - p1[i].left)   as f64 * t) as i64;
        out[i].right  = (p1[i].right  as f64 + (p2[i].right  - p1[i].right)  as f64 * t) as i64;
    }
    out
}

// ---------------------------------------------------------------------------
// PeaksToHarmspect — compute harmonic amplitude table
// ---------------------------------------------------------------------------

/// Compute the harmonic amplitude table from formant peaks.
///
/// `pitch_hz16` — fundamental frequency in Hz << 16 (fixed-point).
/// Returns the highest non-zero harmonic index.
///
/// Mirrors `PeaksToHarmspect()` from wavegen.c.
fn peaks_to_harmspect(peaks: &[Peak; 9], pitch_hz16: i64, htab: &mut [i32; MAX_HARMONIC]) -> usize {
    // Zero out the table first
    htab.fill(0);

    if pitch_hz16 <= 0 {
        return 0;
    }

    // Compute hmax: highest harmonic within 95% of Nyquist (22050 Hz)
    let nyquist_limit = (22050_i64 * 19 / 40) << 16; // 95% of Nyquist in Hz<<16
    let hmax_samplerate = (nyquist_limit / pitch_hz16) as usize;
    let hmax = {
        // From the highest peak frequency + its width (N_HARMONIC_PEAKS = wvoice->n_harmonic_peaks)
        let top_peak = &peaks[N_HARMONIC_PEAKS];
        let top_freq = top_peak.freq + top_peak.right;
        let hmax_peak = if pitch_hz16 > 0 { (top_freq / pitch_hz16) as usize } else { 0 };
        hmax_peak.min(hmax_samplerate).min(MAX_HARMONIC - 1)
    };

    // ── Phase 1: Sum formant peak contributions (WITHOUT >>15 — applied later) ──
    // Mirrors: htab[h++] += pk_shape[...] * p->height;  (C does NOT shift here)
    for pk in 0..=N_HARMONIC_PEAKS {
        let p = &peaks[pk];
        if p.height == 0 || p.freq == 0 {
            continue;
        }
        let fp = p.freq;     // centre freq in Hz<<16
        let fhi = fp + p.right;

        // Starting harmonic at the left edge of this peak
        let h_start = if p.left > 0 {
            let h = ((fp - p.left) / pitch_hz16) as usize;
            h.max(1)
        } else {
            ((fp / pitch_hz16) as usize).max(1)
        };

        let mut h = h_start;
        let mut f = pitch_hz16 * h as i64;

        // Rising slope (below peak centre)
        // C: htab[h++] += pk_shape[...] * p->height;  (direct, no >>15 here)
        // max: 255 * 204800 = 52M < i32::MAX (2.1G); saturating_add for safety
        while f < fp && h < MAX_HARMONIC {
            let diff = fp - f;
            let bw = p.left >> 8;
            if bw > 0 {
                let idx = ((diff / bw) as usize).min(256);
                let contrib = (PK_SHAPE2[idx] as i64 * p.height).min(i32::MAX as i64) as i32;
                htab[h] = htab[h].saturating_add(contrib);
            }
            h += 1;
            f += pitch_hz16;
        }
        // Falling slope (above peak centre)
        while f < fhi && h < MAX_HARMONIC {
            let diff = f - fp;
            let bw = p.right >> 8;
            if bw > 0 {
                let idx = ((diff / bw) as usize).min(256);
                let contrib = (PK_SHAPE2[idx] as i64 * p.height).min(i32::MAX as i64) as i32;
                htab[h] = htab[h].saturating_add(contrib);
            }
            h += 1;
            f += pitch_hz16;
        }
    }

    // ── Phase 2: Add bass boost (BEFORE squaring — mirrors C order) ──
    // C: y = peaks[1].height * 10; h2 = (1000<<16)/pitch; x = y/h2; while(y>0){htab[h++]+=y; y-=x;}
    let bass_h = (1000_i64 << 16) / pitch_hz16; // harmonics below 1000 Hz
    if bass_h > 0 {
        // peaks[1].height is in our scaled units (fheight * 4096); convert to C units
        // C: peaks[1].height = fheight * voice->height << 6 = fheight * 64 * 64 = fheight * 4096
        // Our frame_to_peaks stores peaks[1].height = fheight * 64 * 64 (same)
        // C bass: y = peaks[1].height * 10; but after PeaksToHarmspect squaring the units differ.
        // To match: bass_add ≈ peaks[1].height_after_squaring * 10, where height_after_squaring
        // = (htab_contrib >> 15)^2 >> 8 ≈ same order as peak htab entries.
        // Simpler: replicate C exactly: y = peaks[1].height * 10 (pre-squaring units)
        // C: y = peaks[1].height * 10; x = y/h2; while(y>0){htab[h++]+=y; y-=x;}
        let bass_y0 = peaks[1].height * 10;
        let bass_step = (bass_y0 / bass_h).max(1);
        let mut y = bass_y0;
        let mut h = 1usize;
        while y > 0 && h < MAX_HARMONIC {
            htab[h] = htab[h].saturating_add(y.min(i32::MAX as i64) as i32);
            y -= bass_step;
            h += 1;
        }
    }

    // ── Phase 3: Square-root conversion — x = htab[h]>>15; htab[h] = (x*x)>>8 ──
    let mut effective_hmax = 0;
    for h in 1..=hmax {
        let x = htab[h] >> 15;
        htab[h] = (x * x) >> 8;
        if htab[h] > 0 {
            effective_hmax = h;
        }
    }

    // ── Phase 4: First harmonic adjustment — option_harmonic1 = 10 ──
    // C: h1 = htab[1] * option_harmonic1; htab[1] = h1/8;
    if hmax >= 1 {
        htab[1] = htab[1] * 10 / 8;
        if htab[1] > 0 { effective_hmax = effective_hmax.max(1); }
    }

    effective_hmax.max(1)
}

// ---------------------------------------------------------------------------
// Xorshift32 PRNG (for unvoiced noise)
// ---------------------------------------------------------------------------

struct Xorshift32(u32);
impl Xorshift32 {
    fn new() -> Self { Xorshift32(0xBAD_5EED) }
    #[inline]
    fn next_i32(&mut self) -> i32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as i32
    }
}

// ---------------------------------------------------------------------------
// Segment — a synthesis unit between two frames
// ---------------------------------------------------------------------------

/// A synthesis segment: the harmonic-additive transition from `fr1` to `fr2`.
///
/// Corresponds to one inter-frame interval in a [`SpectSeq`].
///
/// [`SpectSeq`]: super::phondata::SpectSeq
#[derive(Debug, Clone)]
pub struct SynthSegment {
    /// Starting formant frame.
    pub fr1: SpectFrame,
    /// Ending formant frame (linearly interpolated towards over `n_samples`).
    pub fr2: SpectFrame,
    /// Number of samples to generate for this segment.
    pub n_samples: usize,
    /// True if this is a voiced segment (glottal source active).
    pub voiced: bool,
    /// Noise fraction mixed in (0.0 = pure tone, 1.0 = pure noise).
    pub noise_frac: f64,
    /// Amplitude multiplier relative to the voice default (1.0 = normal).
    pub amp_factor: f64,
}

// ---------------------------------------------------------------------------
// wavegen_segment — synthesize one segment
// ---------------------------------------------------------------------------

/// Synthesize one segment (transition between two spectral frames) to samples.
///
/// Mirrors the inner loop of `Wavegen()` in wavegen.c.
///
/// # Arguments
/// * `seg`     — segment to synthesize
/// * `f0_hz`  — fundamental frequency (pitch) in Hz
/// * `global_amp` — overall amplitude scale (0.0–1.0)
/// * `wavephase` — carry-over oscillator phase (i32, mutable)
///
/// The `wavephase` state is threaded through calls so that the oscillator
/// continues smoothly across frame boundaries.
pub fn wavegen_segment(
    seg: &SynthSegment,
    f0_hz: f64,
    global_amp: f64,
    wavephase: &mut i32,
) -> Vec<i32> {
    let n = seg.n_samples;
    if n == 0 {
        return Vec::new();
    }

    let mut output = vec![0i32; n];

    let pitch_i32: i32 = ((f0_hz as i64 * 4096) as i32).max(25 * 4096);
    // phaseinc = (pitch >> 7) * PHASE_INC_FACTOR  (from C: pitch is Hz<<12)
    let phaseinc: i32 = ((pitch_i32 >> 7) as i64 * PHASE_INC_FACTOR as i64) as i32;
    let pitch_hz16: i64 = (f0_hz as i64) << 16;

    let peaks1 = frame_to_peaks(&seg.fr1);
    let peaks2 = frame_to_peaks(&seg.fr2);

    // Amplitude: mirrors C formula:
    //   wdata.amplitude = stress_amp * general_amplitude / 16   (e.g. 75 for primary stress)
    //   amplitude2 = wdata.amplitude * (pitch>>8) * amplitude_fmt / 80000
    // We receive amp_factor = wdata_amplitude * amplitude_fmt (= ~7500 for primary, normalized to 1.0)
    // so amp_scale = global_amp * 7500 * amp_factor
    // (7500 = 75 * 100 for primary stress at default volume)
    let amp_scale: i64 = (global_amp * 7500.0 * seg.amp_factor) as i64;

    // harmonic amplitude tables (two copies, we interpolate between them)
    let mut htab = [[0i32; MAX_HARMONIC]; 2];
    let mut harm_inc = [0i32; N_LOWHARM];
    let mut hswitch = 0usize;
    let mut maxh = 0usize;
    let mut maxh2; // computed each STEPSIZE block

    let mut rng = Xorshift32::new();
    let mut amplitude2: i64 = 0;

    // ── HF peak state ─────────────────────────────────────────────────────
    // Harmonics for peaks above N_HARMONIC_PEAKS (F6, F7, F8) are windowed
    // by the wavemult Hanning window centered on the glottal pulse.
    // C: cbytes = wavemult_offset - cycle_samples/2 at start of each cycle.
    // C: hf_factor = wdata.pitch >> 11 = f0_hz*4096 >> 11 = f0_hz*2
    const SAMPLERATE: f64 = 22050.0;
    let cycle_samples = (SAMPLERATE / f0_hz) as i32;
    let hf_factor = ((f0_hz as i64 * 4096) >> 11).max(1); // = f0_hz * 2
    // peak_harmonic[pk-6] = nearest harmonic to F(6+pk)
    // peak_height[pk-6] = amplitude^2 for HF peak
    const N_HF: usize = 3; // peaks 6,7,8
    let mut peak_harmonic = [0u32; N_HF];
    let mut peak_height = [0i64; N_HF];
    let hmax_sr = ((22050 * 19 / 40) as f64 / f0_hz) as u32; // Nyquist limit

    // Initialize HF peaks from interpolated frame at t=0
    {
        let t = 0.0f64;
        let interp = interpolate_peaks(&frame_to_peaks(&seg.fr1), &frame_to_peaks(&seg.fr2), t);
        for (k, fk) in (N_HARMONIC_PEAKS + 1..9).enumerate() {
            let freq_hz16 = interp[fk].freq;
            let pitch = (f0_hz as i64) * 4096; // wdata.pitch = Hz<<12
            // C: peak_harmonic = (peaks.freq / (pitch*8) + 1) / 2
            // where peaks.freq is Hz<<16 and pitch is Hz<<12
            let h = if pitch > 0 { ((freq_hz16 / (pitch * 8) + 1) / 2).max(0) as u32 } else { 0 };
            peak_harmonic[k] = if h <= hmax_sr { h } else { 0 };
            // C: x = peaks.height >> 14; peak_height = (x*x*5)/2
            let x = (interp[fk].height >> 14) as i64;
            peak_height[k] = if peak_harmonic[k] > 0 { (x * x * 5) / 2 } else { 0 };
        }
    }
    let mut cbytes: i32 = WAVEMULT_OFFSET - cycle_samples / 2; // starts negative

    for i in 0..n {
        // ── Every 64 samples: update harmonic table ──────────────────────
        if (i & 63) == 0 {
            let t = i as f64 / n as f64;
            let interp_peaks = interpolate_peaks(&peaks1, &peaks2, t);

            if i == 0 {
                hswitch = 0;
                let hm = peaks_to_harmspect(&interp_peaks, pitch_hz16, &mut htab[0]);
                hswitch ^= 1;
                maxh = hm;

                // adjust amplitude for pitch (fewer harmonics at high pitch)
                amplitude2 = (amp_scale * ((pitch_i32 >> 8) as i64)) / (10000 * 8);
            } else {
                maxh2 = peaks_to_harmspect(&interp_peaks, pitch_hz16, &mut htab[hswitch]);

                // Compute interpolation increments for low harmonics
                let other = hswitch ^ 1;
                let n_lo = N_LOWHARM.min(maxh2).min(maxh);
                for h in 1..n_lo {
                    harm_inc[h] = (htab[hswitch][h] - htab[other][h]) >> 3;
                }

                hswitch ^= 1;
                maxh = maxh2;
            }
        } else if (i & 7) == 0 {
            // Every 8 samples: interpolate low harmonics
            for h in 1..N_LOWHARM.min(maxh) {
                htab[hswitch][h] = htab[hswitch][h].saturating_add(harm_inc[h]);
            }
        }

        // ── Advance oscillator ────────────────────────────────────────────
        let old_phase = *wavephase;
        *wavephase = wavephase.wrapping_add(phaseinc);

        // Detect zero crossing (quiet point of waveform cycle) — new glottal pulse
        if old_phase > 0 && *wavephase < 0 {
            // Recompute amplitude every cycle
            amplitude2 = (amp_scale * ((pitch_i32 >> 8) as i64)) / (10000 * 8);
            // Reset cbytes to start of window (C: cbytes = wavemult_offset - cycle_samples/2)
            cbytes = WAVEMULT_OFFSET - cycle_samples / 2;
        }

        // ── Generate sample ───────────────────────────────────────────────
        let waveph = (*wavephase >> 16) as u16;

        let mut total: i64 = 0;

        if seg.voiced {
            // ── Step 1: HF peaks (windowed by wavemult) ──────────────────
            // C applies HF contributions FIRST, then multiplies by window.
            // This creates the glottal pulse shape.
            cbytes += 1;
            if cbytes >= 0 && (cbytes as usize) < WAVEMULT_MAX {
                let mut hf_total: i64 = 0;
                for k in 0..N_HF {
                    let h = peak_harmonic[k];
                    if h > 0 && peak_height[k] > 0 {
                        let theta = (waveph as u32).wrapping_mul(h) as u16;
                        let idx = (theta >> 5) as usize;
                        hf_total += SIN_TAB[idx] as i64 * peak_height[k];
                    }
                }
                // C: total = (long)(total / hf_factor) * wavemult[cbytes]
                // hf_total is the HF contribution; multiply by window
                total = (hf_total / hf_factor) * WAVEMULT[cbytes as usize] as i64;
            } else {
                total = 0;
            }

            // ── Step 2: Main peaks (F0-F5), sign-switched above ~900 Hz ─
            let mut theta = waveph;
            let mh = maxh.min(MAX_HARMONIC - 1);
            let h_switch_sign = (890_u32 / (f0_hz as u32).max(1)) as usize;
            let h_switch_sign = h_switch_sign.min(mh);

            for h in 1..=h_switch_sign {
                let idx = (theta >> 5) as usize;
                total += SIN_TAB[idx] as i64 * htab[hswitch][h] as i64;
                theta = theta.wrapping_add(waveph);
            }
            for h in (h_switch_sign + 1)..=mh {
                let idx = (theta >> 5) as usize;
                total -= SIN_TAB[idx] as i64 * htab[hswitch][h] as i64;
                theta = theta.wrapping_add(waveph);
            }
        }

        if seg.noise_frac > 0.0 {
            let noise_amp = (seg.noise_frac * amplitude2 as f64) as i64;
            total += rng.next_i32() as i64 * noise_amp / (1 << 20);
        }

        // Scale by amplitude
        let z = (total >> 8) * amplitude2 / (1 << 13);
        output[i] = z.clamp(i32::MIN as i64 + 1, i32::MAX as i64) as i32;
    }

    output
}

// ---------------------------------------------------------------------------
// synthesize_frames — synthesize a full spectral sequence
// ---------------------------------------------------------------------------

/// Synthesize a full `SpectSeq` (all frames) into raw samples.
///
/// Frames are synthesized in pairs: frame[i] → frame[i+1].
/// The last frame is held for one extra segment (repeated).
///
/// Returns unnormalized i32 samples.
pub fn synthesize_frames(
    seq: &super::phondata::SpectSeq,
    voice: &VoiceParams,
    amp_factor: f64,
    wavephase: &mut i32,
) -> Vec<i32> {
    if seq.frames.is_empty() {
        return Vec::new();
    }

    let speed_factor = 100.0 / voice.speed_percent.max(1) as f64;
    let f0_hz = voice.pitch_hz.max(25) as f64;
    let global_amp = voice.amplitude.clamp(0, 100) as f64 / 100.0;

    let mut output = Vec::new();

    let n = seq.frames.len();
    for i in 0..n {
        let fr1 = &seq.frames[i];
        let fr2 = if i + 1 < n { &seq.frames[i + 1] } else { fr1 };

        let n_samples = fr1.dur_samples(speed_factor);

        // Determine voiced/noise character from the frame
        let av = fr1.klattp[0]; // AV (voicing amplitude)
        let fric = if seq.is_klatt { fr1.klattp2[1] } else { 0 }; // Fric

        let (voiced, noise_frac) = if av > 0 && fr1.ffreq[1] > 0 {
            (true, fric as f64 / 255.0)
        } else if fr1.ffreq[1] == 0 && av == 0 {
            (false, 0.5) // fricative/stop: mostly noise
        } else {
            (av > 0, fric as f64 / 255.0)
        };

        let seg = SynthSegment {
            fr1: fr1.clone(),
            fr2: fr2.clone(),
            n_samples,
            voiced,
            noise_frac,
            amp_factor,
        };

        let samples = wavegen_segment(&seg, f0_hz, global_amp, wavephase);
        output.extend_from_slice(&samples);
    }

    output
}

// ---------------------------------------------------------------------------
// f32_to_i16 normalisation
// ---------------------------------------------------------------------------

/// Normalise an i32 sample buffer and clip to i16.
pub fn i32_to_i16(samples: &[i32]) -> Vec<i16> {
    if samples.is_empty() {
        return Vec::new();
    }
    let peak = samples.iter().map(|&x| x.unsigned_abs()).max().unwrap_or(0);
    // Target: 90% of i16 full scale
    let target = (0.90 * 32767.0) as i64;
    let scale: i64 = if peak > 0 { target * 4096 / peak as i64 } else { 0 };

    samples.iter()
        .map(|&x| ((x as i64 * scale) >> 12).clamp(-32767, 32767) as i16)
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn voiced_frame(f1: i16, f2: i16, f3: i16, len: u8) -> SpectFrame {
        let mut f = SpectFrame::default();
        f.ffreq[1] = f1;
        f.ffreq[2] = f2;
        f.ffreq[3] = f3;
        f.fwidth[0] = 25;  // F0 width
        f.fwidth[1] = 50;  // F1 width
        f.fwidth[2] = 80;  // F2 width
        f.fright[0] = 25;
        f.fright[1] = 50;
        f.fright[2] = 80;
        f.fheight = [100, 60, 40, 20, 10, 5, 2, 0];
        f.rms = 50;
        f.klattp[0] = 50; // AV = 50 (voiced)
        f.length = len;
        f
    }

    fn default_voice() -> VoiceParams { VoiceParams::default() }

    #[test]
    fn peaks_to_harmspect_nonzero() {
        let frame = voiced_frame(500, 1500, 2500, 4);
        let peaks = frame_to_peaks(&frame);
        let mut htab = [0i32; MAX_HARMONIC];
        let hmax = peaks_to_harmspect(&peaks, 118_i64 << 16, &mut htab);
        // Should produce non-zero harmonics
        assert!(hmax > 0);
        let total: i32 = htab[1..].iter().sum();
        assert!(total > 0, "harmonic table should be non-zero");
    }

    #[test]
    fn wavegen_segment_produces_audio() {
        let fr = voiced_frame(500, 1500, 2500, 4);
        let seg = SynthSegment {
            fr1: fr.clone(),
            fr2: fr.clone(),
            n_samples: 512,
            voiced: true,
            noise_frac: 0.0,
            amp_factor: 1.0,
        };
        let mut phase = i32::MAX;
        let samples = wavegen_segment(&seg, 118.0, 0.8, &mut phase);
        assert_eq!(samples.len(), 512);
        let max = samples.iter().map(|x| x.unsigned_abs()).max().unwrap_or(0);
        assert!(max > 0, "voiced segment must produce non-zero audio");
    }

    #[test]
    fn wavegen_segment_silence_when_zero_peaks() {
        let fr = SpectFrame::default(); // all zeros
        let seg = SynthSegment {
            fr1: fr.clone(),
            fr2: fr.clone(),
            n_samples: 256,
            voiced: false,
            noise_frac: 0.0,
            amp_factor: 1.0,
        };
        let mut phase = i32::MAX;
        let samples = wavegen_segment(&seg, 118.0, 0.8, &mut phase);
        assert_eq!(samples.len(), 256);
        // All-zero frame with no voicing and no noise → silence
        let max = samples.iter().map(|x| x.unsigned_abs()).max().unwrap_or(0);
        assert_eq!(max, 0, "zero frame + no voice → silence");
    }

    #[test]
    fn synthesize_frames_uses_all_frames() {
        let seq = super::super::phondata::SpectSeq {
            frames: vec![
                voiced_frame(500, 1500, 2500, 2),
                voiced_frame(510, 1480, 2480, 2),
                voiced_frame(520, 1460, 2460, 2),
            ],
            is_klatt: false,
        };
        let mut phase = i32::MAX;
        let samples = synthesize_frames(&seq, &default_voice(), 1.0, &mut phase);
        // 3 frames × 2 steps × 64 = 384 samples
        let expected = 3 * 2 * 64;
        assert_eq!(samples.len(), expected,
            "expected {expected} samples, got {}", samples.len());
    }

    #[test]
    fn i32_to_i16_normalises() {
        let big = vec![100_000i32, -200_000i32, 150_000i32];
        let out = i32_to_i16(&big);
        assert_eq!(out.len(), 3);
        // Peak should be close to 90% of 32767
        let peak = out.iter().map(|&x| x.unsigned_abs()).max().unwrap();
        assert!(peak > 28000, "should normalise close to full scale: peak={peak}");
    }

    #[test]
    fn i32_to_i16_empty() {
        assert!(i32_to_i16(&[]).is_empty());
    }

    #[test]
    fn phase_carries_over() {
        // Phase must evolve smoothly across two calls
        let fr = voiced_frame(500, 1500, 2500, 1);
        let seg = SynthSegment {
            fr1: fr.clone(),
            fr2: fr.clone(),
            n_samples: 64,
            voiced: true,
            noise_frac: 0.0,
            amp_factor: 1.0,
        };
        let mut phase = i32::MAX;
        let _ = wavegen_segment(&seg, 118.0, 0.8, &mut phase);
        let phase_after_first = phase;
        let _ = wavegen_segment(&seg, 118.0, 0.8, &mut phase);
        // Phase should have advanced (not reset)
        assert_ne!(phase, phase_after_first,
            "phase should evolve between calls");
    }
}
