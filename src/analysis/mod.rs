//! Basic PCM / WAV analysis for verifying synthesis output.
//!
//! Enabled by the `wav-analysis` Cargo feature.  This is a **dev / testing**
//! utility: because the Rust synthesizer will not produce bit-identical PCM to
//! the C reference, we instead compare *derived acoustic features* — total
//! duration, energy (RMS) envelope, and fundamental-frequency (pitch) contour.
//! That is enough to verify prosody work such as intonation and phoneme
//! durations, and to detect inserted events such as sound icons.
//!
//! No external dependencies: WAV parsing and the DSP are implemented here.
//!
//! ```no_run
//! # #[cfg(feature = "wav-analysis")] {
//! use espeak_ng::analysis::{Pcm, compare};
//! let rust = Pcm::from_samples(espeak_ng::text_to_pcm("en", "hello").unwrap().0
//!     .iter().map(|&s| s).collect(), 22_050);
//! let reference = Pcm::read_wav("oracle.wav").unwrap();
//! let sim = compare(&reference, &rust);
//! assert!(sim.energy_corr > 0.7, "energy envelope diverged: {sim:?}");
//! # }
//! ```

use std::fs;
use std::io;
use std::path::Path;

/// Decoded mono 16-bit PCM at a known sample rate.
#[derive(Debug, Clone)]
pub struct Pcm {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
}

/// Options for [`Pcm::pitch_track`].
#[derive(Debug, Clone, Copy)]
pub struct PitchOptions {
    /// Analysis window length in samples.
    pub frame: usize,
    /// Hop between successive frames in samples.
    pub hop: usize,
    /// Lowest F0 to search for (Hz).
    pub fmin: f32,
    /// Highest F0 to search for (Hz).
    pub fmax: f32,
    /// Minimum normalised autocorrelation peak to call a frame voiced.
    pub voicing_threshold: f32,
}

impl Default for PitchOptions {
    fn default() -> Self {
        PitchOptions {
            frame: 1024,
            hop: 256,
            fmin: 60.0,
            fmax: 400.0,
            voicing_threshold: 0.15,
        }
    }
}

impl Pcm {
    /// Wrap raw mono samples.
    pub fn from_samples(samples: Vec<i16>, sample_rate: u32) -> Self {
        Pcm { samples, sample_rate }
    }

    /// Read a 16-bit PCM WAV file (mono, or the mean of all channels).
    pub fn read_wav<P: AsRef<Path>>(path: P) -> io::Result<Pcm> {
        Self::parse_wav(&fs::read(path)?)
    }

    /// Parse a 16-bit PCM WAV from bytes.
    pub fn parse_wav(bytes: &[u8]) -> io::Result<Pcm> {
        let err = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err(err("not a RIFF/WAVE file"));
        }
        let mut channels = 1u16;
        let mut sample_rate = 22_050u32;
        let mut bits = 16u16;
        let mut data: Option<&[u8]> = None;

        // Walk the chunks after the 12-byte RIFF header.
        let mut pos = 12;
        while pos + 8 <= bytes.len() {
            let id = &bytes[pos..pos + 4];
            let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
            let body_start = pos + 8;
            let body_end = (body_start + size).min(bytes.len());
            match id {
                b"fmt " if size >= 16 => {
                    channels = u16::from_le_bytes(bytes[body_start + 2..body_start + 4].try_into().unwrap());
                    sample_rate = u32::from_le_bytes(bytes[body_start + 4..body_start + 8].try_into().unwrap());
                    bits = u16::from_le_bytes(bytes[body_start + 14..body_start + 16].try_into().unwrap());
                }
                b"data" => data = Some(&bytes[body_start..body_end]),
                _ => {}
            }
            // Chunks are word-aligned (padded to even length).
            pos = body_start + size + (size & 1);
        }

