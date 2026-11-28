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
use crate::bytes::Bounded;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum harmonic count.  C uses 400 but we cap at 200 for speed.
pub const MAX_HARMONIC: usize = 400;

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
static PK_SHAPE1: [u8; 257] = [
    255, 254, 254, 254, 254, 254, 253, 253, 252, 251, 251, 250, 249, 248, 247, 246, 245, 244, 242,
    241, 239, 238, 236, 234, 233, 231, 229, 227, 225, 223, 220, 218, 216, 213, 211, 209, 207, 205,
    203, 201, 199, 197, 195, 193, 191, 189, 187, 185, 183, 180, 178, 176, 173, 171, 169, 166, 164,
    161, 159, 156, 154, 151, 148, 146, 143, 140, 138, 135, 132, 129, 126, 123, 120, 118, 115, 112,
    108, 105, 102, 99, 96, 95, 93, 91, 90, 88, 86, 85, 83, 82, 80, 79, 77, 76, 74, 73, 72, 70, 69,
    68, 67, 66, 64, 63, 62, 61, 60, 59, 58, 57, 56, 55, 55, 54, 53, 52, 52, 51, 50, 50, 49, 48, 48,
    47, 47, 46, 46, 46, 45, 45, 45, 44, 44, 44, 44, 44, 44, 44, 43, 43, 43, 43, 44, 43, 42, 42, 41,
    40, 40, 39, 38, 38, 37, 36, 36, 35, 35, 34, 33, 33, 32, 32, 31, 30, 30, 29, 29, 28, 28, 27, 26,
    26, 25, 25, 24, 24, 23, 23, 22, 22, 21, 21, 20, 20, 19, 19, 18, 18, 18, 17, 17, 16, 16, 15, 15,
    15, 14, 14, 13, 13, 13, 12, 12, 11, 11, 11, 10, 10, 10, 9, 9, 9, 8, 8, 8, 7, 7, 7, 7, 6, 6, 6,
    5, 5, 5, 5, 4, 4, 4, 4, 4, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0,
];

/// `Flutter_tab[]` — a slow irregular wobble added to the pitch.
static FLUTTER_TAB: [u8; 0x170] = [
    128, 155, 181, 203, 220, 232, 237, 236, 230, 220, 206, 191, 176, 163, 152, 144, 140, 139, 140,
    143, 146, 148, 149, 146, 140, 131, 120, 105, 89, 73, 60, 49, 42, 41, 45, 54, 68, 86, 105, 125,
    143, 159, 170, 177, 178, 173, 164, 150, 135, 120, 105, 92, 83, 79, 79, 85, 94, 107, 122, 136,
    150, 162, 171, 176, 177, 174, 168, 160, 152, 145, 139, 136, 137, 141, 148, 157, 168, 178, 187,
    192, 193, 189, 180, 165, 146, 124, 99, 74, 50, 30, 14, 5, 2, 5, 15, 30, 48, 68, 89, 109, 127,
    140, 150, 156, 159, 159, 157, 155, 153, 153, 156, 161, 169, 179, 191, 202, 213, 220, 224, 222,
    216, 204, 187, 166, 143, 119, 96, 75, 58, 46, 40, 41, 47, 58, 72, 89, 106, 122, 134, 144, 148,
    149, 145, 137, 128, 117, 107, 98, 92, 90, 92, 97, 105, 116, 128, 138, 148, 154, 158, 157, 152,
    144, 134, 124, 113, 104, 98, 96, 99, 107, 120, 136, 155, 175, 194, 210, 223, 230, 231, 226,
    215, 198, 178, 156, 132, 111, 91, 75, 64, 57, 55, 56, 61, 67, 74, 80, 84, 86, 85, 82, 77, 72,
    66, 63, 62, 65, 73, 86, 103, 124, 147, 171, 195, 217, 234, 246, 252, 251, 244, 231, 213, 192,
    170, 148, 128, 113, 100, 93, 90, 92, 97, 104, 112, 119, 125, 127, 127, 123, 116, 107, 97, 87,
    78, 72, 70, 72, 78, 89, 102, 117, 132, 147, 159, 167, 171, 170, 164, 153, 139, 123, 106, 91,
    78, 70, 67, 69, 77, 90, 107, 127, 146, 166, 184, 197, 207, 211, 210, 205, 196, 185, 173, 161,
    150, 142, 137, 135, 135, 138, 141, 145, 146, 145, 140, 132, 120, 104, 85, 65, 46, 28, 14, 5, 1,
    5, 15, 31, 52, 77, 104, 129, 154, 176, 193, 205, 211, 211, 208, 200, 191, 181, 171, 164, 159,
    156, 157, 160, 165, 170, 174, 177, 176, 171, 163, 150, 135, 118, 99, 81, 66, 54, 47, 45, 49,
    58, 72, 89, 107, 126, 142, 156, 166, 170, 169, 163, 152, 138, 123, 108, 93, 82, 74, 72, 74, 80,
    90, 103, 117, 130,
];

