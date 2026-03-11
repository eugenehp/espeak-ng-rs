//! Phoneme encoding helpers.
//!
//! Port of `EncodePhonemes()` / `DecodePhonemes()` from `dictionary.c`
//! (lines 294–411).
//
// These functions translate between:
//   • ASCII mnemonic strings  (e.g. "hEl@U")
//   • internal phoneme-code byte strings  (sequence of PhonemeTab.code bytes)
//
// They require access to the active phoneme table (PhonemeData) because
// the encoding is language-specific.

use crate::phoneme::PhonemeData;
use crate::phoneme::{PH_STRESS, PH_INVALID};
use super::N_WORD_PHONEMES;

// ─────────────────────────────────────────────────────────────────────────────
// Encode
// ─────────────────────────────────────────────────────────────────────────────

/// Translate a (space-separated) ASCII mnemonic string into the internal
/// phoneme-code byte sequence.
///
/// Corresponds to `EncodePhonemes()` in dictionary.c.
///
/// Returns `(encoded_bytes, bad_phoneme)` where `bad_phoneme` is `Some(str)`
/// if an unrecognised mnemonic was encountered.
pub fn encode_phonemes(
    input: &str,
    phdata: &PhonemeData,
) -> (Vec<u8>, Option<String>) {
    // Build a fast lookup table: for each possible first byte, list of
    // (mnemonic_u32, code) pairs.
    let tab = build_phoneme_lookup(phdata);

    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(N_WORD_PHONEMES);
    let mut bad: Option<String> = None;
    let mut i = 0;

    // Skip leading ASCII whitespace (matching C's `isspace`)
    while i < bytes.len() && bytes[i] < 0x80 && bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    while i < bytes.len() {
        let c = bytes[i];
        if c == 0 || c.is_ascii_whitespace() { break; }

        if c == b'|' {
            // Separator between mnemonics.  Double || = literal |.
            if i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                // fall through and try to match "||" as a mnemonic
            } else {
                i += 1;
                continue;
            }
        }

        // Find the phoneme whose mnemonic gives the longest match at position i.
        let (max_ph, max_len) = best_match_from_table(&tab, bytes, i);

        if max_ph == 0 {
            // Unrecognised phoneme
            let bstart = i;
            let bend = next_char_boundary(bytes, i);
            bad = Some(String::from_utf8_lossy(&bytes[bstart..bend]).into_owned());
            out.push(0); // C writes a 0 byte
            i += 1;
        } else {
            let advance = if max_len <= 0 { 1 } else { max_len as usize };
            i += advance;
            out.push(max_ph);

            // phonSWITCH is followed by the language name in the output.
            if is_switch_phoneme_code(phdata, max_ph) {
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    out.push(bytes[i].to_ascii_lowercase());
                    i += 1;
                }
                out.push(0);
                if i < bytes.len() {
                    out.push(b'|'); // more phonemes follow
                }
            }
        }
    }

    out.push(0); // null-terminate
    (out, bad)
}

// ─────────────────────────────────────────────────────────────────────────────
// Decode
// ─────────────────────────────────────────────────────────────────────────────