        if bits != 16 {
            return Err(err("only 16-bit PCM WAV is supported"));
        }
        let data = data.ok_or_else(|| err("no data chunk"))?;
        let ch = channels.max(1) as usize;
        let frames = data.len() / (2 * ch);
        let mut samples = Vec::with_capacity(frames);
        for f in 0..frames {
            // Average channels down to mono.
            let mut acc = 0i32;
            for c in 0..ch {
                let o = (f * ch + c) * 2;
                acc += i16::from_le_bytes([data[o], data[o + 1]]) as i32;
            }
            samples.push((acc / ch as i32) as i16);
        }
        Ok(Pcm { samples, sample_rate })
    }

    /// Serialize as a 16-bit mono PCM WAV.
    pub fn to_wav(&self) -> Vec<u8> {
        let data_len = self.samples.len() * 2;
        let mut out = Vec::with_capacity(44 + data_len);
        let byte_rate = self.sample_rate * 2;
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&self.sample_rate.to_le_bytes());
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes()); // block align
        out.extend_from_slice(&16u16.to_le_bytes()); // bits
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data_len as u32).to_le_bytes());
        for &s in &self.samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    /// Duration in seconds.
    pub fn duration_secs(&self) -> f32 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.samples.len() as f32 / self.sample_rate as f32
    }

    /// Root-mean-square amplitude over the whole signal (0.0–1.0 scale).
    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.samples.iter().map(|&s| {
            let v = s as f64 / i16::MAX as f64;
            v * v
        }).sum();
        (sum / self.samples.len() as f64).sqrt() as f32
    }

    /// Per-frame RMS energy envelope.
    pub fn energy_envelope(&self, frame: usize, hop: usize) -> Vec<f32> {
        let frame = frame.max(1);
        let hop = hop.max(1);
        let mut out = Vec::new();
        let mut start = 0;
        while start < self.samples.len() {
            let end = (start + frame).min(self.samples.len());
            let win = &self.samples[start..end];
            let sum: f64 = win.iter().map(|&s| {
                let v = s as f64 / i16::MAX as f64;
                v * v
            }).sum();
            out.push((sum / win.len().max(1) as f64).sqrt() as f32);
            start += hop;
        }
        out
    }

    /// Per-frame fundamental frequency in Hz (0.0 for unvoiced frames).
    ///
    /// Uses normalised autocorrelation with parabolic peak interpolation.
    pub fn pitch_track(&self, opts: PitchOptions) -> Vec<f32> {
        let sr = self.sample_rate as f32;
        let min_lag = (sr / opts.fmax).floor() as usize;
        let max_lag = (sr / opts.fmin).ceil() as usize;
        let frame = opts.frame.max(max_lag + 1);
        let mut out = Vec::new();
        let mut start = 0;
        while start + frame <= self.samples.len() {
            let win: Vec<f64> = self.samples[start..start + frame]
                .iter()
                .map(|&s| s as f64)
                .collect();
            out.push(estimate_f0(&win, sr, min_lag, max_lag, opts.voicing_threshold));
            start += opts.hop;
        }
        out
    }

    /// Mean F0 over voiced frames, or `None` if the signal is entirely unvoiced.
    pub fn mean_pitch(&self, opts: PitchOptions) -> Option<f32> {
        let voiced: Vec<f32> = self.pitch_track(opts).into_iter().filter(|&f| f > 0.0).collect();
        if voiced.is_empty() {
            None
        } else {
            Some(voiced.iter().sum::<f32>() / voiced.len() as f32)
        }
    }

    // ── Spectral analysis ─────────────────────────────────────────────────

    /// Average magnitude spectrum over Hann-windowed frames.
    ///
    /// Returns `fft_size/2 + 1` bins; bin `k` is centred at
    /// `k * sample_rate / fft_size` Hz (see [`Pcm::bin_hz`]).  `fft_size` is
    /// rounded up to a power of two.  A signal shorter than one frame is
    /// zero-padded to a single frame.
    pub fn avg_spectrum(&self, fft_size: usize, hop: usize) -> Vec<f32> {
        let n = fft_size.next_power_of_two().max(2);
        let hop = hop.max(1);
        let half = n / 2 + 1;
        let hann: Vec<f64> = (0..n)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos())
            .collect();

        let mut acc = vec![0f64; half];
        let mut frames = 0usize;
        let mut start = 0;
        while start + n <= self.samples.len() {
            self.accumulate_frame(start, n, &hann, &mut acc);
            frames += 1;
            start += hop;
        }
        if frames == 0 {
            // Shorter than one frame: analyse a single zero-padded frame.
            self.accumulate_frame(0, n, &hann, &mut acc);
            frames = 1;
        }
        acc.iter().map(|&v| (v / frames as f64) as f32).collect()
    }

    /// FFT one Hann-windowed frame at `start` and add its magnitude to `acc`.
    fn accumulate_frame(&self, start: usize, n: usize, hann: &[f64], acc: &mut [f64]) {
        let mut re = vec![0f64; n];
        let mut im = vec![0f64; n];
        for i in 0..n {
            if let Some(&s) = self.samples.get(start + i) {
                re[i] = s as f64 / i16::MAX as f64 * hann[i];
            }
        }
        fft(&mut re, &mut im);
        for (k, a) in acc.iter_mut().enumerate() {
            *a += (re[k] * re[k] + im[k] * im[k]).sqrt();
        }
    }

    /// Hz per spectrum bin for a given `fft_size` (rounded up to a power of two).
    pub fn bin_hz(&self, fft_size: usize) -> f32 {
        self.sample_rate as f32 / fft_size.next_power_of_two().max(2) as f32
    }

    /// Spectral centroid (Hz): the magnitude-weighted mean frequency
    /// ("brightness").  Distinguishes fricatives — /s/ ≈ 5–8 kHz vs /ʃ/ ≈ 2–4 kHz.
    pub fn spectral_centroid(&self, fft_size: usize, hop: usize) -> f32 {
        let spec = self.avg_spectrum(fft_size, hop);
        let bin_hz = self.bin_hz(fft_size);
        let (mut num, mut den) = (0f64, 0f64);
        for (k, &m) in spec.iter().enumerate() {
            num += (k as f32 * bin_hz) as f64 * m as f64;
            den += m as f64;
        }
        if den > 0.0 { (num / den) as f32 } else { 0.0 }
    }

    /// Spectral flatness (0..1): geometric mean ÷ arithmetic mean of the power
    /// spectrum.  Near 1 ⇒ noise-like (fricatives/stops); near 0 ⇒ tonal (vowels).
    pub fn spectral_flatness(&self, fft_size: usize, hop: usize) -> f32 {
        let spec = self.avg_spectrum(fft_size, hop);
        let bins = if spec.len() > 1 { &spec[1..] } else { &spec[..] }; // skip DC
        if bins.is_empty() {
            return 0.0;
        }
        let (mut log_sum, mut sum) = (0f64, 0f64);
        for &m in bins {
            let p = (m as f64 * m as f64).max(1e-12);
            log_sum += p.ln();
            sum += p;
        }
        let count = bins.len() as f64;
        let gmean = (log_sum / count).exp();
        let amean = sum / count;
        if amean > 0.0 { (gmean / amean) as f32 } else { 0.0 }
    }

    /// Spectral roll-off (Hz): frequency below which `pct` (0..1) of the total
    /// magnitude lies.  High for /s/, lower for /ʃ/ and vowels.
    pub fn spectral_rolloff(&self, fft_size: usize, hop: usize, pct: f32) -> f32 {
        let spec = self.avg_spectrum(fft_size, hop);
        let bin_hz = self.bin_hz(fft_size);
        let total: f64 = spec.iter().map(|&m| m as f64).sum();
        if total <= 0.0 {
            return 0.0;
        }
        let target = total * pct.clamp(0.0, 1.0) as f64;
        let mut acc = 0f64;
        for (k, &m) in spec.iter().enumerate() {
            acc += m as f64;
            if acc >= target {
                return k as f32 * bin_hz;
            }
        }
        (spec.len().saturating_sub(1)) as f32 * bin_hz
    }

    /// Fraction of spectral *power* in the band `[lo, hi]` Hz (0..1).
    pub fn band_energy(&self, fft_size: usize, hop: usize, lo: f32, hi: f32) -> f32 {
        let spec = self.avg_spectrum(fft_size, hop);
        let bin_hz = self.bin_hz(fft_size);
        let (mut band, mut total) = (0f64, 0f64);
        for (k, &m) in spec.iter().enumerate() {
            let f = k as f32 * bin_hz;
            let p = m as f64 * m as f64;
            total += p;
            if f >= lo && f <= hi {
                band += p;
            }
        }
        if total > 0.0 { (band / total) as f32 } else { 0.0 }
    }
}

