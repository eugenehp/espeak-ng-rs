//! Text encoding detection and decoding.
//!
//! Rust port of `encoding.c` and `encoding.h`.
//!
//! The C original uses a vtable of function pointers (`espeak_ng_TEXT_DECODER`).
//! Here we use an enum + method dispatch instead, which is idiomatic Rust and
//! avoids `unsafe`.
//!
//! # Supported encodings
//! UTF-8, US-ASCII, ISO-8859-1 through -16, KOI8-R, ISCII, UCS-2.
//!
//! # Example
//! ```rust
//! use espeak_ng::encoding::{Encoding, TextDecoder, DecodeMode};
//!
//! let enc = Encoding::from_name("UTF-8");
//! assert_eq!(enc, Encoding::Utf8);
//!
//! let mut dec = TextDecoder::utf8("héllo".as_bytes());
//! let codepoints = dec.collect_codepoints();
//! assert_eq!(codepoints[0], 'h' as u32);
//! ```

pub mod codepages;

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Encoding enum
// ---------------------------------------------------------------------------

/// Text encoding, mirroring `espeak_ng_ENCODING` from `encoding.h`.
///
/// Variants are in the same order as the C enum so that casting a raw integer
/// (e.g. from a binary data file) to `Encoding` works correctly.
///
/// Use [`Encoding::from_name`] to resolve an IANA/MIME name, and
/// [`TextDecoder`] to decode byte streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
#[allow(missing_docs)] // variant names match standard ISO/IANA charset names
pub enum Encoding {
    /// Encoding not recognised.
    Unknown       = 0,
    /// 7-bit US-ASCII.
    UsAscii       = 1,
    /// ISO-8859-1 (Latin-1 — Western European).
    Iso8859_1     = 2,
    /// ISO-8859-2 (Latin-2 — Central European).
    Iso8859_2     = 3,
    /// ISO-8859-3 (Latin-3 — South European).
    Iso8859_3     = 4,
    /// ISO-8859-4 (Latin-4 — North European).
    Iso8859_4     = 5,
    /// ISO-8859-5 (Cyrillic).
    Iso8859_5     = 6,
    /// ISO-8859-6 (Arabic).
    Iso8859_6     = 7,
    /// ISO-8859-7 (Greek).
    Iso8859_7     = 8,
    /// ISO-8859-8 (Hebrew).
    Iso8859_8     = 9,
    /// ISO-8859-9 (Latin-5 — Turkish).
    Iso8859_9     = 10,
    /// ISO-8859-10 (Latin-6 — Nordic).
    Iso8859_10    = 11,
    /// ISO-8859-11 (Thai).
    Iso8859_11    = 12,
    /// ISO-8859-13 (Latin-7 — Baltic Rim).  Note: 12 is not a valid ISO-8859 number.
    Iso8859_13    = 13,
    /// ISO-8859-14 (Latin-8 — Celtic).
    Iso8859_14    = 14,
    /// ISO-8859-15 (Latin-9 — Western European with €).
    Iso8859_15    = 15,
    /// ISO-8859-16 (Latin-10 — South-Eastern European).
    Iso8859_16    = 16,
    /// KOI8-R (Russian Cyrillic).
    Koi8R         = 17,
    /// ISCII (Indian scripts).
    Iscii         = 18,
    /// UTF-8 Unicode.
    Utf8          = 19,
    /// UCS-2 little-endian (16-bit Unicode, no surrogates).
    Ucs2          = 20,
}