/// `pk_shape2` — the squarer peaks of a `formant…` voice with `peak_shape` 1.
#[allow(dead_code)]
static PK_SHAPE2: [u8; 257] = [
    255, 254, 254, 254, 254, 254, 254, 254, 254, 254, 253, 253, 253, 253, 252, 252, 252, 251, 251,
    251, 250, 250, 249, 249, 248, 248, 247, 247, 246, 245, 245, 244, 243, 243, 242, 241, 239, 237,
    235, 233, 231, 229, 227, 225, 223, 221, 218, 216, 213, 211, 208, 205, 203, 200, 197, 194, 191,
    187, 184, 181, 178, 174, 171, 167, 163, 160, 156, 152, 148, 144, 140, 136, 132, 127, 123, 119,
    114, 110, 105, 100, 96, 94, 91, 88, 86, 83, 81, 78, 76, 74, 71, 69, 66, 64, 62, 60, 57, 55, 53,
    51, 49, 47, 44, 42, 40, 38, 36, 34, 32, 30, 29, 27, 25, 23, 21, 19, 18, 16, 14, 12, 11, 9, 7,
    6, 4, 3, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0,
];

/// Wavemult window — Hanning window for HF peak synthesis.
/// Preset for 22050 Hz sample rate; 128 entries (N_WAVEMULT=128).
/// Mirrors `wavemult[N_WAVEMULT]` in wavegen.c.
static WAVEMULT: [u8; 128] = [
    0, 0, 0, 2, 3, 5, 8, 11, 14, 18, 22, 27, 32, 37, 43, 49, 55, 62, 69, 76, 83, 90, 98, 105, 113,
    121, 128, 136, 144, 152, 159, 166, 174, 181, 188, 194, 201, 207, 213, 218, 224, 228, 233, 237,
    240, 244, 246, 249, 251, 252, 253, 253, 253, 253, 252, 251, 249, 246, 244, 240, 237, 233, 228,
    224, 218, 213, 207, 201, 194, 188, 181, 174, 166, 159, 152, 144, 136, 128, 121, 113, 105, 98,
    90, 83, 76, 69, 62, 55, 49, 43, 37, 32, 27, 22, 18, 14, 11, 8, 5, 3, 2, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Compute wavemult_max for 22050 Hz rate.
/// C: wavemult_max = (samplerate * wavemult_fact) / (256 * 50); wavemult_fact = 60.
const WAVEMULT_MAX: usize = (22050 * 60) / (256 * 50); // = 103

/// Wavemult offset (center of window): wavemult_offset = wavemult_max / 2.
const WAVEMULT_OFFSET: i32 = (WAVEMULT_MAX / 2) as i32; // = 51

// ---------------------------------------------------------------------------
// Voice scale factors (default voice)
// ---------------------------------------------------------------------------

/// Formant scaling of a voice (`voice->freq/height/width/freqadd`), 256 = as in
/// the data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormantScale {
    pub freq: [i32; 9],
    pub height: [i32; 9],
    pub width: [i32; 9],
    /// Added to the frequency, in Hz.
    pub freq_add: [i32; 9],
}

impl Default for FormantScale {
    /// `VoiceReset`: `default_heights` and `default_widths` × 2.
    fn default() -> Self {
        FormantScale {
            freq: [256; 9],
            height: [260, 256, 240, 232, 200, 200, 256, 256, 256],
            width: [280, 256, 256, 320, 342, 342, 256, 256, 256],
            freq_add: [0; 9],
        }
    }
}

impl FormantScale {
    /// A frame's formant frequency in this voice, Hz × 256.
    pub(super) fn freq(&self, ix: usize, hz: i16) -> i64 {
        let freq =
            i64::from(hz) * i64::from(self.freq.at(ix)) + i64::from(self.freq_add.at(ix)) * 256;
        freq.max(0)
    }

    /// A `formant` line (`VoiceFormant`); negative values leave the default.
    pub fn set(&mut self, ix: usize, freq: i32, height: i32, width: i32, freq_add: i32) {
        if ix > 8 {
            return;
        }
        // (Bounded, so that no voice file overflows the synthesizer.)
        let scale = |percent: i32| (f64::from(percent.min(2000)) * 2.56001) as i32;
        if freq >= 0 {
            self.freq.put(ix, scale(freq));
        }
        if height >= 0 {
            self.height.put(ix, scale(height));
        }
        if width >= 0 {
            self.width.put(ix, scale(width));
        }
        if ix == 0 {
            let width = self.width.at(0) * 105 / 100;
            self.width.put(0, width);
        }
        self.freq_add.put(ix, freq_add.clamp(-5000, 5000));
    }
}

/// Default `breathw` (`VoiceReset`).
pub const BREATH_WIDTHS: [i32; 9] = [0, 200, 200, 400, 400, 400, 600, 600, 600];

/// Number of formant peaks used for harmonic computation (n_harmonic_peaks).
const N_HARMONIC_PEAKS: usize = 5;

// ---------------------------------------------------------------------------
// Peak struct (mirrors wavegen_peaks_t)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
struct Peak {
    freq: i64,   // Hz << 16 (fixed point)
    height: i64, // amplitude (scaled)
    left: i64,   // left bandwidth
    right: i64,  // right bandwidth (= left for f3-f5)
}