/// Similarity between a reference signal `a` and a candidate `b`.
#[derive(Debug, Clone)]
pub struct Similarity {
    /// `b.duration / a.duration` (1.0 = same length).
    pub dur_ratio: f32,
    /// `b.rms / a.rms` (1.0 = same loudness).
    pub rms_ratio: f32,
    /// Pearson correlation of the (length-normalised) energy envelopes.
    pub energy_corr: f32,
    /// Pearson correlation of the voiced pitch contours (`NaN` if too few
    /// voiced frames overlap).
    pub pitch_corr: f32,
    /// Mean voiced F0 of `a` and `b` (Hz), if any.
    pub mean_pitch_a: Option<f32>,
    pub mean_pitch_b: Option<f32>,
    /// Pearson correlation of the log average-magnitude spectra (spectral
    /// shape/timbre similarity, 1.0 = identical shape).
    pub spectral_corr: f32,
    /// Spectral centroid ("brightness", Hz) of `a` and `b`.
    pub centroid_a: f32,
    pub centroid_b: f32,
}

/// Compare two signals on duration, energy envelope and pitch contour.
pub fn compare(a: &Pcm, b: &Pcm) -> Similarity {
    let da = a.duration_secs();
    let db = b.duration_secs();
    let ra = a.rms();
    let rb = b.rms();

    // Energy envelopes resampled to a common length for correlation.
    let ea = a.energy_envelope(1024, 256);
    let eb = b.energy_envelope(1024, 256);
    let n = ea.len().min(eb.len()).max(1);
    let energy_corr = pearson(&resample(&ea, n), &resample(&eb, n));

    let opts = PitchOptions::default();
    let pa = a.pitch_track(opts);
    let pb = b.pitch_track(opts);
    let m = pa.len().min(pb.len()).max(1);
    let (rpa, rpb) = (resample(&pa, m), resample(&pb, m));
    // Correlate only where both frames are voiced.
    let (mut va, mut vb) = (Vec::new(), Vec::new());
    for i in 0..m {
        if rpa[i] > 0.0 && rpb[i] > 0.0 {
            va.push(rpa[i]);
            vb.push(rpb[i]);
        }
    }
    let pitch_corr = if va.len() >= 3 { pearson(&va, &vb) } else { f32::NAN };

    Similarity {
        dur_ratio: if da > 0.0 { db / da } else { 0.0 },
        rms_ratio: if ra > 0.0 { rb / ra } else { 0.0 },
        energy_corr,
        pitch_corr,
        mean_pitch_a: a.mean_pitch(opts),
        mean_pitch_b: b.mean_pitch(opts),
        spectral_corr: spectral_similarity(a, b, 1024, 256),
        centroid_a: a.spectral_centroid(1024, 256),
        centroid_b: b.spectral_centroid(1024, 256),
    }
}