impl Encoding {
    /// Resolve an encoding from its IANA/MIME name.
    ///
    /// Mirrors `espeak_ng_EncodingFromName()` in encoding.c, which uses the
    /// `mnem_encoding[]` table.  Comparison is case-insensitive.
    pub fn from_name(name: &str) -> Self {
        // The C table is huge; we implement the same look-up as a match on
        // normalised names.  The canonical names come from:
        //   http://www.iana.org/assignments/character-sets/character-sets.xhtml
        match name.to_ascii_uppercase().as_str() {
            // US-ASCII aliases
            "ANSI_X3.4-1968" | "ANSI_X3.4-1986" | "ASCII" | "US-ASCII"
            | "ISO646-US" | "IBM367" | "US" | "ISO_646.IRV:1991"
            | "ISO-IR-6" | "CP367" | "CSASCII" => Encoding::UsAscii,

            // ISO-8859-1
            "ISO_8859-1" | "ISO_8859-1:1987" | "ISO-8859-1" | "ISO-IR-100"
            | "LATIN1" | "L1" | "IBM819" | "CSISOLATIN1" => Encoding::Iso8859_1,

            // ISO-8859-2
            "ISO_8859-2" | "ISO_8859-2:1987" | "ISO-8859-2" | "ISO-IR-101"
            | "LATIN2" | "L2" | "CSISOLATIN2" => Encoding::Iso8859_2,

            // ISO-8859-3
            "ISO_8859-3" | "ISO_8859-3:1988" | "ISO-8859-3" | "ISO-IR-109"
            | "LATIN3" | "L3" | "CSISOLATIN3" => Encoding::Iso8859_3,

            // ISO-8859-4
            "ISO_8859-4" | "ISO_8859-4:1988" | "ISO-8859-4" | "ISO-IR-110"
            | "LATIN4" | "L4" | "CSISOLATIN4" => Encoding::Iso8859_4,

            // ISO-8859-5
            "ISO_8859-5" | "ISO_8859-5:1988" | "ISO-8859-5" | "ISO-IR-144"
            | "CYRILLIC" | "CSISOLATINCYRILLIC" => Encoding::Iso8859_5,

            // ISO-8859-6
            "ISO_8859-6" | "ISO_8859-6:1987" | "ISO-8859-6" | "ISO-IR-127"
            | "ECMA-114" | "ASMO-708" | "ARABIC" | "CSISOLATINARABIC"
            => Encoding::Iso8859_6,

            // ISO-8859-7
            "ISO_8859-7" | "ISO_8859-7:1987" | "ISO-8859-7" | "ISO-IR-126"
            | "ECMA-118" | "ELOT_928" | "GREEK" | "GREEK8"
            | "CSISOLATINGREEK" => Encoding::Iso8859_7,

            // ISO-8859-8
            "ISO_8859-8" | "ISO_8859-8:1988" | "ISO-8859-8" | "ISO-IR-138"
            | "HEBREW" | "CSISOLATINHEBREW" => Encoding::Iso8859_8,

            // ISO-8859-9
            "ISO_8859-9" | "ISO_8859-9:1989" | "ISO-8859-9" | "ISO-IR-148"
            | "LATIN5" | "L5" | "CSISOLATIN5" => Encoding::Iso8859_9,

            // ISO-8859-10
            "ISO_8859-10" | "ISO-8859-10" | "ISO-IR-157" | "LATIN6" | "L6"
            | "CSISOLATIN6" => Encoding::Iso8859_10,

            // ISO-8859-11 / TIS-620
            "ISO_8859-11" | "ISO-8859-11" | "TIS-620" => Encoding::Iso8859_11,

            // ISO-8859-13
            "ISO_8859-13" | "ISO-8859-13" | "LATIN7" | "L7" => Encoding::Iso8859_13,

            // ISO-8859-14
            "ISO_8859-14" | "ISO-8859-14" | "ISO-IR-199" | "LATIN8" | "L8"
            | "ISO-CELTIC" => Encoding::Iso8859_14,

            // ISO-8859-15
            "ISO_8859-15" | "ISO-8859-15" | "LATIN9" | "LATIN-9" | "LATIN0"
            => Encoding::Iso8859_15,

            // ISO-8859-16
            "ISO_8859-16" | "ISO-8859-16" | "ISO-IR-226" | "LATIN10" | "L10"
            => Encoding::Iso8859_16,

            // KOI8-R
            "KOI8-R" | "CSKOI8R" => Encoding::Koi8R,

            // ISCII
            "ISCII" => Encoding::Iscii,

            // UTF-8
            "UTF-8" | "UTF8" => Encoding::Utf8,

            // UCS-2 / ISO 10646
            "ISO-10646-UCS-2" | "UCS-2" | "CSUNICODE" => Encoding::Ucs2,

            _ => Encoding::Unknown,
        }
    }

    /// Returns `true` if this is a single-byte encoding (includes ASCII).
    pub fn is_single_byte(self) -> bool {
        matches!(
            self,
            Encoding::UsAscii
                | Encoding::Iso8859_1
                | Encoding::Iso8859_2
                | Encoding::Iso8859_3
                | Encoding::Iso8859_4
                | Encoding::Iso8859_5
                | Encoding::Iso8859_6
                | Encoding::Iso8859_7
                | Encoding::Iso8859_8
                | Encoding::Iso8859_9
                | Encoding::Iso8859_10
                | Encoding::Iso8859_11
                | Encoding::Iso8859_13
                | Encoding::Iso8859_14
                | Encoding::Iso8859_15
                | Encoding::Iso8859_16
                | Encoding::Koi8R
                | Encoding::Iscii
        )
    }