/// Convert a `SpectFrame` to a peak array using the default voice scaling.
///
/// Mirrors the `SetSynth()` initialisation in wavegen.c.
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
fn frame_to_peaks(fr: &SpectFrame, voice: &FormantScale) -> [Peak; 9] {
    let mut peaks = [Peak::default(); 9];
    // C runs `for (ix = 0; ix < 8; ix++)`: frequencies for peaks 0–6 come from
    // the frame, but **heights run to peak 7** — `fheight` has eight entries, and
    // peak 7 sits at the fixed 7800 Hz set below.  Stopping the height loop at 6
    // silences that peak and costs ~10 dB above 5 kHz.
    for ix in 0..8 {
        if ix < 7 {
            peaks[ix].freq = voice.freq(ix, fr.ffreq.at(ix)) << 8;
        }
        peaks[ix].height = (fr.fheight.at(ix) as i64 * voice.height.at(ix) as i64) << 6;
        if ix <= 5 {
            let width = (fr.fwidth.at(ix) as i64 * voice.width.at(ix) as i64) << 10;
            peaks[ix].left = width;
            if ix < 3 {
                let rw = (fr.fright.at(ix) as i64 * voice.width.at(ix) as i64) << 10;
                peaks[ix].right = rw;
            } else {
                peaks[ix].right = width;
            }
        }
    }
    // Peaks 7 and 8 sit at fixed frequencies (C's `SetSynth` hardcodes them).
    // Peak 8's height is never assigned by C either, so it stays silent.
    peaks[7].freq = 7800_i64 << 16;
    peaks[8].freq = 9000_i64 << 16;
    peaks
}

