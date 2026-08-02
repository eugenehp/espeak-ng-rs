//! Pitch-preserving tempo scaling — the role of `libsonic` in upstream espeak-ng
//! (§8).  Changes the *duration* of a PCM buffer without shifting its pitch, by a
//! pitch-synchronous overlap-add (the core idea of Bill Cox's `sonic`): the
//! signal is walked one detected pitch period at a time, and whole periods are
//! **merged** (to speed up) or **repeated** (to slow down) with a short crossfade,
//! so the local waveform shape — and thus the pitch — is preserved while the
//! number of periods (the duration) changes.
//!
//! This is a standalone, self-verifying DSP utility (no `phondata` involved): a
//! caller can post-process synthesized audio to adjust tempo independently of the
//! frame-duration rate control already in the synthesizer.

/// Lowest pitch we track (Hz) → the longest pitch period searched.
const MIN_PITCH_HZ: f64 = 65.0;
/// Highest pitch we track (Hz) → the shortest pitch period searched.
const MAX_PITCH_HZ: f64 = 400.0;

/// Estimate the local pitch period (in samples) of `s` over `[min_p, max_p]`
/// using the Average Magnitude Difference Function (AMDF, as `sonic` does): the
/// period that best repeats the waveform minimises `Σ|s[i] − s[i+p]|`.
fn find_pitch_period(s: &[i16], min_p: usize, max_p: usize) -> usize {
    let max_p = max_p.min(s.len() / 2);
    if max_p <= min_p {
        return min_p.max(1);
    }
    // Compare over one max-period window so every candidate uses the same count.
    let n = max_p;
    let mut best_p = min_p;
    let mut best_diff = i64::MAX;
    for p in min_p..=max_p {
        let mut diff = 0i64;
        for i in 0..n {
            diff += (s[i] as i64 - s[i + p] as i64).abs();
        }
        if diff < best_diff {
            best_diff = diff;
            best_p = p;
        }
    }
    best_p
}

/// Append `n` crossfaded samples: `down` faded out while `up` fades in.
fn overlap_add(out: &mut Vec<i16>, down: &[i16], up: &[i16], n: usize) {
    for i in 0..n {
        let t = (i as f64 + 0.5) / n as f64;
        let v = down[i] as f64 * (1.0 - t) + up[i] as f64 * t;
        out.push(v.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16);
    }
}

/// Time-scale `pcm` by `speed` while preserving pitch: `speed > 1.0` makes it
/// faster (shorter), `speed < 1.0` slower (longer); the output length is
/// approximately `pcm.len() / speed`.  `speed == 1.0` (or a buffer too short to
/// hold two pitch periods) returns an unchanged copy.
pub fn change_tempo(pcm: &[i16], speed: f64, sample_rate: u32) -> Vec<i16> {
    let min_p = (sample_rate as f64 / MAX_PITCH_HZ) as usize;
    let max_p = (sample_rate as f64 / MIN_PITCH_HZ) as usize;
    if !(speed.is_finite()) || (speed - 1.0).abs() < 1e-3 || speed <= 0.0 || pcm.len() < 2 * max_p {
        return pcm.to_vec();
    }

    let mut out: Vec<i16> = Vec::with_capacity((pcm.len() as f64 / speed) as usize + max_p);
    let mut pos = 0usize;

    while pos + 2 * max_p <= pcm.len() {
        let period = find_pitch_period(&pcm[pos..], min_p, max_p).clamp(min_p.max(1), max_p);
        // Where the output "should" be, given how much input we've consumed.
        let expected = pos as f64 / speed;

        if speed > 1.0 && out.len() as f64 > expected {
            // Ahead of schedule → drop one period: merge this period with the
            // next (crossfade), consuming two periods but emitting one.
            overlap_add(&mut out, &pcm[pos..], &pcm[pos + period..], period);
            pos += 2 * period;
        } else if speed < 1.0 && (out.len() as f64) < expected {
            // Behind schedule → insert one period: emit it, then a crossfade
            // into the next, emitting two periods for one consumed.
            out.extend_from_slice(&pcm[pos..pos + period]);
            overlap_add(&mut out, &pcm[pos..], &pcm[pos + period..], period);
            pos += period;
        } else {
            // On schedule → copy one period straight through.
            out.extend_from_slice(&pcm[pos..pos + period]);
            pos += period;
        }
    }

    // Copy whatever tail remains.
    out.extend_from_slice(&pcm[pos..]);
    out
}

/// Linearly resample `s` to exactly `target_len` samples.
fn resample_linear(s: &[i16], target_len: usize) -> Vec<i16> {
    if s.is_empty() || target_len == 0 {
        return Vec::new();
    }
    if s.len() == target_len {
        return s.to_vec();
    }
    let last = (s.len() - 1) as f64;
    let denom = (target_len - 1).max(1) as f64;
    (0..target_len)
        .map(|i| {
            let src = i as f64 * last / denom;
            let idx = src.floor() as usize;
            let frac = src - idx as f64;
            let a = s[idx] as f64;
            let b = s[(idx + 1).min(s.len() - 1)] as f64;
            (a + (b - a) * frac).round().clamp(i16::MIN as f64, i16::MAX as f64) as i16
        })
        .collect()
}