    /// Return the codepage table for single-byte encodings, if one exists.
    /// The table maps bytes 0x80–0xFF (index = byte - 0x80) to Unicode codepoints.
    pub fn codepage(self) -> Option<&'static [u16; 128]> {
        use codepages::*;
        match self {
            Encoding::Iso8859_1  => Some(&ISO_8859_1),
            Encoding::Iso8859_2  => Some(&ISO_8859_2),
            Encoding::Iso8859_3  => Some(&ISO_8859_3),
            Encoding::Iso8859_4  => Some(&ISO_8859_4),
            Encoding::Iso8859_5  => Some(&ISO_8859_5),
            Encoding::Iso8859_6  => Some(&ISO_8859_6),
            Encoding::Iso8859_7  => Some(&ISO_8859_7),
            Encoding::Iso8859_8  => Some(&ISO_8859_8),
            Encoding::Iso8859_9  => Some(&ISO_8859_9),
            Encoding::Iso8859_10 => Some(&ISO_8859_10),
            Encoding::Iso8859_11 => Some(&ISO_8859_11),
            Encoding::Iso8859_13 => Some(&ISO_8859_13),
            Encoding::Iso8859_14 => Some(&ISO_8859_14),
            Encoding::Iso8859_15 => Some(&ISO_8859_15),
            Encoding::Iso8859_16 => Some(&ISO_8859_16),
            Encoding::Koi8R      => Some(&KOI8_R),
            Encoding::Iscii      => Some(&ISCII),
            _                    => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Replacement character
// ---------------------------------------------------------------------------

/// Unicode Replacement Character U+FFFD, used for invalid sequences.
pub const REPLACEMENT_CHAR: u32 = 0xFFFD;

// ---------------------------------------------------------------------------
// UTF-8 decoding (standalone, stateless)
// ---------------------------------------------------------------------------

/// Decode one UTF-8 codepoint from `buf`.
///
/// Returns `(codepoint, bytes_consumed)`.  On error the replacement character
/// U+FFFD is returned and `bytes_consumed` is 1 (the invalid lead byte is
/// skipped so the caller can continue).
///
/// Mirrors `string_decoder_getc_utf_8()` in encoding.c, including the
/// "I umlaut a half" (U+FFFD → U+001A) workaround for 3-byte sequences.
///
/// # Panics
/// Panics in debug builds if `buf` is empty.
pub fn utf8_decode_one(buf: &[u8]) -> (u32, usize) {
    debug_assert!(!buf.is_empty(), "utf8_decode_one called on empty buffer");

    let c0 = buf[0];
    match c0 >> 4 {
        // 0xxxxxxx — 1-byte ASCII
        0x0..=0x7 => (c0 as u32, 1),

        // 10xxxxxx — unexpected continuation byte
        0x8..=0xB => (REPLACEMENT_CHAR, 1),

        // 110xxxxx — 2-byte sequence
        0xC | 0xD => {
            if buf.len() < 2 {
                return (REPLACEMENT_CHAR, buf.len());
            }
            let c1 = buf[1];
            if c1 & 0xC0 != 0x80 {
                return (REPLACEMENT_CHAR, 1);
            }
            let cp = ((c0 as u32 & 0x1F) << 6) | (c1 as u32 & 0x3F);
            (cp, 2)
        }

        // 1110xxxx — 3-byte sequence
        0xE => {
            if buf.len() < 3 {
                return (REPLACEMENT_CHAR, buf.len().min(1));
            }
            let c1 = buf[1];
            let c2 = buf[2];
            if c1 & 0xC0 != 0x80 {
                return (REPLACEMENT_CHAR, 1);
            }
            if c2 & 0xC0 != 0x80 {
                return (REPLACEMENT_CHAR, 1);
            }
            let cp = ((c0 as u32 & 0x0F) << 12)
                | ((c1 as u32 & 0x3F) << 6)
                | (c2 as u32 & 0x3F);
            // Mirror C code: "fix the I umlaut a half bug"
            let cp = if cp == 0xFFFD { 0x001A } else { cp };
            (cp, 3)
        }

        // 11110xxx — 4-byte sequence
        _ /* 0xF */ => {
            if buf.len() < 4 {
                return (REPLACEMENT_CHAR, buf.len().min(1));
            }
            let c1 = buf[1];
            let c2 = buf[2];
            let c3 = buf[3];
            if c1 & 0xC0 != 0x80 || c2 & 0xC0 != 0x80 || c3 & 0xC0 != 0x80 {
                return (REPLACEMENT_CHAR, 1);
            }
            let cp = ((c0 as u32 & 0x07) << 18)
                | ((c1 as u32 & 0x3F) << 12)
                | ((c2 as u32 & 0x3F) << 6)
                | (c3 as u32 & 0x3F);
            let cp = if cp <= 0x10_FFFF { cp } else { REPLACEMENT_CHAR };
            (cp, 4)
        }
    }
}

/// Encode a Unicode codepoint as UTF-8.
///
/// Mirrors `utf8_out()` / `out_ptr` logic from various files.
/// Returns the number of bytes written into `buf` (1–4).
///
/// # Panics
/// Panics in debug builds if `cp` is not a valid Unicode scalar value or if
/// `buf` is too small.
pub fn utf8_encode_one(cp: u32, buf: &mut [u8]) -> usize {
    if cp < 0x80 {
        debug_assert!(buf.len() >= 1);
        buf[0] = cp as u8;
        1
    } else if cp < 0x800 {
        debug_assert!(buf.len() >= 2);
        buf[0] = 0xC0 | (cp >> 6) as u8;
        buf[1] = 0x80 | (cp & 0x3F) as u8;
        2
    } else if cp < 0x10000 {
        debug_assert!(buf.len() >= 3);
        buf[0] = 0xE0 | (cp >> 12) as u8;
        buf[1] = 0x80 | ((cp >> 6) & 0x3F) as u8;
        buf[2] = 0x80 | (cp & 0x3F) as u8;
        3
    } else {
        debug_assert!(buf.len() >= 4);
        buf[0] = 0xF0 | (cp >> 18) as u8;
        buf[1] = 0x80 | ((cp >> 12) & 0x3F) as u8;
        buf[2] = 0x80 | ((cp >> 6) & 0x3F) as u8;
        buf[3] = 0x80 | (cp & 0x3F) as u8;
        4
    }
}

// ---------------------------------------------------------------------------
// TextDecoder – the main streaming decoder
// ---------------------------------------------------------------------------

/// Decoding mode, controlling how the auto-detection heuristic works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeMode {
    /// Strict: use exactly the encoding provided, no auto-detection.
    Strict,
    /// Auto: try UTF-8 first; fall back to the provided single-byte encoding
    /// on the first invalid byte (mirrors `espeakCHARS_AUTO`).
    Auto,
}

/// A streaming text decoder that produces Unicode codepoints from an in-memory
/// byte slice.
///
/// Mirrors the `espeak_ng_TEXT_DECODER` C struct + its associated functions.
///
/// The decoder borrows the input slice for its lifetime so no allocation is
/// needed.
pub struct TextDecoder<'a> {
    buf:      &'a [u8],
    pos:      usize,
    encoding: Encoding,
    mode:     DecodeMode,
    /// When `mode == Auto` and we have fallen back to the codepage, this flag
    /// records that the switch has happened so we stop trying UTF-8.
    fell_back: bool,
}

