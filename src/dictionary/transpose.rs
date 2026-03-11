//! Script-specific alphabet compression for dictionary word storage.
//!
//! Port of `TransposeAlphabet()` from `dictionary.c`.
//!
//! Each character is mapped to a 6-bit code via a per-language table, then
//! the 6-bit codes are packed into bytes.  If any character falls outside the
//! mapped range the word is returned uncompressed (bit 6 NOT set in returned
//! length).
//!
//! The returned `wlen` mirrors the C return value:
//!   bit 6 SET   → compressed;  lower 6 bits = byte count
//!   bit 6 CLEAR → uncompressed; value = byte count (= UTF-8 byte length)

// ---------------------------------------------------------------------------
// Latin transpose map (from `tr_languages.c: transpose_map_latin[]`)
// ---------------------------------------------------------------------------

/// Index: `char_code - 0x60` → 6-bit code (0 = not in alphabet).
pub static TRANSPOSE_MAP_LATIN: &[u8] = &[
     0,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14, 15, // 0x60
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,  0,  0,  0,  0,  0, // 0x70
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x80
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x90
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0xa0
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0xb0
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0xc0
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0xd0
    27, 28, 29,  0,  0, 30, 31, 32, 33, 34, 35, 36,  0, 37, 38,  0, // 0xe0
     0,  0,  0, 39,  0,  0, 40,  0, 41,  0, 42,  0, 43,  0,  0,  0, // 0xf0
     0,  0,  0, 44,  0, 45,  0, 46,  0,  0,  0,  0,  0, 47,  0,  0, // 0x100
     0, 48,  0,  0,  0,  0,  0,  0,  0, 49,  0,  0,  0,  0,  0,  0, // 0x110
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x120
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x130
     0,  0, 50,  0, 51,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x140
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 52,  0,  0,  0,  0, // 0x150
     0, 53,  0, 54,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x160
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 55,  0, 56,  0, 57,  0, // 0x170
];

// ---------------------------------------------------------------------------
// Farsi/Persian transpose map (from `tr_languages.c: transpose_map_fa[]`)
// Index: `char_code - 0x620` → 6-bit code (0 = not mapped).
// Used by: fa (Farsi/Persian)
// ---------------------------------------------------------------------------
pub static TRANSPOSE_MAP_FA: &[u8] = &[
     0,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14, 15, // 0x620
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,  0,  0,  0,  0,  0, // 0x630
     0, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, // 0x640
    42, 43,  0,  0, 44,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x650
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x660
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 45,  0, // 0x670
     0,  0,  0,  0,  0,  0, 46,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x680
     0,  0,  0,  0,  0,  0,  0,  0, 47,  0,  0,  0,  0,  0,  0,  0, // 0x690
     0,  0,  0,  0,  0,  0,  0,  0,  0, 48,  0,  0,  0,  0,  0, 49, // 0x6a0
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x6b0
    50,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 51,             // 0x6c0–0x6cc
];

// ---------------------------------------------------------------------------
// Frequent 2-character pairs for Russian (and all Cyrillic-script languages).
// From `tr_languages.c: pairs_ru[]`.  Each u16 = low_char | (high_char << 8).
// Sorted ascending; sentinel = 0x7fff.
// ---------------------------------------------------------------------------
pub static PAIRS_RU: &[u16] = &[
    0x010c, // ла
    0x010e, // на
    0x0113, // та
    0x0301, // ав
    0x030f, // ов
    0x060e, // не
    0x0611, // ре
    0x0903, // ви
    0x0b01, // ак
    0x0b0f, // ок
    0x0c01, // ал
    0x0c09, // ил
    0x0e01, // ан
    0x0e06, // ен
    0x0e09, // ин
    0x0e0e, // нн
    0x0e0f, // он
    0x0e1c, // ын
    0x0f03, // во
    0x0f11, // ро
    0x0f12, // со
    0x100f, // оп
    0x1011, // рп
    0x1101, // ар
    0x1106, // ер
    0x1109, // ир
    0x110f, // ор
    0x1213, // тс
    0x1220, // яс
    0x7fff, // sentinel
];

// ---------------------------------------------------------------------------
// TransposeConfig
// ---------------------------------------------------------------------------

/// Configuration for `TransposeAlphabet`.  Mirrors per-language fields in
/// the C `Translator` struct.
#[derive(Clone, Debug)]
pub struct TransposeConfig {
    /// Minimum Unicode codepoint in the mapped range (inclusive).
    pub transpose_min: u32,
    /// Maximum Unicode codepoint in the mapped range (inclusive).
    pub transpose_max: u32,
    /// Explicit character-to-code mapping table.
    ///
    /// `Some(map)` — `map[c - transpose_min]` → 6-bit code (0 = not mapped).
    /// `None`      — use direct subtraction: `code = c - transpose_min + 1`.
    ///               Mirrors C behaviour when `tr->transpose_map == NULL`.
    pub transpose_map: Option<&'static [u8]>,
    /// Frequent 2-character pairs.  `None` = no pair compression.
    pub frequent_pairs: Option<&'static [u16]>,
}

