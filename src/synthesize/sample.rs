//! WAV noise-sample playback from the `phondata` binary.

// src/synthesize/sample.rs
//
// WAV noise-sample playback from the phondata binary.
//
// espeak-ng stores pre-recorded PCM samples for stops and fricatives
// directly in the `phondata` file.  The address of each sample is
// provided by the `i_WAV` (0xc000) instruction in the phoneme bytecode.
//
// Binary layout at `wav_addr` (mirrors DoSample2 in synthesize.c):
//
//   byte 0-1: wav_length  — total sample data length in bytes (LE u16)
//   byte 2  : wav_scale   — 0 = 16-bit LE samples; >0 = 8-bit scaled
//   byte 3  : (reserved / padding)
//   bytes 4…: raw sample data
//
// Amplitude formula (mirrors DoSample3 / wavegen PlaySamples):
//   consonant_amp = 26 (wavegen.c)
//   general_amplitude = 55  (GetAmplitude, default EMBED_A=100)
//   amp = consonant_amp * general_amplitude / 16 = 26*55/16 = 89
//   output_sample = (raw_sample * amp) >> 8

/// Consonant amplitude (from wavegen.c constant `consonant_amp`).
pub const CONSONANT_AMP: i32 = 26;

/// Default general amplitude (GetAmplitude with EMBED_A=100).
pub const GENERAL_AMPLITUDE: i32 = 55;

/// Parse and decode a WAV noise sample from phondata.
///
/// Returns the decoded PCM samples as `Vec<i16>`, or `None` if the address
/// is out of range or the data is malformed.
///
/// `addr`        — address in `phondata` (from `PhonemeExtract::wav_addr`).
/// `phondata`    — the full phondata binary.
/// `wav_param`   — the phoneme's `sound_param[pd_WAV]` (signed).  `0` → default
///                 100, mirroring `DoSample3`: `amp2 = (param?param:100)*32/100`.
pub fn parse_wav_sample(
    addr: u32,
    phondata: &[u8],
    speed_factor: f64,
    wav_param: i32,
) -> Option<Vec<i16>> {
    let idx = (addr as usize) & 0x7f_ffff;
    if idx + 4 > phondata.len() {
        return None;
    }

    // Header
    let wav_length = (phondata[idx] as usize) | ((phondata[idx + 1] as usize) << 8);
    let wav_scale  = phondata[idx + 2] as u16;

    if wav_length == 0 {
        return None;
    }

    let data = &phondata[idx + 4..];
    if data.len() < wav_length {
        return None;
    }

    // Decode samples
    let raw: Vec<i32> = if wav_scale == 0 {
        // 16-bit little-endian
        let n = wav_length / 2;
        (0..n)
            .map(|i| {
                let lo = data[i * 2] as i32;
                let hi = (data[i * 2 + 1] as i8) as i32;
                lo | (hi << 8)
            })
            .collect()
    } else {
        // 8-bit signed, scaled
        (0..wav_length)
            .map(|i| (data[i] as i8) as i32 * wav_scale as i32)
            .collect()
    };

    if raw.is_empty() {
        return None;
    }

    // Amplitude (mirrors DoSample3 + PlayWave in synthesize.c / wavegen.c):
    //   amp2  = (wav_param==0 ? 100 : wav_param) * 32 / 100     [DoSample3]
    //   value = raw * consonant_amp * general_amplitude >> 10   [PlayWave]
    //   value = value * amp2 / 32
    // The previous code used `(raw * 89) >> 8` (≈ raw*0.35), ~4× too quiet, and
    // ignored the phoneme's WAV amplitude parameter.
    let amp2 = {
        let p = if wav_param == 0 { 100 } else { wav_param };
        p * 32 / 100
    };

    // Target length in output samples.
    let target_len = (((raw.len() as f64) * speed_factor) as usize).max(1);
    let wav_length = raw.len();
    let len4 = wav_length / 4;

    // Build the play sequence exactly as C `DoSample2`/`PlayWave` does — a noise
    // sample is *not* stretched by resampling (which lowers the fricative's
    // spectral content and blurs its texture).  Instead the whole sample is
    // played and its steady-state middle half is **looped** to sustain a longer
    // fricative, then a tail from the end closes it; a shorter target simply
    // truncates the sample.  This preserves the natural noise spectrum.
    let mut seq: Vec<i32> = Vec::with_capacity(target_len);
    if len4 == 0 || target_len <= wav_length {
        // Short target (or a tiny sample): play the first `target_len` samples,
        // repeating cyclically only if the sample is shorter than the target.
        for i in 0..target_len {
            seq.push(raw[i % wav_length]);
        }
    } else {
        // Longer than the sample: first 3/4 (len4*3), then loop the middle half
        // ([len4, 3*len4)) until nearly full, then the final tail from the end.
        let mut remaining = target_len;
        let c1 = len4 * 3;
        seq.extend_from_slice(&raw[..c1]);
        remaining -= c1;
        while remaining > len4 * 3 {
            seq.extend_from_slice(&raw[len4..len4 * 3]);
            remaining -= len4 * 2;
        }
        if remaining > 0 {
            seq.extend_from_slice(&raw[wav_length - remaining..]);
        }
    }

    // Apply amplitude (mirrors PlayWave; echo feedback is a separate post-fx).
    let out: Vec<i16> = seq
        .iter()
        .map(|&s| {
            let value = (s * CONSONANT_AMP * GENERAL_AMPLITUDE) >> 10;
            (value * amp2 / 32).clamp(-32768, 32767) as i16
        })
        .collect();

    Some(out)
}