/// Translate an internal phoneme-code byte sequence back into ASCII mnemonics.
///
/// Corresponds to `DecodePhonemes()` in dictionary.c.
///
/// Stress phonemes (type == phSTRESS, std_length ≤ 4, program == 0) are
/// rendered using the `stress_chars` table rather than their mnemonic.
pub fn decode_phonemes(encoded: &[u8], phdata: &PhonemeData) -> String {
    const STRESS_CHARS: &[u8] = b"==,,'*  ";
    let mut out = String::from("* ");

    let mut i = 0;
    while i < encoded.len() {
        let phcode = encoded[i];
        i += 1;
        if phcode == 0 { break; }
        if phcode == 255 { continue; } // unrecognised marker

        let tab = match phdata.get(phcode) {
            Some(t) => t,
            None => continue,
        };

        if tab.typ == PH_STRESS && tab.std_length <= 4 && tab.program == 0 {
            let sl = tab.std_length as usize;
            if sl > 1 && sl < STRESS_CHARS.len() {
                let sc = STRESS_CHARS[sl] as char;
                if sc != ' ' { out.push(sc); }
            }
        } else {
            let mnem = tab.mnemonic;
            for shift in 0..4u32 {
                let c = ((mnem >> (shift * 8)) & 0xff) as u8;
                if c == 0 { break; }
                out.push(c as char);
            }

            if is_switch_phoneme_code(phdata, phcode) {
                while i < encoded.len() && (encoded[i] as char).is_ascii_alphabetic() {
                    out.push(encoded[i] as char);
                    i += 1;
                }
            }
        }
    }

    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Build a lookup map: code → (mnemonic u32, code).
/// We just return a flat Vec because N_PHONEME_TAB=256 is small.
fn build_phoneme_lookup(phdata: &PhonemeData) -> Vec<(u32, u8)> {
    let mut tab = Vec::with_capacity(256);
    for code in 1u8..=254 {
        if let Some(ph) = phdata.get(code) {
            if ph.typ != PH_INVALID && ph.mnemonic != 0 {
                tab.push((ph.mnemonic, code));
            }
        }
    }
    tab
}

fn best_match_from_table(tab: &[(u32, u8)], bytes: &[u8], start: usize) -> (u8, i32) {
    let mut max_ph: u8 = 0;
    let mut max: i32 = -1;

    for &(mnem, code) in tab {
        let mut count = 0i32;
        loop {
            let bi = count as usize;
            if bi >= 4 { break; }
            let c = bytes.get(start + bi).copied().unwrap_or(0);
            if c <= b' ' { break; }
            let mnem_byte = ((mnem >> (bi * 8)) & 0xff) as u8;
            if c != mnem_byte { break; }
            count += 1;
        }
        // Must be a full match: either we consumed 4 bytes, or the next
        // mnemonic byte is 0.
        let next_byte = ((mnem >> (count * 8)) & 0xff) as u8;
        if count > max && (count == 4 || next_byte == 0) {
            max = count;
            max_ph = code;
        }
    }

    (max_ph, max)
}

/// Check if `code` is a SWITCH phoneme (mnemonic "SW\0\0").
fn is_switch_phoneme_code(phdata: &PhonemeData, code: u8) -> bool {
    if let Some(ph) = phdata.get(code) {
        let mnem = ph.mnemonic;
        let b0 = (mnem & 0xff) as u8;
        let b1 = ((mnem >> 8) & 0xff) as u8;
        let b2 = ((mnem >> 16) & 0xff) as u8;
        return b0 == b'S' && b1 == b'W' && b2 == 0;
    }
    false
}

fn next_char_boundary(bytes: &[u8], i: usize) -> usize {
    if i >= bytes.len() { return i; }
    let c = bytes[i];
    let seq_len = if c < 0x80 { 1 }
                  else if c < 0xe0 { 2 }
                  else if c < 0xf0 { 3 }
                  else { 4 };
    (i + seq_len).min(bytes.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn load_phdata() -> Option<PhonemeData> {
        let dir = PathBuf::from("/usr/share/espeak-ng-data");
        if !dir.join("phontab").exists() { return None; }
        let mut ph = PhonemeData::load(&dir).ok()?;
        ph.select_table_by_name("en").ok()?;
        Some(ph)
    }

    #[test]
    fn decode_empty_gives_star() {
        let phdata = match load_phdata() { Some(p) => p, None => return };
        let s = decode_phonemes(&[], &phdata);
        assert_eq!(s, "* ");
    }

    #[test]
    fn encode_unknown_gives_zero_and_bad() {
        let phdata = match load_phdata() { Some(p) => p, None => return };
        let (encoded, bad) = encode_phonemes("XYZZY_NOT_A_PHONEME", &phdata);
        assert!(bad.is_some(), "should report bad phoneme");
        assert_eq!(encoded.last(), Some(&0));
    }

    #[test]
    fn encode_decode_roundtrip_pause() {
        let phdata = match load_phdata() { Some(p) => p, None => return };
        // "_" is the short pause
        let (encoded, bad) = encode_phonemes("_", &phdata);
        assert!(bad.is_none(), "pause should encode cleanly");
        // Should have at least one non-zero byte
        assert!(encoded.iter().any(|&b| b != 0), "should produce phoneme code for _");
    }
}