impl<'a> TextDecoder<'a> {
    /// Create a new decoder for `buf` with the given `encoding` and `mode`.
    pub fn new(buf: &'a [u8], encoding: Encoding, mode: DecodeMode) -> Result<Self> {
        if encoding == Encoding::Unknown {
            return Err(Error::UnknownTextEncoding(
                "cannot decode with Encoding::Unknown".to_string(),
            ));
        }
        Ok(TextDecoder {
            buf,
            pos: 0,
            encoding,
            mode,
            fell_back: false,
        })
    }

    /// Convenience: UTF-8 strict decoder (the most common case).
    pub fn utf8(buf: &'a [u8]) -> Self {
        TextDecoder {
            buf,
            pos: 0,
            encoding: Encoding::Utf8,
            mode: DecodeMode::Strict,
            fell_back: false,
        }
    }

    /// Returns `true` when all bytes have been consumed.
    pub fn is_eof(&self) -> bool {
        self.pos >= self.buf.len()
    }

    /// Remaining bytes (useful for slicing the original buffer).
    pub fn remaining(&self) -> &[u8] {
        &self.buf[self.pos..]
    }

    /// Peek at the next codepoint without advancing the position.
    pub fn peek(&self) -> Option<u32> {
        if self.is_eof() {
            return None;
        }
        // We clone just the position to avoid borrowing issues.
        let mut clone = TextDecoder {
            buf:       self.buf,
            pos:       self.pos,
            encoding:  self.encoding,
            mode:      self.mode,
            fell_back: self.fell_back,
        };
        clone.next_codepoint()
    }

