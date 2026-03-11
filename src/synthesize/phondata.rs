//! Parser for spectral frame sequences in the espeak-ng `phondata` binary.

// src/synthesize/phondata.rs
//
// Parse spectral frame sequences from the espeak-ng `phondata` binary file.
//
// The phondata file contains precomputed acoustic data for each phoneme.
// Frame sequences are stored as either:
//   - SPECT_SEQ  (non-Klatt):  4-byte header + N × 44-byte frame_t2 structs
//   - SPECT_SEQK (Klatt):      4-byte header + N × 64-byte frame_t  structs
//
// Which variant is used is determined by `frame[0].frflags & FRFLAG_KLATT`.
//
// References: synthesize.h (frame_t, SPECT_SEQ, SPECT_SEQK)

/// Maximum frames in a spectral sequence (N_SEQ_FRAMES in synthesize.h)
pub const N_SEQ_FRAMES: usize = 25;

/// FRFLAG_KLATT: frame uses extra Klatt parameters (64-byte layout)
pub const FRFLAG_KLATT: u16 = 0x01;
/// FRFLAG_VOWEL_CENTRE: marks the centre point of a vowel
pub const FRFLAG_VOWEL_CENTRE: u16 = 0x02;
/// FRFLAG_BREAK: don't merge with next frame
pub const FRFLAG_BREAK: u16 = 0x10;

/// Size of the non-Klatt frame_t2 struct in C (bytes)
pub const FRAME_T2_SIZE: usize = 44;
/// Size of the full Klatt frame_t struct in C (bytes)
pub const FRAME_T_SIZE: usize = 64;

// ---------------------------------------------------------------------------
// SpectFrame — a single formant synthesis frame
// ---------------------------------------------------------------------------

/// Spectral synthesis frame — mirrors `frame_t` from synthesize.h.
///
/// For non-Klatt sequences only the first set of fields (matching `frame_t2`)
/// are meaningful.
#[derive(Debug, Clone, Default)]
pub struct SpectFrame {
    /// Frame flags (FRFLAG_* bitmask)
    pub frflags: u16,
    /// Formant frequencies F0–F6 in Hz
    pub ffreq: [i16; 7],
    /// Frame duration in STEPSIZE units (64 samples ≈ 2.9 ms at 22050 Hz)
    pub length: u8,
    /// Relative amplitude (root-mean-square)
    pub rms: u8,
    /// Formant heights (amplitude of each formant), F0–F7
    pub fheight: [u8; 8],
    /// Formant widths / 4, F0–F5
    pub fwidth: [u8; 6],
    /// Right-side formant widths / 4, F0–F2
    pub fright: [u8; 3],
    /// Klatt bandwidths / 2: BNZ, F1, F2, F3
    pub bw: [u8; 4],
    /// Klatt parameters: AV, FNZ, Tilt, Aspr, Skew
    pub klattp: [u8; 5],
    /// Extended Klatt parameters: AVp, Fric, FricBP, Turb, (spare)
    pub klattp2: [u8; 5],
    /// Klatt parallel amplitudes F0–F6
    pub klatt_ap: [u8; 7],
    /// Klatt parallel bandwidths / 2, F0–F6
    pub klatt_bp: [u8; 7],
}

impl SpectFrame {
    /// Parse a non-Klatt frame_t2 from 44 bytes.
    pub fn from_bytes_t2(data: &[u8]) -> Option<Self> {
        if data.len() < FRAME_T2_SIZE { return None; }
        let mut f = SpectFrame::default();
        f.frflags = u16::from_le_bytes([data[0], data[1]]);
        for i in 0..7 {
            f.ffreq[i] = i16::from_le_bytes([data[2 + i*2], data[3 + i*2]]);
        }
        // offset 16
        f.length = data[16];
        f.rms    = data[17];
        // offset 18
        f.fheight.copy_from_slice(&data[18..26]);
        // offset 26
        f.fwidth.copy_from_slice(&data[26..32]);
        // offset 32
        f.fright.copy_from_slice(&data[32..35]);
        // offset 35
        f.bw.copy_from_slice(&data[35..39]);
        // offset 39
        f.klattp.copy_from_slice(&data[39..44]);
        Some(f)
    }