/// Correlation of the log average-magnitude spectra of two signals — a
/// timbre/spectral-shape similarity that is deterministic (no ASR needed) and
/// well suited to comparing consonant (fricative/stop) spectral character
/// against the C reference.  Both signals must share a sample rate.
pub fn spectral_similarity(a: &Pcm, b: &Pcm, fft_size: usize, hop: usize) -> f32 {
    let sa = a.avg_spectrum(fft_size, hop);
    let sb = b.avg_spectrum(fft_size, hop);
    let n = sa.len().min(sb.len());
    if n < 2 {
        return f32::NAN;
    }
    // Log-magnitude de-emphasises the dominant low-frequency peaks so the
    // comparison reflects overall spectral shape, not just the loudest band.
    let la: Vec<f32> = sa[..n].iter().map(|&m| (m + 1e-6).ln()).collect();
    let lb: Vec<f32> = sb[..n].iter().map(|&m| (m + 1e-6).ln()).collect();
    pearson(&la, &lb)
}

// ---------------------------------------------------------------------------
// DSP helpers
// ---------------------------------------------------------------------------

/// In-place iterative radix-2 Cooley–Tukey FFT.  `re`/`im` must be the same
/// length and a power of two.  A dependency-free FFT so `wav-analysis` needs no
/// external crate.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    if n <= 1 {
        return;
    }
    debug_assert!(n.is_power_of_two() && im.len() == n);

    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    // Butterfly stages.
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut base = 0;
        while base < n {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..len / 2 {
                let a = base + k;
                let b = base + k + len / 2;
                let tr = cr * re[b] - ci * im[b];
                let ti = cr * im[b] + ci * re[b];
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
            base += len;
        }
        len <<= 1;
    }
}