    /// Consume and return the next codepoint, or `None` at EOF.
    pub fn next_codepoint(&mut self) -> Option<u32> {
        if self.is_eof() {
            return None;
        }
        let cp = self.decode_one();
        Some(cp)
    }

    /// Collect all remaining codepoints into a `Vec`.
    pub fn collect_codepoints(&mut self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.buf.len() - self.pos);
        while let Some(cp) = self.next_codepoint() {
            if cp == 0 {
                break; // null-terminated, as the C code does
            }
            out.push(cp);
        }
        out
    }

    /// Collect and decode to a Rust `String` (replacing invalid codepoints
    /// with U+FFFD, mirroring C behaviour).
    pub fn decode_to_string(&mut self) -> String {
        let codepoints = self.collect_codepoints();
        codepoints
            .into_iter()
            .map(|cp| char::from_u32(cp).unwrap_or('\u{FFFD}'))
            .collect()
    }

    // ----- private ----------------------------------------------------------

    fn decode_one(&mut self) -> u32 {
        match self.encoding {
            Encoding::UsAscii => self.decode_ascii(),
            Encoding::Utf8    => self.decode_utf8(),
            Encoding::Ucs2    => self.decode_ucs2(),
            enc if enc.is_single_byte() => {
                if self.mode == DecodeMode::Auto && !self.fell_back {
                    self.decode_auto()
                } else {
                    self.decode_codepage()
                }
            }
            _ => {
                self.pos += 1;
                REPLACEMENT_CHAR
            }
        }
    }

    fn decode_ascii(&mut self) -> u32 {
        let b = self.buf[self.pos];
        self.pos += 1;
        if b < 0x80 { b as u32 } else { REPLACEMENT_CHAR }
    }

    fn decode_utf8(&mut self) -> u32 {
        let (cp, consumed) = utf8_decode_one(&self.buf[self.pos..]);
        self.pos += consumed;
        cp
    }

    fn decode_codepage(&mut self) -> u32 {
        let b = self.buf[self.pos];
        self.pos += 1;
        if b < 0x80 {
            b as u32
        } else if let Some(table) = self.encoding.codepage() {
            table[(b - 0x80) as usize] as u32
        } else {
            REPLACEMENT_CHAR
        }
    }

    /// Auto mode: try UTF-8; on first failure, switch to the codepage.
    /// Mirrors `string_decoder_getc_auto()` in encoding.c.
    fn decode_auto(&mut self) -> u32 {
        let saved_pos = self.pos;
        let (cp, consumed) = utf8_decode_one(&self.buf[self.pos..]);
        if cp == REPLACEMENT_CHAR {
            // UTF-8 failed; fall back permanently to the codepage
            self.fell_back = true;
            self.pos = saved_pos;
            self.decode_codepage()
        } else {
            self.pos += consumed;
            cp
        }
    }

    fn decode_ucs2(&mut self) -> u32 {
        if self.pos + 1 >= self.buf.len() {
            self.pos = self.buf.len();
            return REPLACEMENT_CHAR;
        }
        let lo = self.buf[self.pos] as u32;
        let hi = self.buf[self.pos + 1] as u32;
        self.pos += 2;
        lo | (hi << 8)
    }
}

// ---------------------------------------------------------------------------
// Iterator impl
// ---------------------------------------------------------------------------

impl<'a> Iterator for TextDecoder<'a> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if self.is_eof() {
            return None;
        }
        let cp = self.decode_one();
        if cp == 0 {
            // null terminator – signal end like the C code does
            self.pos = self.buf.len();
            return None;
        }
        Some(cp)
    }
}

// ---------------------------------------------------------------------------
// Convenience free functions (mirrors the C public API)
// ---------------------------------------------------------------------------

/// Decode a UTF-8 byte slice to a `String`.
///
/// Invalid sequences are replaced with U+FFFD, matching C behaviour.
pub fn decode_utf8_to_string(bytes: &[u8]) -> String {
    let mut dec = TextDecoder::utf8(bytes);
    dec.decode_to_string()
}