    /// Parse a Klatt frame_t from 64 bytes.
    pub fn from_bytes_t(data: &[u8]) -> Option<Self> {
        if data.len() < FRAME_T_SIZE { return None; }
        let mut f = SpectFrame::default();
        f.frflags = u16::from_le_bytes([data[0], data[1]]);
        for i in 0..7 {
            f.ffreq[i] = i16::from_le_bytes([data[2 + i*2], data[3 + i*2]]);
        }
        f.length = data[16];
        f.rms    = data[17];
        f.fheight.copy_from_slice(&data[18..26]);
        f.fwidth.copy_from_slice(&data[26..32]);
        f.fright.copy_from_slice(&data[32..35]);
        f.bw.copy_from_slice(&data[35..39]);
        f.klattp.copy_from_slice(&data[39..44]);
        f.klattp2.copy_from_slice(&data[44..49]);
        f.klatt_ap.copy_from_slice(&data[49..56]);
        f.klatt_bp.copy_from_slice(&data[56..63]);
        // data[63] = spare
        Some(f)
    }

    /// Return the F1 frequency in Hz (for synthesis).
    #[inline]
    pub fn f1_hz(&self) -> f64 { self.ffreq[1] as f64 }

    /// Return the F2 frequency in Hz.
    #[inline]
    pub fn f2_hz(&self) -> f64 { self.ffreq[2] as f64 }

    /// Return the F3 frequency in Hz.
    #[inline]
    pub fn f3_hz(&self) -> f64 { self.ffreq[3] as f64 }

    /// Return duration in samples at 22050 Hz (STEPSIZE = 64 samples).
    #[inline]
    pub fn dur_samples(&self, speed_factor: f64) -> usize {
        let base = (self.length as usize) * 64;
        ((base as f64 * speed_factor) as usize).max(1)
    }
}

// ---------------------------------------------------------------------------
// SpectSeq — sequence of frames for one phoneme
// ---------------------------------------------------------------------------

/// A spectral frame sequence as stored in phondata.
///
/// Mirrors `SPECT_SEQ` / `SPECT_SEQK` from synthesize.h.
#[derive(Debug, Clone)]
pub struct SpectSeq {
    /// The individual frames (in display order).
    pub frames: Vec<SpectFrame>,
    /// True if this is a Klatt sequence (frame_t, 64 bytes each).
    pub is_klatt: bool,
}

impl SpectSeq {
    /// Parse a `SPECT_SEQ` or `SPECT_SEQK` from `phondata` at a given offset.
    ///
    /// Returns `None` if the data is truncated or the frame count is 0.
    pub fn parse(phondata: &[u8], offset: usize) -> Option<Self> {
        // Header: length_total(i16) + n_frames(u8) + sqflags(u8) = 4 bytes
        if offset + 4 > phondata.len() { return None; }
        let data = &phondata[offset..];
        let n_frames = data[2] as usize;
        if n_frames == 0 || n_frames > N_SEQ_FRAMES { return None; }

        // Determine Klatt vs non-Klatt from first frame's frflags
        if data.len() < 4 + 2 { return None; } // need at least frflags of frame 0
        let first_frflags = u16::from_le_bytes([data[4], data[5]]);
        let is_klatt = (first_frflags & FRFLAG_KLATT) != 0;
        let frame_size = if is_klatt { FRAME_T_SIZE } else { FRAME_T2_SIZE };

        if data.len() < 4 + n_frames * frame_size { return None; }

        let mut frames = Vec::with_capacity(n_frames);
        for i in 0..n_frames {
            let base = 4 + i * frame_size;
            let frame_data = &data[base..base + frame_size];
            let frame = if is_klatt {
                SpectFrame::from_bytes_t(frame_data)?
            } else {
                SpectFrame::from_bytes_t2(frame_data)?
            };
            frames.push(frame);
        }

        Some(SpectSeq { frames, is_klatt })
    }