impl TransposeConfig {
    /// Latin-script configuration (English and most European languages).
    /// `transpose_min = 0x60`, `transpose_max = 0x17f`, explicit map.
    pub const LATIN: Self = TransposeConfig {
        transpose_min: 0x60,
        transpose_max: 0x17f,
        transpose_map: Some(TRANSPOSE_MAP_LATIN),
        frequent_pairs: None,
    };

    /// Cyrillic-script configuration.
    /// Used by: ru, bg, tt, uk, and other Cyrillic-script languages that call
    /// `SetCyrillicLetters()` in `tr_languages.c`.
    ///
    /// Range 0x430–0x451 (basic Cyrillic block), direct subtraction,
    /// with the Russian frequent-pair table.
    pub const CYRILLIC: Self = TransposeConfig {
        transpose_min: 0x430,
        transpose_max: 0x451,
        transpose_map: None, // code = c - 0x430 + 1
        frequent_pairs: Some(PAIRS_RU),
    };

    /// Arabic-script configuration.
    /// Used by: ar.
    ///
    /// Range 0x600–0x65f, direct subtraction, no pairs.
    pub const ARABIC: Self = TransposeConfig {
        transpose_min: 0x600,
        transpose_max: 0x65f,
        transpose_map: None, // code = c - 0x600 + 1
        frequent_pairs: None,
    };

    /// Farsi/Persian-script configuration.
    /// Used by: fa.
    ///
    /// Range 0x620–0x6cc, explicit map, no pairs.
    pub const PERSIAN: Self = TransposeConfig {
        transpose_min: 0x620,
        transpose_max: 0x6cc,
        transpose_map: Some(TRANSPOSE_MAP_FA),
        frequent_pairs: None,
    };

    /// No compression (transparent pass-through of raw UTF-8 bytes).
    pub const NONE: Self = TransposeConfig {
        transpose_min: 0,
        transpose_max: 0,
        transpose_map: None,
        frequent_pairs: None,
    };

    /// Returns `true` when this config applies any script-specific compression.
    pub fn is_active(&self) -> bool { self.transpose_min > 0 }
}

// ---------------------------------------------------------------------------
// TransposeResult
// ---------------------------------------------------------------------------

/// Result of `transpose_alphabet`.
#[derive(Clone, Debug)]
pub struct TransposeResult {
    /// The (possibly compressed) word bytes.
    pub bytes: Vec<u8>,
    /// The `wlen` value as used by the C code:
    ///   bit 6 = compressed flag; lower bits = byte count.
    pub wlen: u8,
}

impl TransposeResult {
    /// Whether the word was stored in compressed form.
    pub fn is_compressed(&self) -> bool { self.wlen & 0x40 != 0 }
    /// The actual byte length for comparison purposes.
    pub fn byte_len(&self) -> u8 { self.wlen & 0x3f }
}

// ---------------------------------------------------------------------------
// transpose_alphabet
// ---------------------------------------------------------------------------