/// Interpolate two peak arrays by factor t ∈ [0, 1].
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
fn interpolate_peaks(p1: &[Peak; 9], p2: &[Peak; 9], t: f64) -> [Peak; 9] {
    let mut out = [Peak::default(); 9];
    for i in 0..9 {
        out[i].freq = (p1.at(i).freq as f64 + (p2.at(i).freq - p1.at(i).freq) as f64 * t) as i64;
        out[i].height =
            (p1.at(i).height as f64 + (p2.at(i).height - p1.at(i).height) as f64 * t) as i64;
        out[i].left = (p1.at(i).left as f64 + (p2.at(i).left - p1.at(i).left) as f64 * t) as i64;
        out[i].right =
            (p1.at(i).right as f64 + (p2.at(i).right - p1.at(i).right) as f64 * t) as i64;
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
fn peaks_to_harmspect(
    peaks: &[Peak; 9],
    pitch_hz16: i64,
    htab: &mut [i32; MAX_HARMONIC],
    tone_adjust: &[u8; N_TONE_ADJUST],
) -> usize {
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
        let hmax_peak = if pitch_hz16 > 0 {
            (top_freq / pitch_hz16) as usize
        } else {
            0
        };
        hmax_peak.min(hmax_samplerate).min(MAX_HARMONIC - 1)
    };

    // ── Phase 1: Sum formant peak contributions (WITHOUT >>15 — applied later) ──
    // Mirrors: htab[h++] += pk_shape[...] * p->height;  (C does NOT shift here)
    for pk in 0..=N_HARMONIC_PEAKS {
        let p = &peaks.at(pk);
        if p.height == 0 || p.freq == 0 {
            continue;
        }
        let fp = p.freq; // centre freq in Hz<<16
        let fhi = fp + p.right;

        // Starting harmonic at the left edge of this peak.  C is
        // `h = ((p->freq - p->left) / pitch) + 1; if (h <= 0) h = 1;` — the
        // `+1` picks the first harmonic *inside* the peak's skirt.
        let h_start = {
            let edge = if p.left > 0 { fp - p.left } else { fp };
            let h = edge / pitch_hz16 + 1;
            if h <= 0 {
                1usize
            } else {
                h as usize
            }
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
                let contrib = (PK_SHAPE1.at(idx) as i64 * p.height).min(i32::MAX as i64) as i32;
                let level = htab.at(h).saturating_add(contrib);
                htab.put(h, level);
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
                let contrib = (PK_SHAPE1.at(idx) as i64 * p.height).min(i32::MAX as i64) as i32;
                let level = htab.at(h).saturating_add(contrib);
                htab.put(h, level);
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
            let level = htab.at(h).saturating_add(y.min(i32::MAX as i64) as i32);
            htab.put(h, level);
            y -= bass_step;
            h += 1;
        }
    }

    // ── Phase 3: Square-root conversion — (x * x) >> 8 with x = level >> 15 ──
    // …then the voice's `tone` curve, indexed by the harmonic's frequency in
    // 8 Hz steps (`wvoice->tone_adjust`).  The default curve tilts the spectrum
    // down above 600 Hz.
    for h in 1..=hmax {
        let x = htab.at(h) >> 15;
        let level = (x * x) >> 8;
        htab.put(h, level);
        let ix = ((pitch_hz16 * h as i64) >> 19) as usize;
        if ix < N_TONE_ADJUST {
            let level = ((htab.at(h) as i64 * tone_adjust.at(ix) as i64) >> 13) as i32;
            htab.put(h, level);
        }
    }

    // ── Phase 4: First harmonic adjustment — option_harmonic1 = 10 ──
    // C: h1 = htab[1] * option_harmonic1; htab[1] = h1/8;
    if hmax >= 1 {
        htab[1] = htab[1] * 10 / 8;
    }

    hmax.max(1)
}

/// `N_TONE_ADJUST` — the voice tone curve has 1000 entries in 8 Hz steps (8 kHz).
pub const N_TONE_ADJUST: usize = 1000;

/// `tone_points` — the default tone curve every voice starts from
/// (`voices.c`): flat to 600 Hz, then falling.  `-1` ends the list and extends
/// the last height to the top of the range.
pub const DEFAULT_TONE_POINTS: [i32; 12] = [600, 170, 1200, 135, 2000, 110, 3000, 110, -1, 0, 0, 0];

/// The expanded default curve, used as the reference the applied curve is
/// measured against (see `peaks_to_harmspect`).
pub static DEFAULT_TONE_ADJUST: std::sync::LazyLock<[u8; N_TONE_ADJUST]> =
    std::sync::LazyLock::new(|| set_tone_adjust(&DEFAULT_TONE_POINTS));

/// `SetToneAdjust` — expand `tone <freq> <height> …` pairs into the per-harmonic
/// amplitude table, linearly interpolating between the points.
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
pub fn set_tone_adjust(tone_pts: &[i32; 12]) -> [u8; N_TONE_ADJUST] {
    let mut out = [0u8; N_TONE_ADJUST];
    let mut pts = *tone_pts;
    let mut freq1 = 0usize;
    let mut height1 = pts[1];

    let mut pt = 0usize;
    while pt < 12 {
        if pts.at(pt) == -1 {
            pts.put(pt, (N_TONE_ADJUST * 8) as i32);
            if pt > 0 {
                pts[pt + 1] = pts.at(pt - 1);
            }
        }
        let freq2 = (pts.at(pt) / 8) as usize; // 8 Hz steps
        let height2 = pts.at(pt + 1);
        if freq2 > freq1 {
            for ix in freq1..freq2.min(N_TONE_ADJUST) {
                let y =
                    height1 + ((ix - freq1) as i32 * (height2 - height1)) / (freq2 - freq1) as i32;
                out.put(ix, y.clamp(0, 255) as u8);
            }
        }
        freq1 = freq2;
        height1 = height2;
        pt += 2;
    }
    out
}

// ---------------------------------------------------------------------------
// synthesize_frames — synthesize a full spectral sequence
// ---------------------------------------------------------------------------

/// Synthesize a full `SpectSeq` (all frames, the last one held) into raw
/// samples, at the voice's own pitch.
///
/// The rate is already in the frame lengths, so `speed_percent` is not applied.
pub fn synthesize_frames(
    seq: &super::phondata::SpectSeq,
    voice: &VoiceParams,
    amp_factor: f64,
    wavephase: &mut i32,
) -> Vec<i32> {
    let Some(last) = seq.frames.last() else {
        return Vec::new();
    };
    let mut frames = seq.frames.clone();
    frames.push(SpectFrame {
        length: 0,
        ..last.clone()
    });
    let voice = VoiceParams {
        speed_percent: 100,
        ..voice.clone()
    };
    let amps = vec![amp_factor; frames.len()];
    let pitches = vec![f64::from(voice.pitch_hz.max(25)); frames.len()];
    synthesize_frames_seq(&frames, &amps, &pitches, &[], &voice, wavephase)
}

/// Synthesize a *run* of formant frames spanning one or more phonemes, with a
/// per-frame amplitude factor and per-frame pitch (Hz).  Because the whole run
/// is one sequence, consecutive frames across phoneme boundaries interpolate
/// continuously (the cross-phoneme continuity the per-phoneme path lacks).
///
/// `amps` and `pitches_hz` are parallel to `frames`.  `voice.speed_percent`
/// still governs the per-frame sample count (frame lengths already encode the
/// duration model).  Returns unnormalised i32 samples.
#[expect(
    clippy::indexing_slicing,
    reason = "indices are bounded by the loops over these lists"
)]
pub fn synthesize_frames_seq(
    frames: &[super::phondata::SpectFrame],
    amps: &[f64],
    pitches_hz: &[f64],
    // Per-frame duration multiplier: `1.0` is the voice's own rate, and an
    // inline `\x01<n>S` command scales the frames after it.
    speeds: &[f64],
    voice: &VoiceParams,
    wavephase: &mut i32,
) -> Vec<i32> {
    let speed_factor = 100.0 / voice.speed_percent.max(1) as f64;
    // `amplitude` is half the volume setting: 50 is C's `-a 100`.
    // An echo is (partly) compensated for.
    let echo = f64::from(500 - voice.echo_amp.clamp(0, 100)) / 500.0;
    let global_amp = voice.amplitude.clamp(0, 100) as f64 / 50.0 * echo;

    // C emits a segment per frame but the last, which is only a target; a frame
    // of no length passes its successor on as the next segment's start.
    let segments: Vec<Segment> = frames
        .iter()
        .enumerate()
        .filter(|(_, fr)| fr.duration() > 0)
        .map(|(i, fr)| {
            let pitch = |k: usize| pitches_hz.get(k).copied().unwrap_or(120.0).max(25.0);
            let speed = speeds.get(i).copied().unwrap_or(1.0);
            let next = (i + 1).min(frames.len() - 1);
            Segment {
                fr1: fr,
                fr2: &frames[next],
                length: match fr.samples {
                    Some(n) => n as usize,
                    None => fr.dur_samples(speed_factor * speed),
                },
                amplitude: (75.625 * global_amp * amps.get(i).copied().unwrap_or(1.0)) as i64,
                pitch1: (pitch(i) * 4096.0) as i64,
                pitch2: (pitch(next) * 4096.0) as i64,
            }
        })
        .collect();
    let mut gen = WaveGen::new(*wavephase, voice);
    let last = segments.len().saturating_sub(1);
    for (i, segment) in segments.iter().enumerate() {
        gen.run(segment, i == last);
    }
    *wavephase = gen.wavephase;
    gen.out
}

/// One `WCMD_SPECT`: the move from one frame to the next.
struct Segment<'a> {
    fr1: &'a SpectFrame,
    fr2: &'a SpectFrame,
    /// Samples, before rounding to whole steps.
    length: usize,
    /// `wdata.amplitude`.
    amplitude: i64,
    /// Pitch at the start and end, Hz << 12.
    pitch1: i64,
    pitch2: i64,
}