    /// True if the sequence is intended for a voiced phoneme.
    ///
    /// A sequence is considered voiced if any frame has non-zero klattp[0] (AV).
    /// For non-Klatt frames, we check whether F1 is non-zero.
    pub fn is_voiced(&self) -> bool {
        self.frames.iter().any(|f| {
            f.klattp[0] > 0 || f.ffreq[1] > 0
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal valid SPECT_SEQ (non-Klatt, 1 frame):
    // Header: length_total=0 (i16 LE), n_frames=1, sqflags=0
    // Frame (44 bytes): frflags=0, ffreq=[500,1500,2500,...], length=4, rms=100, ...
    fn make_seq_1frame() -> Vec<u8> {
        let mut data = vec![0u8; 4 + FRAME_T2_SIZE];
        data[2] = 1; // n_frames
        // Frame starts at offset 4
        // frflags = 0 (non-Klatt)
        // ffreq[1] = 500 Hz (F1)
        let f1 = 500i16.to_le_bytes();
        let f2 = 1500i16.to_le_bytes();
        data[4+2..4+4].copy_from_slice(&[0,0]); // ffreq[0] = 0
        data[4+4..4+6].copy_from_slice(&f1);    // ffreq[1] = 500
        data[4+6..4+8].copy_from_slice(&f2);    // ffreq[2] = 1500
        data[4+16] = 4;  // length = 4 steps
        data[4+17] = 80; // rms = 80
        data
    }

    #[test]
    fn parse_basic_seq() {
        let raw = make_seq_1frame();
        let seq = SpectSeq::parse(&raw, 0).expect("should parse");
        assert_eq!(seq.frames.len(), 1);
        assert!(!seq.is_klatt);
        assert_eq!(seq.frames[0].f1_hz(), 500.0);
        assert_eq!(seq.frames[0].f2_hz(), 1500.0);
        assert_eq!(seq.frames[0].length, 4);
        assert_eq!(seq.frames[0].rms, 80);
    }

    #[test]
    fn parse_too_short_returns_none() {
        let raw = vec![0u8; 3];
        assert!(SpectSeq::parse(&raw, 0).is_none());
    }

    #[test]
    fn parse_zero_frames_returns_none() {
        let raw = vec![0u8; 8]; // header with n_frames=0
        assert!(SpectSeq::parse(&raw, 0).is_none());
    }

    #[test]
    fn parse_at_offset() {
        let mut raw = vec![0xff_u8; 8]; // garbage prefix
        let seq_data = make_seq_1frame();
        raw.extend_from_slice(&seq_data);
        let seq = SpectSeq::parse(&raw, 8).expect("should parse at offset 8");
        assert_eq!(seq.frames.len(), 1);
        assert_eq!(seq.frames[0].f1_hz(), 500.0);
    }

    #[test]
    fn frame_t2_round_trip() {
        let mut raw = [0u8; FRAME_T2_SIZE];
        // frflags = 2
        raw[0] = 2; raw[1] = 0;
        // ffreq[0]=100, ffreq[1]=500, ffreq[2]=1500
        raw[2] = 100; raw[3] = 0;
        raw[4] = 244; raw[5] = 1;  // 500 = 0x01F4
        raw[6] = 220; raw[7] = 5;  // 1500 = 0x05DC
        raw[16] = 8;  // length
        raw[17] = 60; // rms
        let frame = SpectFrame::from_bytes_t2(&raw).unwrap();
        assert_eq!(frame.frflags, 2);
        assert_eq!(frame.ffreq[1], 500);
        assert_eq!(frame.ffreq[2], 1500);
        assert_eq!(frame.length, 8);
        assert_eq!(frame.rms, 60);
    }

    #[test]
    fn frame_t_round_trip() {
        let mut raw = [0u8; FRAME_T_SIZE];
        raw[0] = 1; // frflags = FRFLAG_KLATT
        let frame = SpectFrame::from_bytes_t(&raw).unwrap();
        assert_eq!(frame.frflags, 1);
    }

    #[test]
    fn klatt_seq_detected() {
        let mut raw = vec![0u8; 4 + FRAME_T_SIZE];
        raw[2] = 1; // n_frames=1
        raw[4] = 1; // first frame frflags bit 0 = FRFLAG_KLATT
        let seq = SpectSeq::parse(&raw, 0).expect("should parse");
        assert!(seq.is_klatt);
        assert_eq!(seq.frames.len(), 1);
    }

    #[test]
    fn voiced_detection() {
        let mut raw = make_seq_1frame();
        // set klattp[0] (AV) = 0 initially → not voiced
        // klattp starts at offset 4+39 = 43
        raw[4+39] = 0;
        let seq_unvoiced = SpectSeq::parse(&raw, 0).unwrap();
        // F1 is 500, so is_voiced via ffreq check
        assert!(seq_unvoiced.is_voiced(), "non-zero F1 → voiced");

        // Now set F1 = 0 and AV = 0 → unvoiced
        raw[4+4] = 0; raw[4+5] = 0; // ffreq[1] = 0
        let seq_v2 = SpectSeq::parse(&raw, 0).unwrap();
        assert!(!seq_v2.is_voiced());
    }

    #[test]
    fn dur_samples_speed_factor() {
        let mut f = SpectFrame::default();
        f.length = 4;
        // 4 * 64 = 256 samples at speed 1.0
        assert_eq!(f.dur_samples(1.0), 256);
        // double speed → 128
        assert_eq!(f.dur_samples(0.5), 128);
    }
}