/// Estimate F0 of one frame with the YIN algorithm (de Cheveigné & Kawahara,
/// 2002).  YIN's cumulative-mean-normalised difference function locks onto the
/// *fundamental* period, avoiding the octave/formant errors that plague plain
/// autocorrelation on speech.  Returns 0.0 for unvoiced frames.
///
/// `thresh` is the voicing threshold on the normalised difference (lower =
/// stricter; a frame is voiced when some lag dips below it).
fn estimate_f0(win: &[f64], sr: f32, min_lag: usize, max_lag: usize, thresh: f32) -> f32 {
    let n = win.len();
    if n == 0 || max_lag >= n {
        return 0.0;
    }
    // Enough energy to be speech at all?
    let energy: f64 = win.iter().map(|&v| v * v).sum();
    if energy <= 1.0 {
        return 0.0;
    }

    // Difference function d(tau).
    let mut d = vec![0.0f64; max_lag + 1];
    for tau in min_lag..=max_lag {
        let mut sum = 0.0;
        for j in 0..(n - tau) {
            let diff = win[j] - win[j + tau];
            sum += diff * diff;
        }
        d[tau] = sum;
    }

    // Cumulative mean normalised difference d'(tau).
    let mut cmnd = vec![1.0f64; max_lag + 1];
    let mut running = 0.0;
    for tau in 1..=max_lag {
        running += d[tau];
        cmnd[tau] = if running > 0.0 { d[tau] * tau as f64 / running } else { 1.0 };
    }

    // Absolute threshold: first local minimum below `thresh`.
    let yin_thresh = thresh as f64;
    let mut best_tau = 0usize;
    let mut tau = min_lag.max(1);
    while tau <= max_lag {
        if cmnd[tau] < yin_thresh {
            while tau + 1 <= max_lag && cmnd[tau + 1] < cmnd[tau] {
                tau += 1;
            }
            best_tau = tau;
            break;
        }
        tau += 1;
    }
    // No dip below threshold → take the global minimum, but only if it is a
    // reasonably strong periodicity (else the frame is unvoiced).
    if best_tau == 0 {
        let mut mval = f64::INFINITY;
        for t in min_lag..=max_lag {
            if cmnd[t] < mval {
                mval = cmnd[t];
                best_tau = t;
            }
        }
        if best_tau == 0 || mval > 0.5 {
            return 0.0;
        }
    }

    // Parabolic interpolation on d() around the chosen lag.
    let lag = if best_tau > min_lag && best_tau < max_lag {
        let (a, b, c) = (d[best_tau - 1], d[best_tau], d[best_tau + 1]);
        let denom = a - 2.0 * b + c;
        let delta = if denom.abs() > 1e-9 { 0.5 * (a - c) / denom } else { 0.0 };
        best_tau as f64 + delta.clamp(-1.0, 1.0)
    } else {
        best_tau as f64
    };
    if lag > 0.0 { (sr as f64 / lag) as f32 } else { 0.0 }
}

