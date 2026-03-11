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
/// `amp_override`— if > 0, use this amplitude instead of the default.
///                 Mirrors `DoSample3(&phdata, 0, amp)` where amp=0 → default.
pub fn parse_wav_sample(
    addr: u32,
    phondata: &[u8],
    speed_factor: f64,
    amp_override: i32,
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

    // Amplitude: default = consonant_amp * general_amplitude / 16 = 89
    let amp = if amp_override > 0 {
        amp_override
    } else {
        CONSONANT_AMP * GENERAL_AMPLITUDE / 16
    };

    // Apply speed factor: repeat/drop samples to adjust length
    let target_len = ((raw.len() as f64) * speed_factor) as usize;
    let target_len = target_len.max(1);

    // Resample linearly to target_len
    let resampled: Vec<i16> = (0..target_len)
        .map(|i| {
            let src_idx = i * raw.len() / target_len;
            let s = raw[src_idx.min(raw.len() - 1)];
            // Scale: output = (sample * amp) >> 8
            let out = (s * amp) >> 8;
            out.clamp(-32767, 32767) as i16
        })
        .collect();

    Some(resampled)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
    fn speed_factor_changes_length() {
        let mut data = vec![4u8, 0, 0, 0]; // 2 16-bit samples
        data.extend_from_slice(&(1000i16).to_le_bytes());
        data.extend_from_slice(&(-1000i16).to_le_bytes());

        let slow = parse_wav_sample(0, &data, 2.0, 0).unwrap();
        let fast = parse_wav_sample(0, &data, 0.5, 0).unwrap();
        assert!(slow.len() > fast.len(), "slower playback → more samples");
    }
}