/// Compile 16-bit PCM `samples` into the `phondata` WAV-sample format read by
/// [`parse_wav_sample`] — the WAV-data half of the phoneme compiler (§1.6).
///
/// Header: `wav_length` (u16 LE = byte count) + `wav_scale` (`0` → 16-bit LE
/// data) + a padding byte, followed by the samples as little-endian `i16`.
pub fn compile_wav_sample(samples: &[i16]) -> Vec<u8> {
    let wav_length = samples.len() * 2;
    let mut out = Vec::with_capacity(4 + wav_length);
    out.extend_from_slice(&(wav_length as u16).to_le_bytes());
    out.push(0); // wav_scale = 0 → 16-bit samples
    out.push(0); // reserved / padding
    for &s in samples {
        out.extend_from_slice(&s.to_le_bytes());
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
    fn wav_sample_compile_round_trip() {
        let samples = vec![1000i16, -2000, 500, 12000, -8000];
        let bytes = compile_wav_sample(&samples);

        // Structural: header + raw i16 LE data (exact, independent of the
        // reader's amplitude scaling).
        assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]) as usize, samples.len() * 2);
        assert_eq!(bytes[2], 0, "16-bit → wav_scale 0");
        for (i, &s) in samples.iter().enumerate() {
            assert_eq!(i16::from_le_bytes([bytes[4 + i * 2], bytes[4 + i * 2 + 1]]), s);
        }

        // The reader decodes the right number of samples from the compiled data.
        let decoded = parse_wav_sample(0, &bytes, 1.0, 100).expect("decode compiled wav");
        assert_eq!(decoded.len(), samples.len());
    }

    #[test]
    fn empty_wav_data_returns_none() {
        let data = vec![0u8, 0u8, 0u8, 0u8]; // wav_length = 0
        assert!(parse_wav_sample(0, &data, 1.0, 0).is_none());
    }

    #[test]
    fn parse_8bit_sample() {
        // Header: length=3, scale=1 (8-bit), padding
        let mut data = vec![3u8, 0, 1, 0]; // length=3, scale=1
        data.extend_from_slice(&[100u8, 200u8, 50u8]); // 3 bytes of 8-bit data
        let result = parse_wav_sample(0, &data, 1.0, 0);
        assert!(result.is_some());
        let pcm = result.unwrap();
        assert!(!pcm.is_empty());
    }

    #[test]
    fn parse_16bit_sample() {
        // Header: length=4 (2 samples), scale=0 (16-bit), padding
        let mut data = vec![4u8, 0, 0, 0]; // length=4, scale=0 (16-bit)
        // Two 16-bit samples: 1000 and -1000
        data.extend_from_slice(&(1000i16).to_le_bytes());
        data.extend_from_slice(&(-1000i16).to_le_bytes());
        let result = parse_wav_sample(0, &data, 1.0, 0);
        assert!(result.is_some());
        let pcm = result.unwrap();
        assert_eq!(pcm.len(), 2);
    }

    #[test]
    fn long_target_loops_the_middle_not_resamples() {
        // 16 distinct increasing 16-bit samples (a ramp).
        let n = 16usize;
        let mut data = vec![((n * 2) & 0xff) as u8, ((n * 2) >> 8) as u8, 0, 0]; // len bytes, scale 0
        for i in 0..n {
            data.extend_from_slice(&((i as i16) * 1000).to_le_bytes());
        }
        // Stretch past the sample length → the looping path (not resampling).
        let out = parse_wav_sample(0, &data, 2.5, 100).unwrap();
        assert!(out.len() > n, "should be stretched: {}", out.len());
        // Looping the steady-state middle resets the ramp repeatedly (a sawtooth);
        // a linear resample of a ramp would be (weakly) monotonic non-decreasing.
        let resets = out.windows(2).filter(|w| w[1] < w[0]).count();
        assert!(resets >= 2, "middle must loop (sawtooth), not resample: resets={resets}");
    }

    #[test]
    fn short_target_truncates() {
        // 16 samples, target shorter than the sample → truncation (first N).
        let n = 16usize;
        let mut data = vec![((n * 2) & 0xff) as u8, ((n * 2) >> 8) as u8, 0, 0];
        for i in 0..n {
            data.extend_from_slice(&((i as i16) * 1000).to_le_bytes());
        }
        let out = parse_wav_sample(0, &data, 0.5, 100).unwrap();
        assert_eq!(out.len(), n / 2, "0.5× → half length");
        // Truncation keeps the ramp monotonic (no loop reset).
        assert!(out.windows(2).all(|w| w[1] >= w[0]), "truncated ramp stays monotonic: {out:?}");
    }

    #[test]
    fn speed_factor_changes_length() {
        let mut data = vec![4u8, 0, 0, 0]; // 2 16-bit samples
        data.extend_from_slice(&(1000i16).to_le_bytes());
        data.extend_from_slice(&(-1000i16).to_le_bytes());

        let slow = parse_wav_sample(0, &data, 2.0, 0).unwrap();
        let fast = parse_wav_sample(0, &data, 0.5, 0).unwrap();
        assert!(slow.len() > fast.len(), "slower playback → more samples");
    }
}
