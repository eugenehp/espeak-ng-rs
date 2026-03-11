//! Phoneme table entry types.
//!
//! Rust equivalents of `PHONEME_TAB`, `PHONEME_TAB_LIST`, and
//! `REPLACE_PHONEMES` from `phoneme.h`.

use super::N_PHONEME_TAB_NAME;

// ---------------------------------------------------------------------------
// PhonemeTab — mirrors PHONEME_TAB (16 bytes, little-endian)
// ---------------------------------------------------------------------------

/// One row in a phoneme table.  Mirrors `PHONEME_TAB` from `phoneme.h`.
///
/// Layout matches the C struct exactly (16 bytes, no padding):
/// ```text
/// offset 0  u32  mnemonic   up to 4 ASCII chars, first char in LSB
/// offset 4  u32  phflags    bits 16-19 = place of articulation
/// offset 8  u16  program    byte offset into phondata
/// offset 10 u8   code       phoneme index (0-255)
/// offset 11 u8   type       phVOWEL / phSTOP / phPAUSE etc.
/// offset 12 u8   start_type
/// offset 13 u8   end_type
/// offset 14 u8   std_length for vowels: ms/2; for stress: stress type
/// offset 15 u8   length_mod length_mod group number
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PhonemeTab {
    /// Up to 4 ASCII chars packed little-endian: first char in the LSByte.
    pub mnemonic:   u32,
    /// Flag bits; bits 16-19 encode place of articulation.
    pub phflags:    u32,
    /// Byte offset into `phondata` for this phoneme's synthesis program.
    pub program:    u16,
    /// Index of this phoneme within the active table (0-255).
    pub code:       u8,
    /// Phoneme type: `PH_VOWEL`, `PH_STOP`, `PH_PAUSE`, etc.
    pub typ:        u8,
    /// Start type (consonant cluster context).
    pub start_type: u8,
    /// End type / voicing-switch info.
    pub end_type:   u8,
    /// Standard length in ms/2 (vowels) or stress type (stress phonemes).
    pub std_length: u8,
    /// Length-modifier group number.
    pub length_mod: u8,
}

impl PhonemeTab {
    /// Decode the mnemonic u32 into a display string (up to 4 chars).
    pub fn mnemonic_str(&self) -> String {
        let mut s = String::with_capacity(4);
        for shift in 0..4u32 {
            let c = ((self.mnemonic >> (shift * 8)) & 0xff) as u8;
            if c == 0 { break; }
            s.push(c as char);
        }
        s
    }

    /// Pack up to 4 ASCII bytes into a mnemonic `u32` (first char in LSByte).
    pub fn pack_mnemonic(s: &str) -> u32 {
        let b = s.as_bytes();
        let mut v = 0u32;
        for (i, &byte) in b.iter().take(4).enumerate() {
            v |= (byte as u32) << (i * 8);
        }
        v
    }

    /// Parse a single `PhonemeTab` from 16 raw bytes (little-endian).
    pub fn from_bytes(bytes: &[u8; 16]) -> Self {
        let mnemonic   = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let phflags    = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let program    = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
        let code       = bytes[10];
        let typ        = bytes[11];
        let start_type = bytes[12];
        let end_type   = bytes[13];
        let std_length = bytes[14];
        let length_mod = bytes[15];
        Self { mnemonic, phflags, program, code, typ, start_type, end_type, std_length, length_mod }
    }

    /// Serialize back to 16 bytes (for round-trip testing).
    pub fn to_bytes(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&self.mnemonic.to_le_bytes());
        out[4..8].copy_from_slice(&self.phflags.to_le_bytes());
        out[8..10].copy_from_slice(&self.program.to_le_bytes());
        out[10] = self.code;
        out[11] = self.typ;
        out[12] = self.start_type;
        out[13] = self.end_type;
        out[14] = self.std_length;
        out[15] = self.length_mod;
        out
    }
}

// ---------------------------------------------------------------------------
// PhonemeTabList — mirrors PHONEME_TAB_LIST
// ---------------------------------------------------------------------------

/// One entry in the list of all loaded phoneme tables.
/// Mirrors `PHONEME_TAB_LIST` from `phoneme.h`.
#[derive(Debug, Clone)]
pub struct PhonemeTabList {
    /// Table name (null-padded 32-byte field in the file).
    pub name: String,
    /// All phoneme entries for this table (parsed from the binary).
    pub phonemes: Vec<PhonemeTab>,
    /// Number of phonemes (= `phonemes.len()`).
    pub n_phonemes: usize,
    /// 1-based index of the base table to inherit from, or 0 = none.
    pub includes: u8,
}

impl PhonemeTabList {
    /// Parse the table name from a 32-byte null-padded buffer.
    pub fn parse_name(buf: &[u8; N_PHONEME_TAB_NAME]) -> String {
        let end = buf.iter().position(|&b| b == 0).unwrap_or(N_PHONEME_TAB_NAME);
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }
}

// ---------------------------------------------------------------------------
// ReplacePhoneme — mirrors REPLACE_PHONEMES
// ---------------------------------------------------------------------------

/// A phoneme substitution rule for the current voice.
/// Mirrors `REPLACE_PHONEMES` from `phoneme.h`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReplacePhoneme {
    /// Phoneme code to replace.
    pub old_ph: u8,
    /// Replacement phoneme code.
    pub new_ph: u8,
    /// 0 = always replace; 1 = only at end of word.
    pub kind: i8,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mnemonic_roundtrip_ascii() {
        for name in &["@", "@-", "nas", "vwl!", "_:"] {
            let packed = PhonemeTab::pack_mnemonic(name);
            let mut ph = PhonemeTab::default();
            ph.mnemonic = packed;
            assert_eq!(&ph.mnemonic_str(), name, "roundtrip failed for {:?}", name);
        }
    }

    #[test]
    fn from_bytes_roundtrip() {
        // Known entry from Table 1 of the installed phontab:
        // offset 0x28 in file: 5f 01 00 00  00 00 00 00  00 00  00 00  00 00 00 00
        // BUT the first real phoneme in base1 at offset 0x278 was:
        // _:  code=9  type=0  phflags=0  program=0  std_len=37
        let raw: [u8; 16] = [
            0x5f, 0x3a, 0x00, 0x00, // mnemonic "_:"
            0x00, 0x00, 0x00, 0x00, // phflags
            0x00, 0x00,             // program
            0x09,                   // code = 9
            0x00,                   // type = phPAUSE
            0x00,                   // start_type
            0x00,                   // end_type
            0x25,                   // std_length = 37
            0x00,                   // length_mod
        ];
        let ph = PhonemeTab::from_bytes(&raw);
        assert_eq!(ph.mnemonic_str(), "_:");
        assert_eq!(ph.code, 9);
        assert_eq!(ph.std_length, 37);
        assert_eq!(ph.to_bytes(), raw, "round-trip failed");
    }

    #[test]
    fn parse_name_null_padded() {
        let mut buf = [0u8; 32];
        buf[..4].copy_from_slice(b"base");
        assert_eq!(PhonemeTabList::parse_name(&buf), "base");
    }

    #[test]
    fn parse_name_full() {
        let mut buf = [b'x'; 32];
        assert_eq!(PhonemeTabList::parse_name(&buf).len(), 32);
        buf[31] = 0;
        assert_eq!(PhonemeTabList::parse_name(&buf).len(), 31);
    }
}