/// Shift the pitch of `pcm` by `factor` while keeping its **duration** — the
/// counterpart to [`change_tempo`].  `factor > 1.0` raises the pitch, `< 1.0`
/// lowers it; the output length equals `pcm.len()`.
///
/// Method (the standard `sonic` pitch-shift): time-stretch to `factor×` length
/// preserving pitch, then linearly resample back to the original length — the
/// stretched waveform "played faster" comes out `factor×` higher in pitch at the
/// original duration.  `factor == 1.0` (or empty input) returns a copy.
pub fn change_pitch(pcm: &[i16], factor: f64, sample_rate: u32) -> Vec<i16> {
    if !factor.is_finite() || (factor - 1.0).abs() < 1e-3 || factor <= 0.0 || pcm.is_empty() {
        return pcm.to_vec();
    }
    let stretched = change_tempo(pcm, 1.0 / factor, sample_rate);
    resample_linear(&stretched, pcm.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 22050;

    /// A pure sine at `freq` Hz for `n` samples.
    fn sine(freq: f64, n: usize) -> Vec<i16> {
        (0..n)
            .map(|i| {
                let t = i as f64 / SR as f64;
                (10_000.0 * (2.0 * std::f64::consts::PI * freq * t).sin()) as i16
            })
            .collect()
    }

    /// Estimate frequency from mean spacing between positive-going zero crossings.
    fn est_freq(s: &[i16]) -> f64 {
        let mut crossings = Vec::new();
        for i in 1..s.len() {
            if s[i - 1] < 0 && s[i] >= 0 {
                crossings.push(i);
            }
        }
        if crossings.len() < 2 {
            return 0.0;
        }
        let span = (crossings[crossings.len() - 1] - crossings[0]) as f64;
        let periods = (crossings.len() - 1) as f64;
        SR as f64 * periods / span
    }

    #[test]
    fn speed_one_is_unchanged() {
        let s = sine(200.0, 8000);
        assert_eq!(change_tempo(&s, 1.0, SR), s);
    }

    #[test]
    fn faster_shortens_but_keeps_pitch() {
        let s = sine(200.0, 20_000);
        let fast = change_tempo(&s, 2.0, SR);
        // ~half the length.
        let ratio = fast.len() as f64 / s.len() as f64;
        assert!((ratio - 0.5).abs() < 0.12, "expected ~0.5x length, got {ratio:.3}");
        // Pitch preserved (not doubled, as naive resampling would).
        let f = est_freq(&fast);
        assert!((f - 200.0).abs() < 20.0, "pitch should stay ~200 Hz, got {f:.0}");
    }

    #[test]
    fn slower_lengthens_but_keeps_pitch() {
        let s = sine(150.0, 20_000);
        let slow = change_tempo(&s, 0.5, SR);
        let ratio = slow.len() as f64 / s.len() as f64;
        assert!((ratio - 2.0).abs() < 0.25, "expected ~2x length, got {ratio:.3}");
        let f = est_freq(&slow);
        assert!((f - 150.0).abs() < 15.0, "pitch should stay ~150 Hz, got {f:.0}");
    }

    #[test]
    fn short_or_degenerate_input_is_passthrough() {
        let s = sine(200.0, 100); // shorter than two max periods
        assert_eq!(change_tempo(&s, 2.0, SR), s);
        let s2 = sine(200.0, 20_000);
        assert_eq!(change_tempo(&s2, 0.0, SR), s2); // non-positive speed
    }

    #[test]
    fn raise_pitch_keeps_duration() {
        let s = sine(200.0, 20_000);
        let up = change_pitch(&s, 1.5, SR);
        // Duration unchanged.
        assert_eq!(up.len(), s.len(), "pitch shift must preserve length");
        // Pitch raised ~1.5× (200 → ~300 Hz).
        let f = est_freq(&up);
        assert!((f - 300.0).abs() < 30.0, "expected ~300 Hz, got {f:.0}");
    }

    #[test]
    fn lower_pitch_keeps_duration() {
        let s = sine(300.0, 20_000);
        let down = change_pitch(&s, 0.5, SR);
        assert_eq!(down.len(), s.len());
        let f = est_freq(&down);
        assert!((f - 150.0).abs() < 20.0, "expected ~150 Hz, got {f:.0}");
    }

    #[test]
    fn pitch_factor_one_is_unchanged() {
        let s = sine(200.0, 8000);
        assert_eq!(change_pitch(&s, 1.0, SR), s);
    }
}