/// Pearson correlation of two equal-length slices (`NaN` if undefined).
fn pearson(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return f32::NAN;
    }
    let (ma, mb) = (mean(&a[..n]), mean(&b[..n]));
    let mut num = 0.0f64;
    let mut da = 0.0f64;
    let mut db = 0.0f64;
    for i in 0..n {
        let xa = a[i] as f64 - ma as f64;
        let xb = b[i] as f64 - mb as f64;
        num += xa * xb;
        da += xa * xa;
        db += xb * xb;
    }
    if da <= 0.0 || db <= 0.0 {
        return f32::NAN;
    }
    (num / (da.sqrt() * db.sqrt())) as f32
}

fn mean(v: &[f32]) -> f32 {
    if v.is_empty() { 0.0 } else { v.iter().sum::<f32>() / v.len() as f32 }
}

/// Linear-interpolation resample of `v` to exactly `n` points.
fn resample(v: &[f32], n: usize) -> Vec<f32> {
    if v.is_empty() || n == 0 {
        return vec![0.0; n];
    }
    if v.len() == 1 {
        return vec![v[0]; n];
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 * (v.len() - 1) as f32 / (n.max(2) - 1) as f32;
        let lo = t.floor() as usize;
        let hi = (lo + 1).min(v.len() - 1);
        let frac = t - lo as f32;
        out.push(v[lo] * (1.0 - frac) + v[hi] * frac);
    }
    out
}