/// The state `Wavegen()` carries from one segment to the next: the glottal
/// cycle, the harmonic tables and the sample count run straight through a run
/// of formant frames.
struct WaveGen<'a> {
    voice: &'a VoiceParams,
    out: Vec<i32>,
    wavephase: i32,
    pitch: i64,
    samplecount: usize,
    nsamples: usize,
    /// Two harmonic tables: the one playing, moved towards the other.
    hspect: [[i32; MAX_HARMONIC]; 2],
    hswitch: usize,
    playing: usize,
    harm_inc: [i32; N_LOWHARM],
    maxh: usize,
    maxh2: usize,
    h_switch_sign: usize,
    amplitude2: i64,
    cbytes: i32,
    cycle_count: u32,
    peak_harmonic: [i64; N_PEAKS],
    peak_height: [i64; N_PEAKS],
    flutter_ix: usize,
    breath: [BreathFilter; N_PEAKS],
    noise_seed: u32,
}

/// A breath resonator (`RESONATOR`).
#[derive(Debug, Clone, Copy, Default)]
struct BreathFilter {
    a: f64,
    b: f64,
    c: f64,
    x1: f64,
    x2: f64,
}

impl BreathFilter {
    /// `setresonator`.
    fn set(&mut self, freq: i64, bandwidth: i32) {
        use std::f64::consts::PI;
        let r = (-PI / 22050.0 * f64::from(bandwidth)).exp();
        self.c = -(r * r);
        self.b = r * (2.0 * PI / 22050.0 * freq as f64).cos() * 2.0;
        self.a = 1.0 - self.b - self.c;
    }

    fn next(&mut self, input: f64) -> f64 {
        let x = self.a * input + self.b * self.x1 + self.c * self.x2;
        self.x2 = self.x1;
        self.x1 = x;
        x
    }
}

/// `N_PEAKS`: formants 0–5 have shaped peaks, 6–8 a single harmonic each.
const N_PEAKS: usize = 9;

impl<'a> WaveGen<'a> {
    fn new(wavephase: i32, voice: &'a VoiceParams) -> Self {
        WaveGen {
            voice,
            out: Vec::new(),
            wavephase,
            pitch: 0,
            samplecount: 0,
            nsamples: 0,
            hspect: [[0; MAX_HARMONIC]; 2],
            hswitch: 0,
            playing: 0,
            harm_inc: [0; N_LOWHARM],
            maxh: 0,
            maxh2: 0,
            h_switch_sign: 0,
            amplitude2: 0,
            cbytes: 0,
            cycle_count: 0,
            peak_harmonic: [0; N_PEAKS],
            peak_height: [0; N_PEAKS],
            flutter_ix: 0,
            breath: Default::default(),
            noise_seed: voice.rand_seed.max(1),
        }
    }

    /// `amplitude2`: compensated for the fewer harmonics at a higher pitch.
    fn amplitude_for_pitch(&self, amplitude: i64) -> i64 {
        const AMPLITUDE_FMT: i64 = 100;
        amplitude * (self.pitch >> 8) * AMPLITUDE_FMT / (10000 << 3)
    }