/// Apply the `TransposeAlphabet` compression, producing the key used for
/// dictionary hashing and entry comparison.
///
/// `word` must be a UTF-8 string (lowercase, no trailing space/null).
///
/// Mirrors `TransposeAlphabet(tr, text)` from `dictionary.c`.
pub fn transpose_alphabet(word: &str, cfg: &TransposeConfig) -> TransposeResult {
    if !cfg.is_active() {
        // No compression — use raw UTF-8 bytes.
        let bytes = word.as_bytes().to_vec();
        let wlen = bytes.len() as u8;
        return TransposeResult { bytes, wlen };
    }

    // The C code: pairs_start = max - min + 2
    let pairs_start = cfg.transpose_max - cfg.transpose_min + 2;
    let mut codes: Vec<u32> = Vec::with_capacity(word.len());
    let mut all_alpha = true;

    // ── Step 1: map each Unicode codepoint to a code. ────────────────────
    for c in word.chars() {
        let cp = c as u32;
        if cp >= cfg.transpose_min && cp <= cfg.transpose_max {
            let code = match cfg.transpose_map {
                None => {
                    // Direct subtraction: code = c - (transpose_min - 1)
                    // i.e.  code = c - transpose_min + 1  (1-indexed)
                    cp - cfg.transpose_min + 1
                }
                Some(map) => {
                    let idx = (cp - cfg.transpose_min) as usize;
                    if idx < map.len() { map[idx] as u32 } else { 0 }
                }
            };
            if code > 0 {
                codes.push(code);
                continue;
            }
        }
        all_alpha = false;
        break;
    }

    if !all_alpha {
        // Not all chars mapped — store uncompressed (raw UTF-8).
        let bytes = word.as_bytes().to_vec();
        let wlen = bytes.len() as u8;
        return TransposeResult { bytes, wlen };
    }

    // ── Step 2: replace frequent 2-char pairs with single codes. ─────────
    //
    // C code iterates the pairs list forward while `c2 >= pairs_list[ix]`.
    if let Some(pairs_list) = cfg.frequent_pairs {
        let mut i = 0;
        let mut merged: Vec<u32> = Vec::with_capacity(codes.len());
        while i < codes.len() {
            if i + 1 < codes.len() {
                let c2 = codes[i] | (codes[i + 1] << 8);
                let mut found = false;
                for (ix, &pair) in pairs_list.iter().enumerate() {
                    if pair == 0x7fff || pair == 0 { break; }
                    if c2 as u16 == pair {
                        merged.push(ix as u32 + pairs_start);
                        i += 2;
                        found = true;
                        break;
                    }
                    if c2 < pair as u32 { break; }
                }
                if !found {
                    merged.push(codes[i]);
                    i += 1;
                }
            } else {
                merged.push(codes[i]);
                i += 1;
            }
        }
        codes = merged;
    }

    // ── Step 3: pack 6-bit codes into bytes. ─────────────────────────────
    let mut out: Vec<u8> = Vec::with_capacity((codes.len() * 6 + 7) / 8);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;

    for c in &codes {
        acc = (acc << 6) | (c & 0x3f);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    if bits > 0 {
        out.push(((acc << (8 - bits)) & 0xff) as u8);
    }

    let byte_count = out.len() as u8;
    let wlen = byte_count | 0x40; // bit 6 = compressed
    TransposeResult { bytes: out, wlen }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transpose_a() {
        let r = transpose_alphabet("a", &TransposeConfig::LATIN);
        assert!(r.is_compressed(), "'a' should compress");
        // 'a' = 0x61, offset = 0x60, code = map[1] = 1
        // pack: acc=1, bits=6 → after loop: (1 << 2) = 4
        assert_eq!(r.bytes, &[0x04]);
        assert_eq!(r.byte_len(), 1);
    }

    #[test]
    fn transpose_the() {
        let r = transpose_alphabet("the", &TransposeConfig::LATIN);
        assert!(r.is_compressed());
        // 't' → code 20, 'h' → code 8, 'e' → code 5
        assert_eq!(r.bytes, &[0x50, 0x81, 0x40]);
        assert_eq!(r.byte_len(), 3);
        assert_eq!(r.wlen, 0x43);
    }

    #[test]
    fn transpose_hello() {
        let r = transpose_alphabet("hello", &TransposeConfig::LATIN);
        assert!(r.is_compressed());
        // 'h'→8, 'e'→5, 'l'→12, 'l'→12, 'o'→15
        // 5 × 6 = 30 bits → 4 bytes
        assert_eq!(r.byte_len(), 4);
    }

    #[test]
    fn transpose_no_compress() {
        // '1' is below transpose_min (0x60), so not mapped → uncompressed
        let r = transpose_alphabet("abc123", &TransposeConfig::LATIN);
        assert!(!r.is_compressed());
        assert_eq!(r.bytes, b"abc123");
    }

    #[test]
    fn transpose_none_config() {
        let r = transpose_alphabet("hello", &TransposeConfig::NONE);
        assert!(!r.is_compressed());
        assert_eq!(r.bytes, b"hello");
    }

    #[test]
    fn hash_compressed_the() {
        use crate::dictionary::lookup::hash_word;
        let r = transpose_alphabet("the", &TransposeConfig::LATIN);
        let h = hash_word(&r.bytes);
        assert_eq!(h, 75, "hash of compressed 'the' should be 75");
    }

    #[test]
    fn hash_compressed_a() {
        use crate::dictionary::lookup::hash_word;
        let r = transpose_alphabet("a", &TransposeConfig::LATIN);
        let h = hash_word(&r.bytes);
        assert_eq!(h, 5, "hash of compressed 'a' should be 5");
    }

    // ── Cyrillic ──────────────────────────────────────────────────────────

    #[test]
    fn cyrillic_basic_mapping() {
        // а (U+0430) → code 1, б (U+0431) → code 2
        // Direct subtraction: code = codepoint - 0x430 + 1
        let r = transpose_alphabet("а", &TransposeConfig::CYRILLIC);
        assert!(r.is_compressed(), "Cyrillic 'а' should compress");
        // Single code 1 → 6 bits → 0b000001_00 = 0x04
        assert_eq!(r.bytes, &[0x04]);
    }

    #[test]
    fn cyrillic_privet() {
        // привет = п(0x10) р(0x11) и(0x09) в(0x03) е(0x06) т(0x13)
        let r = transpose_alphabet("привет", &TransposeConfig::CYRILLIC);
        assert!(r.is_compressed());
        // 6 codes × 6 bits = 36 bits → 5 bytes (32 used + 4 padding bits)
        assert_eq!(r.byte_len(), 5);
    }
}