/// Decode a byte slice with the given encoding to a `String`.
pub fn decode_to_string(bytes: &[u8], encoding: Encoding) -> Result<String> {
    let mut dec = TextDecoder::new(bytes, encoding, DecodeMode::Strict)?;
    Ok(dec.decode_to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- utf8_decode_one ---------------------------------------------------

    #[test]
    fn utf8_decode_ascii_range() {
        for b in 0u8..0x80 {
            let (cp, n) = utf8_decode_one(&[b]);
            assert_eq!(cp, b as u32, "ascii byte 0x{b:02x}");
            assert_eq!(n, 1);
        }
    }

    #[test]
    fn utf8_decode_two_byte() {
        // U+00E9 LATIN SMALL LETTER E WITH ACUTE  →  0xC3 0xA9
        let (cp, n) = utf8_decode_one(&[0xC3, 0xA9]);
        assert_eq!(cp, 0x00E9);
        assert_eq!(n, 2);
    }

    #[test]
    fn utf8_decode_three_byte() {
        // U+20AC EURO SIGN  →  0xE2 0x82 0xAC
        let (cp, n) = utf8_decode_one(&[0xE2, 0x82, 0xAC]);
        assert_eq!(cp, 0x20AC);
        assert_eq!(n, 3);
    }

    #[test]
    fn utf8_decode_four_byte() {
        // U+1F600 GRINNING FACE  →  0xF0 0x9F 0x98 0x80
        let (cp, n) = utf8_decode_one(&[0xF0, 0x9F, 0x98, 0x80]);
        assert_eq!(cp, 0x1F600);
        assert_eq!(n, 4);
    }

    #[test]
    fn utf8_decode_overlong_replacement() {
        // Continuation byte in lead position → replacement char, skip 1
        let (cp, n) = utf8_decode_one(&[0x80]);
        assert_eq!(cp, REPLACEMENT_CHAR);
        assert_eq!(n, 1);
    }

    #[test]
    fn utf8_decode_bad_continuation() {
        // Lead byte says 2-byte, but second byte is ASCII (not 10xxxxxx)
        let (cp, n) = utf8_decode_one(&[0xC3, 0x20]);
        assert_eq!(cp, REPLACEMENT_CHAR);
        assert_eq!(n, 1);
    }

    #[test]
    fn utf8_decode_codepoint_max() {
        // U+10FFFF — maximum valid codepoint (4 bytes)
        let (cp, n) = utf8_decode_one(&[0xF4, 0x8F, 0xBF, 0xBF]);
        assert_eq!(cp, 0x10FFFF);
        assert_eq!(n, 4);
    }

    #[test]
    fn utf8_decode_above_max_is_replacement() {
        // Byte pattern that would give cp > 0x10FFFF  (0xF4 0x90 0x80 0x80 → U+110000)
        let (cp, _) = utf8_decode_one(&[0xF4, 0x90, 0x80, 0x80]);
        assert_eq!(cp, REPLACEMENT_CHAR);
    }

    #[test]
    fn utf8_decode_iumlaut_half_bug_workaround() {
        // The C code maps U+FFFD from a 3-byte sequence to U+001A.
        // The 3-byte encoding of U+FFFD is EF BF BD.
        let (cp, n) = utf8_decode_one(&[0xEF, 0xBF, 0xBD]);
        // Normal: should be 0xFFFD but the C workaround makes it 0x001A
        assert_eq!(cp, 0x001A, "expected the C workaround U+001A, got 0x{cp:04x}");
        assert_eq!(n, 3);
    }

    // ---- round-trip --------------------------------------------------------

    #[test]
    fn utf8_roundtrip_bmp() {
        let mut buf = [0u8; 4];
        // A selection of BMP codepoints including surrogates (which are invalid
        // in UTF-8 but let's not crash on them).
        for cp in [0u32, 0x41, 0xFF, 0x100, 0x7FF, 0x800, 0xFFFE, 0xFFFF] {
            if let Some(ch) = char::from_u32(cp) {
                let s = ch.encode_utf8(&mut buf);
                let (decoded, _) = utf8_decode_one(s.as_bytes());
                // Account for the C workaround: U+FFFD maps to U+001A in 3-byte
                let expected = if cp == 0xFFFD { 0x001A } else { cp };
                assert_eq!(decoded, expected, "cp=U+{cp:04X}");
            }
        }
    }

    // ---- utf8_encode_one ---------------------------------------------------

    #[test]
    fn utf8_encode_ascii() {
        let mut buf = [0u8; 4];
        assert_eq!(utf8_encode_one(b'A' as u32, &mut buf), 1);
        assert_eq!(buf[0], b'A');
    }

    #[test]
    fn utf8_encode_two_byte() {
        let mut buf = [0u8; 4];
        let n = utf8_encode_one(0x00E9, &mut buf); // é
        assert_eq!(n, 2);
        assert_eq!(&buf[..2], &[0xC3, 0xA9]);
    }

    #[test]
    fn utf8_encode_three_byte() {
        let mut buf = [0u8; 4];
        let n = utf8_encode_one(0x20AC, &mut buf); // €
        assert_eq!(n, 3);
        assert_eq!(&buf[..3], &[0xE2, 0x82, 0xAC]);
    }

    #[test]
    fn utf8_encode_four_byte() {
        let mut buf = [0u8; 4];
        let n = utf8_encode_one(0x1F600, &mut buf); // 😀
        assert_eq!(n, 4);
        assert_eq!(&buf[..4], &[0xF0, 0x9F, 0x98, 0x80]);
    }

    // ---- Encoding::from_name -----------------------------------------------

    #[test]
    fn encoding_from_name_utf8() {
        assert_eq!(Encoding::from_name("UTF-8"),  Encoding::Utf8);
        assert_eq!(Encoding::from_name("UTF8"),   Encoding::Utf8);
        assert_eq!(Encoding::from_name("utf-8"),  Encoding::Utf8); // case-insensitive
    }

    #[test]
    fn encoding_from_name_ascii_aliases() {
        for alias in &["ASCII", "US-ASCII", "ANSI_X3.4-1968", "IBM367"] {
            assert_eq!(
                Encoding::from_name(alias),
                Encoding::UsAscii,
                "alias: {alias}"
            );
        }
    }

    #[test]
    fn encoding_from_name_latin1_aliases() {
        for alias in &["ISO-8859-1", "ISO_8859-1", "LATIN1", "L1", "IBM819"] {
            assert_eq!(
                Encoding::from_name(alias),
                Encoding::Iso8859_1,
                "alias: {alias}"
            );
        }
    }

    #[test]
    fn encoding_from_name_koi8r() {
        assert_eq!(Encoding::from_name("KOI8-R"),  Encoding::Koi8R);
        assert_eq!(Encoding::from_name("CSKOI8R"), Encoding::Koi8R);
    }

    #[test]
    fn encoding_from_name_unknown() {
        assert_eq!(Encoding::from_name("bogus"),    Encoding::Unknown);
        assert_eq!(Encoding::from_name(""),         Encoding::Unknown);
        assert_eq!(Encoding::from_name("SHIFT_JIS"),Encoding::Unknown); // not supported
    }

    // ---- codepage tables ---------------------------------------------------

    #[test]
    fn iso8859_1_is_identity() {
        // ISO-8859-1 bytes 0x80–0xFF should map 1-to-1 to Unicode.
        let table = Encoding::Iso8859_1.codepage().unwrap();
        for i in 0usize..128 {
            assert_eq!(table[i] as usize, i + 0x80, "byte 0x{:02X}", i + 0x80);
        }
    }

    #[test]
    fn iso8859_15_euro_sign() {
        // In ISO-8859-15 byte 0xA4 is U+20AC EURO SIGN (not U+00A4 as in ISO-8859-1).
        let table = Encoding::Iso8859_15.codepage().unwrap();
        let idx = 0xA4usize - 0x80; // = 36
        assert_eq!(table[idx], 0x20AC);
    }

    #[test]
    fn koi8r_sample() {
        // The espeak-ng KOI8-R table actually contains ISO-8859-16 data
        // (a quirk of the C implementation we faithfully reproduce).
        // At byte 0xC1 the C table returns U+00C1 (LATIN CAPITAL A WITH ACUTE).
        let table = Encoding::Koi8R.codepage().unwrap();
        let idx = 0xC1usize - 0x80; // = 65
        assert_eq!(table[idx], 0x00C1,
            "espeak-ng KOI8-R table at 0xC1 should be U+00C1 (mirrors C source)");
    }

    // ---- TextDecoder -------------------------------------------------------

    #[test]
    fn text_decoder_utf8_hello() {
        let input = b"hello";
        let codepoints: Vec<u32> = TextDecoder::utf8(input).collect();
        assert_eq!(codepoints, vec![b'h' as u32, b'e' as u32, b'l' as u32,
                                    b'l' as u32, b'o' as u32]);
    }

    #[test]
    fn text_decoder_utf8_multibyte() {
        // "café" = c a f U+00E9
        let input = "café".as_bytes();
        let codepoints: Vec<u32> = TextDecoder::utf8(input).collect();
        assert_eq!(codepoints, vec![b'c' as u32, b'a' as u32, b'f' as u32, 0x00E9]);
    }

    #[test]
    fn text_decoder_null_terminates() {
        let input = b"hi\x00world";
        let codepoints: Vec<u32> = TextDecoder::utf8(input).collect();
        // Should stop at the null byte
        assert_eq!(codepoints, vec![b'h' as u32, b'i' as u32]);
    }

    #[test]
    fn text_decoder_iso8859_1() {
        // byte 0xE9 in ISO-8859-1 → U+00E9 (é)
        let input = &[0xE9u8];
        let mut dec = TextDecoder::new(input, Encoding::Iso8859_1, DecodeMode::Strict).unwrap();
        let cp = dec.next_codepoint().unwrap();
        assert_eq!(cp, 0x00E9);
    }

    #[test]
    fn text_decoder_iso8859_15_euro() {
        // byte 0xA4 in ISO-8859-15 → U+20AC (€)
        let input = &[0xA4u8];
        let mut dec = TextDecoder::new(input, Encoding::Iso8859_15, DecodeMode::Strict).unwrap();
        let cp = dec.next_codepoint().unwrap();
        assert_eq!(cp, 0x20AC);
    }

    #[test]
    fn text_decoder_ascii_rejects_high_bytes() {
        let input = &[0x80u8];
        let mut dec = TextDecoder::new(input, Encoding::UsAscii, DecodeMode::Strict).unwrap();
        let cp = dec.next_codepoint().unwrap();
        assert_eq!(cp, REPLACEMENT_CHAR);
    }

    #[test]
    fn text_decoder_auto_mode_utf8_first() {
        // "hello" in UTF-8 – auto mode should use UTF-8 cleanly
        let mut dec = TextDecoder::new(
            b"hi",
            Encoding::Iso8859_1, // fallback encoding
            DecodeMode::Auto,
        ).unwrap();
        assert_eq!(dec.next_codepoint(), Some(b'h' as u32));
        assert_eq!(dec.next_codepoint(), Some(b'i' as u32));
        assert!(!dec.fell_back, "should not have fallen back");
    }

    #[test]
    fn text_decoder_auto_mode_fallback_on_bad_utf8() {
        // 0xA4 is not valid UTF-8 lead byte; in auto mode with ISO-8859-15
        // it should fall back to the codepage and return U+20AC.
        let mut dec = TextDecoder::new(
            &[0xA4u8],
            Encoding::Iso8859_15,
            DecodeMode::Auto,
        ).unwrap();
        let cp = dec.next_codepoint().unwrap();
        assert_eq!(cp, 0x20AC, "expected euro sign U+20AC");
        assert!(dec.fell_back, "should have fallen back to codepage");
    }

    #[test]
    fn text_decoder_ucs2_hello() {
        // "Hi" in little-endian UCS-2: 0x48 0x00 0x69 0x00
        let input = &[0x48u8, 0x00, 0x69, 0x00];
        let codepoints: Vec<u32> = TextDecoder::new(input, Encoding::Ucs2, DecodeMode::Strict)
            .unwrap()
            .collect();
        assert_eq!(codepoints, vec![b'H' as u32, b'i' as u32]);
    }

    #[test]
    fn text_decoder_eof_flag() {
        let mut dec = TextDecoder::utf8(b"x");
        assert!(!dec.is_eof());
        dec.next_codepoint();
        assert!(dec.is_eof());
    }

    #[test]
    fn decode_utf8_to_string_emoji() {
        let s = "😀 world";
        let decoded = decode_utf8_to_string(s.as_bytes());
        assert_eq!(decoded, s);
    }

    #[test]
    fn decode_to_string_iso8859_1_cafe() {
        // "café" in ISO-8859-1 = b"caf\xE9"
        let input = b"caf\xE9";
        let s = decode_to_string(input, Encoding::Iso8859_1).unwrap();
        assert_eq!(s, "café");
    }

    #[test]
    fn decoder_error_on_unknown_encoding() {
        let result = TextDecoder::new(b"x", Encoding::Unknown, DecodeMode::Strict);
        assert!(result.is_err());
    }
}