    /// `PeaksToHarmspect()` into table `ix`; `first` is the initial call, where
    /// the interpolation increments are not yet wanted.
    #[expect(
        clippy::indexing_slicing,
        reason = "indices are bounded by the loops over these lists"
    )]
    fn fill_table(&mut self, ix: usize, peaks: &[Peak; N_PEAKS], first: bool) -> usize {
        let pitch = self.pitch << 4; // Hz << 16
        let hmax = peaks_to_harmspect(peaks, pitch, &mut self.hspect[ix], &self.voice.tone_adjust);
        let hmax_samplerate = ((22050 * 19 / 40) << 16) / pitch;
        for pk in N_HARMONIC_PEAKS + 1..N_PEAKS {
            let x = peaks.at(pk).height >> 14;
            self.peak_height.put(pk, x * x * 5 / 2);
            if first {
                self.peak_harmonic[pk] = peaks.at(pk).freq / pitch;
            }
            if self.peak_harmonic.at(pk) >= hmax_samplerate {
                self.peak_height.put(pk, 0);
            }
        }
        if !first {
            // The change each 8 samples takes the playing table to this one.
            let (from, to) = (self.hspect[ix ^ 1], self.hspect[ix]);
            for (inc, (to, from)) in self.harm_inc.iter_mut().zip(to.iter().zip(from)) {
                *inc = (to - from) >> 3;
            }
        }
        hmax
    }

    #[expect(
        clippy::indexing_slicing,
        reason = "indices are bounded by the loops over these lists"
    )]
    fn run(&mut self, seg: &Segment, last: bool) {
        // The length is rounded to whole steps and follows on from the last.
        let length = match (seg.length + STEPSIZE / 2) & !(STEPSIZE - 1) {
            0 => STEPSIZE,
            n => n,
        };
        let mut modulation = seg.fr1.modulation;
        let start = self.samplecount;
        self.nsamples += length;
        let (peaks1, peaks2) = (
            frame_to_peaks(seg.fr1, &self.voice.formants),
            frame_to_peaks(seg.fr2, &self.voice.formants),
        );
        let mut peaks = peaks1;
        if start == 0 {
            self.pitch = seg.pitch1.max(25 << 12);
        }
        let mut phaseinc = self.phase_increment();
        let mut cycle_samples = 22050 / (self.pitch >> 12).max(1);
        let mut hf_factor = (self.pitch >> 11).max(1);

        loop {
            if !last && self.samplecount == self.nsamples {
                return;
            }
            // The last segment runs on to the end of its wave cycle; stop in any
            // case well past the longest one.
            if self.samplecount >= self.nsamples + 2 * 22050 / 25 {
                return;
            }
            if self.samplecount % STEPSIZE == 0 {
                let t = ((self.samplecount - start) as f64 / length as f64).min(1.0);
                if self.samplecount == 0 {
                    self.hswitch = 0;
                    self.playing = 0;
                    self.maxh2 = self.fill_table(0, &peaks, true);
                    self.amplitude2 = self.amplitude_for_pitch(seg.amplitude);
                    self.h_switch_sign = (890 / (self.pitch >> 12).max(1)) as usize;
                } else {
                    let pitch = seg.pitch1 + ((seg.pitch2 - seg.pitch1) as f64 * t) as i64;
                    self.pitch = (pitch + self.next_flutter()).max(25 << 12);
                    peaks = interpolate_peaks(&peaks1, &peaks2, t);
                }
                phaseinc = self.phase_increment();
                cycle_samples = 22050 / (self.pitch >> 12).max(1);
                hf_factor = (self.pitch >> 11).max(1);

                self.set_breath(&peaks);
                self.maxh = self.maxh2;
                self.playing = self.hswitch;
                self.hswitch ^= 1;
                self.maxh2 = self.fill_table(self.hswitch, &peaks, false);
            } else if self.samplecount % 8 == 0 {
                let top = N_LOWHARM.min(self.maxh2 + 1).min(self.maxh + 1);
                let table = self.hspect[self.playing].iter_mut().zip(&self.harm_inc);
                for (harmonic, inc) in table.take(top).skip(1) {
                    *harmonic += inc;
                }
            }
            self.samplecount += 1;

            let rising = self.wavephase > 0;
            self.wavephase = self.wavephase.wrapping_add(phaseinc);
            if rising && self.wavephase < 0 {
                // The quiet point of the waveform.
                self.cbytes = WAVEMULT_OFFSET - cycle_samples as i32 / 2;
                if self.samplecount > self.nsamples {
                    return;
                }
                self.cycle_count += 1;
                for pk in N_HARMONIC_PEAKS + 1..N_PEAKS {
                    self.peak_harmonic[pk] = (peaks.at(pk).freq / (self.pitch * 8) + 1) / 2;
                }
                self.amplitude2 = self.amplitude_for_pitch(seg.amplitude);
                // `modulation_tab[roughness]`: some vowels are slightly rough.
                let rough = |amps: [i64; 8]| match amps.at(self.voice.roughness.min(7)) {
                    0 => (16, 1),
                    amp => (amp, 2),
                };
                let (amp, period) = match modulation {
                    1 => rough([0, 0xf, 0xf, 0xe, 0xd, 0xb, 0xa, 0x9]),
                    2 => rough([0, 0, 0, 0, 0xd, 0xb, 0xa, 0x9]),
                    3 => rough([0, 0xf, 0xe, 0xd, 0xc, 0xb, 0xa, 0x9]),
                    // A trill: just one cycle is reduced.
                    6 => {
                        modulation = 0;
                        (2, 1)
                    }
                    _ => (16, 1),
                };
                if self.cycle_count % period == 0 {
                    self.amplitude2 = self.amplitude2 * amp / 16;
                }
            }

            let waveph = (self.wavephase >> 16) as u16;
            let mut total = 0i64;
            // Formants 6–8: one harmonic each, spread by a window.
            self.cbytes += 1;
            if let Some(&window) = usize::try_from(self.cbytes)
                .ok()
                .filter(|&n| n < WAVEMULT_MAX)
                .and_then(|n| WAVEMULT.get(n))
            {
                for pk in N_HARMONIC_PEAKS + 1..N_PEAKS {
                    let theta =
                        (self.peak_harmonic.at(pk) as u32).wrapping_mul(waveph.into()) as u16;
                    total +=
                        i64::from(SIN_TAB.at(usize::from(theta >> 5))) * self.peak_height.at(pk);
                }
                total = total / hf_factor * i64::from(window);
            }
            // Formants 0–5; the sign changes above about 900 Hz to reduce peaks.
            let table = &self.hspect[self.playing];
            let mut theta = waveph;
            for (h, &harmonic) in table.iter().enumerate().take(self.maxh + 1).skip(1) {
                let wave = i64::from(SIN_TAB.at(usize::from(theta >> 5))) * i64::from(harmonic);
                if h <= self.h_switch_sign {
                    total += wave;
                } else {
                    total -= wave;
                }
                theta = theta.wrapping_add(waveph);
            }
            if self.voice.voicing != 64 {
                total = (total >> 6) * i64::from(self.voice.voicing);
            }
            total += self.next_breath(&peaks);
            let sample = ((total >> 8) * self.amplitude2) >> 13;
            self.out
                .push(sample.clamp(i32::MIN.into(), i32::MAX.into()) as i32);
        }
    }

    /// `(wdata.pitch >> 7) * PHASE_INC_FACTOR`.
    fn phase_increment(&self) -> i32 {
        ((self.pitch >> 7) * i64::from(PHASE_INC_FACTOR)) as i32
    }

    /// `SetBreath`: move the breath filters to the formants.
    fn set_breath(&mut self, peaks: &[Peak; N_PEAKS]) {
        let settings = self.voice.breath.iter().zip(&self.voice.breathw);
        for ((filter, peak), (&amp, &width)) in self.breath.iter_mut().zip(peaks).zip(settings) {
            if amp != 0 {
                filter.set(peak.freq >> 16, width);
            }
        }
    }

    /// `ApplyBreath`: the next sample of noise shaped by the formants.
    fn next_breath(&mut self, peaks: &[Peak; N_PEAKS]) -> i64 {
        if self.voice.breath.iter().all(|&amp| amp == 0) {
            return 0;
        }
        self.noise_seed = self
            .noise_seed
            .wrapping_mul(1_103_515_245)
            .wrapping_add(12_345);
        let noise = f64::from((self.noise_seed >> 16) & 0x3fff) - f64::from(0x2000);
        let mut value = 0;
        for ((filter, peak), &amp) in self.breath.iter_mut().zip(peaks).zip(&self.voice.breath) {
            if amp != 0 {
                value += filter.next(noise) as i64 * i64::from(amp) * (peak.height >> 14);
            }
        }
        value
    }

    fn next_flutter(&mut self) -> i64 {
        const FLUTTER_INC: usize = 64;
        if self.flutter_ix >= FLUTTER_TAB.len() * 64 {
            self.flutter_ix = 0;
        }
        let x = FLUTTER_TAB.at(self.flutter_ix >> 6);
        self.flutter_ix += FLUTTER_INC;
        (i64::from(x) - 0x80) * i64::from(self.voice.flutter_amp)
    }
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
    let scale: i64 = if peak > 0 {
        target * 4096 / peak as i64
    } else {
        0
    };

    samples
        .iter()
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
        f.fwidth[0] = 25; // F0 width
        f.fwidth[1] = 50; // F1 width
        f.fwidth[2] = 80; // F2 width
        f.fright[0] = 25;
        f.fright[1] = 50;
        f.fright[2] = 80;
        f.fheight = [100, 60, 40, 20, 10, 5, 2, 0];
        f.rms = 50;
        f.klattp[0] = 50; // AV = 50 (voiced)
        f.length = len;
        f
    }

    fn default_voice() -> VoiceParams {
        VoiceParams::default()
    }

    #[test]
    fn formant_lines_replace_the_defaults() {
        let mut scale = FormantScale::default();
        scale.set(2, 150, 50, 25, 10);
        assert_eq!(scale.freq(2, 1000), 384 * 1000 + 2560);
        assert_eq!((scale.height[2], scale.width[2]), (128, 64));
        // 100 % replaces a default height that isn't 256.
        scale.set(4, 100, 100, -1, 0);
        assert_eq!((scale.height[4], scale.width[4]), (256, 342));
        let before = scale.clone();
        scale.set(9, 1, 1, 1, 1);
        assert_eq!(scale, before, "no such formant");
    }

    #[test]
    fn voicing_scales_and_breath_adds_noise() {
        let frames: Vec<SpectFrame> = (0..8).map(|_| full_voiced_frame(400, 1400, 2400)).collect();
        let render = |voice: VoiceParams| {
            let mut phase = i32::MAX;
            let out =
                synthesize_frames_seq(&frames, &[1.0; 8], &[120.0; 8], &[], &voice, &mut phase);
            out.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>()
        };
        let full = render(VoiceParams::default());
        let quiet = render(VoiceParams {
            voicing: 32,
            ..VoiceParams::default()
        });
        assert!((quiet / full - 0.25).abs() < 0.01, "{}", quiet / full);
        let mut breath = [0; 9];
        breath[1] = -75;
        let noise = render(VoiceParams {
            voicing: 0,
            breath,
            ..VoiceParams::default()
        });
        assert!(noise > 0.0);
    }

    /// Goertzel power at a single frequency (FFT-free single-bin DFT).
    fn goertzel_power(s: &[i32], f_hz: f64, sr: f64) -> f64 {
        let w = 2.0 * std::f64::consts::PI * f_hz / sr;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &x in s {
            let s0 = x as f64 + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0)
    }

    /// A fully-populated voiced frame (all of F1–F5 present, with widths and
    /// heights) so the synthesizer's `hmax` — derived from the F5 peak — is
    /// non-zero and the mid formants actually reach the output.
    fn full_voiced_frame(f1: i16, f2: i16, f3: i16) -> SpectFrame {
        let mut f = SpectFrame::default();
        f.ffreq = [0, f1, f2, f3, 3300, 4300, 5300];
        f.fheight = [0, 100, 80, 60, 45, 30, 15, 0];
        f.fwidth = [40, 55, 70, 90, 110, 130];
        f.fright = [40, 55, 70];
        f.rms = 50;
        f.klattp[0] = 50; // AV = 50 (voiced)
        f.length = 40;
        f
    }

    #[test]
    fn formant_scaling_shifts_energy_upward() {
        // Same voiced frames + pitch; only the per-formant scale differs.  Raising
        // the formants by 60 % moves F2 from 1400 Hz to ~2240 Hz, so the spectral
        // energy *at 2240 Hz* (normalised by total energy) must grow markedly.
        let frames: Vec<SpectFrame> = (0..8).map(|_| full_voiced_frame(400, 1400, 2400)).collect();
        let amps = vec![1.0; frames.len()];
        let pitch = vec![120.0; frames.len()];
        const SR: f64 = 22050.0;

        let probe_energy = |pct: i32, f_hz: f64| -> f64 {
            let voice = VoiceParams {
                formants: FormantScale {
                    freq: [256 * pct / 100; 9],
                    ..FormantScale::default()
                },
                ..VoiceParams::default()
            };
            let mut phase = i32::MAX;
            let s = synthesize_frames_seq(&frames, &amps, &pitch, &[], &voice, &mut phase);
            let total: f64 = s.iter().map(|&x| (x as f64).powi(2)).sum::<f64>().max(1.0);
            goertzel_power(&s, f_hz, SR) / total
        };

        // Probe at the *raised* F2 location (1400 × 1.6 ≈ 2240 Hz).
        let base = probe_energy(100, 2240.0);
        let raised = probe_energy(160, 2240.0);
        assert!(
            raised > base * 2.0,
            "raising formants should push energy to the higher band: base {base:.3e} vs raised {raised:.3e}"
        );
    }

    #[test]
    fn formant_height_scaling_boosts_its_band() {
        // Doubling F2's height (freq unchanged) must raise the raw spectral power
        // at the F2 frequency (1400 Hz).
        let frames: Vec<SpectFrame> = (0..8).map(|_| full_voiced_frame(400, 1400, 2400)).collect();
        let amps = vec![1.0; frames.len()];
        let pitch = vec![120.0; frames.len()];
        const SR: f64 = 22050.0;

        let power = |percent: i32| -> f64 {
            let mut formants = FormantScale::default();
            formants.height[2] = formants.height[2] * percent / 100;
            let voice = VoiceParams {
                formants,
                ..VoiceParams::default()
            };
            let mut phase = i32::MAX;
            let s = synthesize_frames_seq(&frames, &amps, &pitch, &[], &voice, &mut phase);
            goertzel_power(&s, 1400.0, SR)
        };

        let base = power(100);
        let boosted = power(220); // F2 × 2.2
        assert!(
            boosted > base * 1.5,
            "boosting F2 height should raise its band's power: base {base:.3e} vs boosted {boosted:.3e}"
        );
    }

    #[test]
    fn peaks_to_harmspect_nonzero() {
        let frame = voiced_frame(500, 1500, 2500, 4);
        let peaks = frame_to_peaks(&frame, &FormantScale::default());
        let mut htab = [0i32; MAX_HARMONIC];
        let hmax = peaks_to_harmspect(
            &peaks,
            118_i64 << 16,
            &mut htab,
            &set_tone_adjust(&DEFAULT_TONE_POINTS),
        );
        // Should produce non-zero harmonics
        assert!(hmax > 0);
        let total: i32 = htab[1..].iter().sum();
        assert!(total > 0, "harmonic table should be non-zero");
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
        // …and the rest of the last wave cycle.
        let expected = 3 * 2 * 64;
        let cycle = 22050 / default_voice().pitch_hz as usize;
        assert!(
            (expected..=expected + cycle + 1).contains(&samples.len()),
            "expected about {expected} samples, got {}",
            samples.len()
        );
    }

    #[test]
    fn i32_to_i16_normalises() {
        let big = vec![100_000i32, -200_000i32, 150_000i32];
        let out = i32_to_i16(&big);
        assert_eq!(out.len(), 3);
        // Peak should be close to 90% of 32767
        let peak = out.iter().map(|&x| x.unsigned_abs()).max().unwrap();
        assert!(
            peak > 28000,
            "should normalise close to full scale: peak={peak}"
        );
    }

    #[test]
    fn i32_to_i16_empty() {
        assert!(i32_to_i16(&[]).is_empty());
    }

    /// `SetToneAdjust` interpolates between the `tone` points and extends the
    /// last height to the top of the range (the `-1` sentinel).
    #[test]
    fn tone_curve_interpolates_between_points() {
        let t = set_tone_adjust(&DEFAULT_TONE_POINTS);
        // Flat at the first height below the first point (600 Hz = index 75).
        assert_eq!(t[0], 170);
        assert_eq!(t[74], 170);
        // 1200 Hz = index 150 → the second height.
        assert_eq!(t[150], 135);
        // Halfway between 600 and 1200 Hz, halfway between the two heights.
        assert_eq!(t[112] as i32, 170 + (112 - 75) * (135 - 170) / (150 - 75));
        // 2000 Hz = index 250 → 110, and `-1` holds it to the end.
        assert_eq!(t[250], 110);
        assert_eq!(t[N_TONE_ADJUST - 1], 110);
    }

    /// A voice's own curve is applied *relative* to the default, so the standard
    /// voices are unchanged and a `tone` line contributes only its difference.
    #[test]
    fn default_tone_curve_is_a_no_op() {
        let peaks = frame_to_peaks(
            &SpectFrame {
                ffreq: [100, 700, 1200, 2500, 3300, 3700, 7000],
                fheight: [200; 8],
                fwidth: [40; 6],
                fright: [40; 3],
                ..Default::default()
            },
            &FormantScale::default(),
        );
        let mut with_default = [0i32; MAX_HARMONIC];
        let mut without = [0i32; MAX_HARMONIC];
        let flat = [128u8; N_TONE_ADJUST];
        peaks_to_harmspect(
            &peaks,
            118_i64 << 16,
            &mut with_default,
            &set_tone_adjust(&DEFAULT_TONE_POINTS),
        );
        peaks_to_harmspect(&peaks, 118_i64 << 16, &mut without, &DEFAULT_TONE_ADJUST);
        assert_eq!(
            with_default, without,
            "the default curve must not change the spectrum"
        );

        // A flat curve *does* change it: it undoes the default's tilt, so the
        // harmonics above 600 Hz gain relative to those below.
        let mut flattened = [0i32; MAX_HARMONIC];
        peaks_to_harmspect(&peaks, 118_i64 << 16, &mut flattened, &flat);
        assert_ne!(
            flattened, with_default,
            "a different curve must reshape the spectrum"
        );
    }
}