// ===========================================================================
// Tests — validated on synthetic signals, no oracle required.
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn sine(freq: f32, secs: f32, rate: u32, amp: f32) -> Pcm {
        let n = (secs * rate as f32) as usize;
        let samples = (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                (amp * (TAU * freq * t).sin() * i16::MAX as f32) as i16
            })
            .collect();
        Pcm::from_samples(samples, rate)
    }

    #[test]
    fn duration_and_rms() {
        let s = sine(220.0, 1.0, 22_050, 0.5);
        assert!((s.duration_secs() - 1.0).abs() < 0.01);
        // RMS of a sine of amplitude 0.5 is 0.5/sqrt(2) ≈ 0.354.
        assert!((s.rms() - 0.354).abs() < 0.02, "rms={}", s.rms());
    }

    #[test]
    fn pitch_track_detects_tone() {
        for freq in [110.0, 220.0, 330.0] {
            let s = sine(freq, 0.5, 22_050, 0.6);
            let f0 = s.mean_pitch(PitchOptions::default()).unwrap();
            assert!((f0 - freq).abs() / freq < 0.03, "expected {freq}, got {f0}");
        }
    }

    #[test]
    fn silence_is_unvoiced() {
        let s = Pcm::from_samples(vec![0i16; 22_050], 22_050);
        assert_eq!(s.mean_pitch(PitchOptions::default()), None);
    }

    #[test]
    fn wav_roundtrip() {
        let s = sine(200.0, 0.25, 22_050, 0.4);
        let bytes = s.to_wav();
        let back = Pcm::parse_wav(&bytes).unwrap();
        assert_eq!(back.sample_rate, 22_050);
        assert_eq!(back.samples.len(), s.samples.len());
        assert_eq!(back.samples[100], s.samples[100]);
    }

    #[test]
    fn identical_signals_are_maximally_similar() {
        let s = sine(150.0, 0.5, 22_050, 0.5);
        let sim = compare(&s, &s);
        assert!((sim.dur_ratio - 1.0).abs() < 0.01);
        assert!((sim.rms_ratio - 1.0).abs() < 0.01);
        assert!(sim.energy_corr > 0.99 || sim.energy_corr.is_nan());
    }

    #[test]
    fn amplitude_scaling_shows_in_rms_not_pitch() {
        let a = sine(180.0, 0.5, 22_050, 0.6);
        let b = sine(180.0, 0.5, 22_050, 0.3); // half amplitude
        let sim = compare(&a, &b);
        assert!((sim.rms_ratio - 0.5).abs() < 0.03, "rms_ratio={}", sim.rms_ratio);
        // Same pitch → mean pitches match closely.
        let (pa, pb) = (sim.mean_pitch_a.unwrap(), sim.mean_pitch_b.unwrap());
        assert!((pa - pb).abs() < 3.0);
    }

    #[test]
    fn different_pitch_detected() {
        let a = sine(120.0, 0.5, 22_050, 0.5);
        let b = sine(240.0, 0.5, 22_050, 0.5);
        let sim = compare(&a, &b);
        assert!(sim.mean_pitch_b.unwrap() > sim.mean_pitch_a.unwrap() * 1.8);
    }

    // ── Spectral analysis ─────────────────────────────────────────────────

    fn white_noise(secs: f32, rate: u32, amp: f32) -> Pcm {
        // Deterministic LCG so the test is reproducible.
        let n = (secs * rate as f32) as usize;
        let mut state: u32 = 0x1234_5678;
        let samples = (0..n)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let v = (state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
                (v * amp * i16::MAX as f32) as i16
            })
            .collect();
        Pcm::from_samples(samples, rate)
    }

    #[test]
    fn fft_of_sine_peaks_at_its_frequency() {
        let sr = 22_050u32;
        let fft_size = 2048;
        let s = sine(2000.0, 0.5, sr, 0.8);
        let spec = s.avg_spectrum(fft_size, 512);
        // Peak bin should map to ~2000 Hz.
        let (peak_bin, _) = spec
            .iter()
            .enumerate()
            .skip(1) // ignore DC
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();
        let peak_hz = peak_bin as f32 * s.bin_hz(fft_size);
        assert!((peak_hz - 2000.0).abs() < 40.0, "peak at {peak_hz} Hz");
    }

    #[test]
    fn centroid_tracks_frequency() {
        let lo = sine(1000.0, 0.4, 22_050, 0.7).spectral_centroid(2048, 512);
        let hi = sine(5000.0, 0.4, 22_050, 0.7).spectral_centroid(2048, 512);
        assert!(hi > lo * 2.0, "centroids lo={lo} hi={hi}");
        // A 5 kHz tone's centroid should be near 5 kHz.
        assert!((hi - 5000.0).abs() < 400.0, "hi centroid {hi}");
    }

    #[test]
    fn flatness_separates_tone_from_noise() {
        let tone = sine(1500.0, 0.4, 22_050, 0.7).spectral_flatness(2048, 512);
        let noise = white_noise(0.4, 22_050, 0.5).spectral_flatness(2048, 512);
        assert!(noise > tone * 5.0, "tone={tone} noise={noise}");
        assert!(tone < 0.1, "tone flatness {tone}");
    }

    #[test]
    fn spectral_similarity_identical_vs_different() {
        let a = sine(2000.0, 0.4, 22_050, 0.6);
        let same = spectral_similarity(&a, &a, 1024, 256);
        assert!(same > 0.99, "identical spectral_corr={same}");
        let b = sine(6000.0, 0.4, 22_050, 0.6);
        let diff = spectral_similarity(&a, &b, 1024, 256);
        assert!(diff < same, "diff={diff} should be < same={same}");
    }

    #[test]
    fn rolloff_and_band_energy() {
        let s = sine(3000.0, 0.4, 22_050, 0.7);
        let ro = s.spectral_rolloff(2048, 512, 0.85);
        assert!(ro >= 2000.0 && ro <= 4500.0, "rolloff {ro}");
        // Most power is in the 2–4 kHz band for a 3 kHz tone.
        let be = s.band_energy(2048, 512, 2000.0, 4000.0);
        assert!(be > 0.7, "band energy {be}");
    }
}
