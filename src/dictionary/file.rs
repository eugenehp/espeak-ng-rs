//! Dictionary binary file loader.
//!
//! Corresponds to `LoadDictionary` and `InitGroups` in `dictionary.c`.
//!
//! # File layout
//! ```text
//! bytes  0-3              : u32le = N_HASH_DICT (1024) — integrity check
//! bytes  4-7              : u32le = rules_offset
//! bytes  8..rules_offset  : hash buckets (word list)
//! bytes  rules_offset..   : translation rule groups
//! ```

use std::path::Path;

use crate::Error;
use super::{
    N_HASH_DICT, N_LETTER_GROUPS, N_RULE_GROUP2,
    RULE_GROUP_START, RULE_GROUP_END, RULE_REPLACEMENTS, RULE_LETTERGP2,
};
use super::transpose::TransposeConfig;

// ─────────────────────────────────────────────────────────────────────────────
// Rule-group indexing
// ─────────────────────────────────────────────────────────────────────────────

/// All offsets here are **absolute** byte offsets into `Dictionary::data`.
/// A value of `usize::MAX` means "not present" (corresponds to NULL in C).

#[derive(Clone)]
/// Index of all rule groups within a [`Dictionary`].
///
/// Built by `init_groups()` which scans the rules section of the binary file
/// and records the offset to each rule chain, mirroring `InitGroups()` in C.
pub struct Groups {
    /// `groups1[c]` — offset to the rule chain for the single ASCII byte `c`.
    pub groups1: [Option<usize>; 256],

    /// `groups3[c2-1]` — offset to the rule chain for multi-byte alphabet
    /// sequences indexed by the second byte of the `\x01 c2` header.
    pub groups3: [Option<usize>; 128],

    /// Two-letter rule groups, searched in order.
    pub groups2: Vec<Group2Entry>,

    /// For each initial byte `c`: how many `groups2` entries start with `c`.
    pub groups2_count: [u8; 256],

    /// For each initial byte `c`: first index into `groups2` for that byte.
    pub groups2_start: [u8; 256],

    /// `letterGroups[ix]` — offset to the letter-group string list.
    /// Index is `(flag_byte - 'A')` where flag_byte may wrap around 256.
    pub letter_groups: [Option<usize>; N_LETTER_GROUPS],

    /// Absolute offset to the `replace_chars` table inside `data`, if any.
    pub replace_chars: Option<usize>,
}

/// One entry in the two-letter rule group index.
#[derive(Clone, Copy, Debug)]
pub struct Group2Entry {
    /// The two-char key: `c1 | (c2 << 8)` (little-endian uint16).
    pub key: u16,
    /// Absolute offset into `Dictionary::data` of the first rule.
    pub offset: usize,
}

impl Default for Groups {
    fn default() -> Self {
        Groups {
            groups1: [None; 256],
            groups3: [None; 128],
            groups2: Vec::new(),
            groups2_count: [0u8; 256],
            groups2_start: [255u8; 256], // 255 = "not set"
            letter_groups: [None; N_LETTER_GROUPS],
            replace_chars: None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Dictionary
// ─────────────────────────────────────────────────────────────────────────────

/// An in-memory view of a `<lang>_dict` file.
///
/// `data` is the raw file bytes.  All indices (`hashtab`, `groups.*`) are
/// **absolute byte offsets into `data`**.
pub struct Dictionary {
    pub data: Vec<u8>,

    /// Absolute byte offset to the translation-rules section.
    pub rules_offset: usize,

    /// `hashtab[hash]` — absolute offset to the start of bucket `hash`.
    pub hashtab: [usize; N_HASH_DICT],

    /// Rule-group index built by `init_groups()`.
    pub groups: Groups,

    /// BCP-47 language tag this dictionary was loaded for (e.g. `"en"`).
    pub lang: String,

    /// Compression configuration for word hashing and lookup.
    /// Most Latin-script languages use `TransposeConfig::LATIN`.
    pub transpose: TransposeConfig,

    /// Base Unicode codepoint for `groups3` indexing.
    ///
    /// Mirrors `Translator::letter_bits_offset` in C's `tr_languages.c`.
    /// When non-zero, `groups3[wc - letter_bits_offset]` maps a Unicode
    /// codepoint to its rule chain.
    ///
    /// | Language family | Value |
    /// |-----------------|-------|
    /// | Latin (default) | 0 |
    /// | Cyrillic (ru, bg, tt, uk, be) | 0x420 |
    /// | Arabic (ar) | 0x600 |
    /// | Farsi/Persian (fa) | 0x600 |
    pub letter_bits_offset: u32,

    /// Per-character letter-group bitmask table.
    ///
    /// Indexed by `(codepoint - letter_bits_offset) & 0x7f` (clamped to 128).
    /// Bit N is set iff the character belongs to letter group N
    /// (LETTERGP_A=0, LETTERGP_B=1, LETTERGP_C=2, LETTERGP_H=3,
    ///  LETTERGP_F=4, LETTERGP_G=5, LETTERGP_Y=6, LETTERGP_VOWEL2=7).
    ///
    /// For Latin scripts, the table uses direct ASCII indices (offset=0).
    pub letter_bits: Box<[u8; 256]>,
}

impl Dictionary {
    /// Load `<data_dir>/<lang>_dict`.
    pub fn load(lang: &str, data_dir: &Path) -> Result<Self, Error> {
        let path = data_dir.join(format!("{}_dict", lang));
        let data = std::fs::read(&path)
            .map_err(|e| Error::Io(e))?;

        Self::from_bytes(lang, data)
    }

    /// Parse an already-loaded byte buffer.
    pub fn from_bytes(lang: &str, data: Vec<u8>) -> Result<Self, Error> {
        // Header validation
        if data.len() < N_HASH_DICT + 8 {
            return Err(Error::InvalidData(
                format!("dict '{}': file too short ({} bytes)", lang, data.len())));
        }

        let pw0 = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
        let pw1 = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;

        if pw0 != N_HASH_DICT {
            return Err(Error::InvalidData(
                format!("dict '{}': bad magic 0x{:x} (expected 0x{:x})",
                    lang, pw0, N_HASH_DICT)));
        }
        if pw1 == 0 || pw1 > 0x800_0000 || pw1 > data.len() {
            return Err(Error::InvalidData(
                format!("dict '{}': bad rules_offset {}", lang, pw1)));
        }
        let rules_offset = pw1;

        // Build hash table.
        // C: p = &data_dictlist[8]; for each hash: record p, skip entries (length != 0), skip 0-terminator.
        let mut hashtab = [0usize; N_HASH_DICT];
        {
            let mut pos = 8usize;
            for hash in 0..N_HASH_DICT {
                hashtab[hash] = pos;
                // Skip entries in this bucket (each starts with its total length).
                loop {
                    if pos >= data.len() {
                        return Err(Error::InvalidData(
                            format!("dict '{}': hash table overran file at bucket {}", lang, hash)));
                    }
                    let entry_len = data[pos] as usize;
                    if entry_len == 0 { break; }
                    pos += entry_len;
                }
                pos += 1; // skip the 0-terminator
            }
        }

        // Parse rule groups.
        let groups = build_groups(&data, rules_offset)?;

        // Per-script configuration — mirrors `SelectTranslator()` in `tr_languages.c`.
        //
        // transpose:          controls dictionary word hashing/lookup compression
        // letter_bits_offset: base codepoint for groups3 rule indexing
        //
        // OFFSET_CYRILLIC = 0x420, OFFSET_ARABIC = 0x600 (from tr_languages.c)
        let (transpose, letter_bits_offset) = match lang {
            // Cyrillic-script languages: SetCyrillicLetters() → transpose_min=0x430,
            // letter_bits_offset=0x420 (OFFSET_CYRILLIC)
            "ru" | "bg" | "tt" | "uk" | "be" => (TransposeConfig::CYRILLIC, 0x420u32),
            // Arabic script (ar): transpose_min=0x600, letter_bits_offset=0x600
            "ar" => (TransposeConfig::ARABIC, 0x600u32),
            // Farsi/Persian: letter_bits_offset=0x600 (OFFSET_ARABIC)
            "fa" => (TransposeConfig::PERSIAN, 0x600u32),
            // All other languages: default Latin-script compression, no offset
            _ => (TransposeConfig::LATIN, 0u32),
        };

        let letter_bits = build_letter_bits(lang);

        Ok(Dictionary {
            data,
            rules_offset,
            hashtab,
            groups,
            lang: lang.to_owned(),
            transpose,
            letter_bits_offset,
            letter_bits,
        })
    }

    /// Return the slice containing all translation rules.
    #[inline]
    pub fn rules(&self) -> &[u8] {
        &self.data[self.rules_offset..]
    }

    /// Return the translation rule chain for a single ASCII byte `c` (groups1),
    /// as a slice starting at the first rule.
    #[inline]
    pub fn group1(&self, c: u8) -> Option<&[u8]> {
        self.groups.groups1[c as usize].map(|off| &self.data[off..])
    }

    /// Same for the two-byte chain.
    /// Returns `(matching_slice, advance_bytes)` for the best-matching group2 entry.
    #[inline]
    pub fn group2_entries_for(&self, c: u8) -> impl Iterator<Item = &Group2Entry> {
        let start = self.groups.groups2_start[c as usize] as usize;
        let count = self.groups.groups2_count[c as usize] as usize;
        if start >= self.groups.groups2.len() || count == 0 {
            self.groups.groups2[0..0].iter()
        } else {
            let end = (start + count).min(self.groups.groups2.len());
            self.groups.groups2[start..end].iter()
        }
    }

    /// Rule slice for a groups2 entry.
    #[inline]
    pub fn group2_rules(&self, e: &Group2Entry) -> &[u8] {
        &self.data[e.offset..]
    }

    /// Rule slice for a groups3 entry.
    #[inline]
    pub fn group3(&self, c2: u8) -> Option<&[u8]> {
        let idx = c2.wrapping_sub(1) as usize;
        if idx >= 128 { return None; }
        self.groups.groups3[idx].map(|off| &self.data[off..])
    }

    /// Letter-group string list for `letterGroups[ix]`.
    #[inline]
    pub fn letter_group(&self, ix: usize) -> Option<&[u8]> {
        if ix >= N_LETTER_GROUPS { return None; }
        self.groups.letter_groups[ix].map(|off| &self.data[off..])
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// InitGroups — build the rule-group index
// ─────────────────────────────────────────────────────────────────────────────

fn build_groups(data: &[u8], rules_offset: usize) -> Result<Groups, Error> {
    let mut g = Groups::default();
    let mut n_groups2 = 0usize;

    let rules = &data[rules_offset..];
    let mut pos = 0usize; // offset into `rules` (relative to rules_offset)

    // If there are no rules at all, the file just starts with RULE_GROUP_END.
    if rules.is_empty() || rules[pos] == RULE_GROUP_END {
        return Ok(g);
    }

    while pos < rules.len() && rules[pos] != 0 {
        if rules[pos] != RULE_GROUP_START {
            return Err(Error::InvalidData(
                format!("bad rules data: expected RULE_GROUP_START at offset {}", rules_offset + pos)));
        }
        pos += 1; // skip RULE_GROUP_START

        // Check for special group types
        if rules[pos] == RULE_REPLACEMENTS {
            // Advance to next 4-byte-aligned position (relative to data start)
            let abs = rules_offset + pos + 4;
            let aligned = (abs + 3) & !3;
            // replace_chars table starts there
            g.replace_chars = Some(aligned);
            // skip forward until RULE_GROUP_END
            pos = aligned - rules_offset;
            while pos < rules.len() && rules[pos] != RULE_GROUP_END {
                pos += 1;
            }
            pos += 1; // skip RULE_GROUP_END
            continue;
        }

        if rules[pos] == RULE_LETTERGP2 {
            // p[0] = RULE_LETTERGP2, p[1] = group-index byte
            let idx_byte = rules[pos + 1];
            let ix = if idx_byte < b'A' {
                // negative wrap-around: (idx_byte - 'A') mod 256 as signed
                (idx_byte as i16 - b'A' as i16 + 256) as usize
            } else {
                (idx_byte - b'A') as usize
            };
            pos += 2;
            if ix < N_LETTER_GROUPS {
                g.letter_groups[ix] = Some(rules_offset + pos);
            }
        } else {
            // Regular group: name string followed by \0
            let name_start = pos;
            while pos < rules.len() && rules[pos] != 0 {
                pos += 1;
            }
            let name_len = pos - name_start;
            let c  = rules[name_start];       // first byte of group name
            let c2 = if name_len >= 2 { rules[name_start + 1] } else { 0 };
            pos += 1; // skip the \0

            // abs offset of first rule for this group
            let rule_abs = rules_offset + pos;

            match name_len {
                0 => { g.groups1[0] = Some(rule_abs); }
                1 => { g.groups1[c as usize] = Some(rule_abs); }
                _ if c == 1 => {
                    // groups3 indexed by c2-1
                    let idx = c2.wrapping_sub(1) as usize;
                    if idx < 128 {
                        g.groups3[idx] = Some(rule_abs);
                    }
                }
                _ => {
                    // Two-letter group
                    if g.groups2_start[c as usize] == 255 {
                        g.groups2_start[c as usize] = n_groups2 as u8;
                    }
                    g.groups2_count[c as usize] =
                        g.groups2_count[c as usize].saturating_add(1);
                    let key = (c as u16) | ((c2 as u16) << 8);
                    if n_groups2 < N_RULE_GROUP2 {
                        g.groups2.push(Group2Entry { key, offset: rule_abs });
                        n_groups2 += 1;
                    }
                }
            }
        }

        // Skip over all rules in this group until RULE_GROUP_END
        // Each rule is a null-terminated string.
        while pos < rules.len() && rules[pos] != RULE_GROUP_END {
            while pos < rules.len() && rules[pos] != 0 {
                pos += 1;
            }
            pos += 1; // skip the \0 terminator of this rule
        }
        pos += 1; // skip RULE_GROUP_END
    }

    Ok(g)
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-language letter_bits table builder
// ─────────────────────────────────────────────────────────────────────────────

/// Build the letter group bitmask table for `lang`.
///
/// The table is indexed by `(codepoint - letter_bits_offset) & 0x7f`.
/// Bit N = 1 iff the character belongs to letter group N:
///   0=A(vowel)  1=B(soft)  2=C(consonant)  3=H(hard)
///   4=F(not-hard)  5=G(voiced)  6=Y(iotated/front)  7=VOWEL2
///
/// Letter group data mirrors `SetCyrillicLetters()` / `SetLetterBits()` in
/// `tr_languages.c`.
fn build_letter_bits(lang: &str) -> Box<[u8; 256]> {
    let mut bits = Box::new([0u8; 256]);

    match lang {
        "ru" | "bg" | "uk" | "be" | "tt" => {
            // Cyrillic letter groups — `letter_bits_offset = OFFSET_CYRILLIC = 0x420`
            // Indices are (codepoint - 0x420); stored at that index in bits[].

            // LETTERGP_A (0) = vowels: а е ё и о у ы э ю я (0x10,0x15,0x31,0x18,0x1e,0x23,0x2b,0x2d,0x2e,0x2f)
            const RU_VOWELS: &[usize] = &[0x10, 0x15, 0x31, 0x18, 0x1e, 0x23, 0x2b, 0x2d, 0x2e, 0x2f];
            // LETTERGP_B (1) = soft consonants: ь й ч щ (0x2c,0x19,0x27,0x29)
            const CYRL_SOFT: &[usize] = &[0x2c, 0x19, 0x27, 0x29];
            // LETTERGP_C (2) = consonants
            const RU_CONSONANTS: &[usize] = &[
                0x11,0x12,0x13,0x14,0x16,0x17,0x19,0x1a,0x1b,0x1c,
                0x1d,0x1f,0x20,0x21,0x22,0x24,0x25,0x26,0x27,0x28,
                0x29,0x2a,0x2c,
            ];
            // LETTERGP_H (3) = hard consonants: ъ ж ц ш (0x2a,0x16,0x26,0x28)
            const CYRL_HARD: &[usize] = &[0x2a, 0x16, 0x26, 0x28];
            // LETTERGP_F (4) = not-hard
            const CYRL_NOTHARD: &[usize] = &[
                0x11,0x12,0x13,0x14,0x17,0x19,0x1a,0x1b,0x1c,0x1d,
                0x1f,0x20,0x21,0x22,0x24,0x25,0x27,0x29,0x2c,
            ];
            // LETTERGP_G (5) = voiced obstruents: б в г д ж з (0x11-0x14,0x16-0x17)
            const CYRL_VOICED: &[usize] = &[0x11,0x12,0x13,0x14,0x16,0x17];
            // LETTERGP_Y (6) = iotated vowels + soft sign
            //   SetCyrillicLetters: ь ю я ё (0x2c,0x2e,0x2f,0x31)
            //   Translator_Russian adds: е и є ї (0x15,0x18,0x34,0x37)
            const CYRL_IVOWELS: &[usize] = &[0x2c, 0x2e, 0x2f, 0x31, 0x15, 0x18, 0x34, 0x37];

            fn set_bits(bits: &mut [u8; 256], indices: &[usize], group: u8) {
                for &idx in indices {
                    if idx < 256 { bits[idx] |= 1 << group; }
                }
            }

            set_bits(&mut bits, RU_VOWELS,     0); // LETTERGP_A
            set_bits(&mut bits, CYRL_SOFT,     1); // LETTERGP_B
            set_bits(&mut bits, RU_CONSONANTS, 2); // LETTERGP_C
            set_bits(&mut bits, CYRL_HARD,     3); // LETTERGP_H
            set_bits(&mut bits, CYRL_NOTHARD,  4); // LETTERGP_F
            set_bits(&mut bits, CYRL_VOICED,   5); // LETTERGP_G
            set_bits(&mut bits, CYRL_IVOWELS,  6); // LETTERGP_Y
            set_bits(&mut bits, RU_VOWELS,     7); // LETTERGP_VOWEL2
        }
        _ => {
            // Latin/default: use the English letter bits
            let en = crate::translate::english_letter_bits();
            bits.copy_from_slice(en.as_slice());
        }
    }

    bits
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn data_dir() -> PathBuf {
        PathBuf::from("/usr/share/espeak-ng-data")
    }

    fn try_load(lang: &str) -> Option<Dictionary> {
        let dir = data_dir();
        if !dir.join(format!("{}_dict", lang)).exists() {
            return None;
        }
        Some(Dictionary::load(lang, &dir).expect("load succeeded"))
    }

    #[test]
    fn load_en_dict() {
        let dict = match try_load("en") { Some(d) => d, None => return };
        assert_eq!(dict.lang, "en");
        assert_eq!(dict.rules_offset, 0x0001_b188,
            "rules_offset should be 0x1b188 for installed en_dict");
        // groups1['a'] must be non-null (English has 'a' rules)
        assert!(dict.groups.groups1[b'a' as usize].is_some(),
            "groups1['a'] should be set");
    }

    #[test]
    fn hash_table_covers_all_buckets() {
        let dict = match try_load("en") { Some(d) => d, None => return };
        // Every hash table entry must point somewhere inside the data.
        for (i, &off) in dict.hashtab.iter().enumerate() {
            assert!(off < dict.data.len(),
                "hashtab[{}] = {} out of bounds (len={})", i, off, dict.data.len());
        }
    }

    #[test]
    fn group1_default_is_some() {
        let dict = match try_load("en") { Some(d) => d, None => return };
        // groups1[0] is the default rule chain; it should be present in English
        assert!(dict.groups.groups1[0].is_some(),
            "default rule chain (groups1[0]) should be set for English");
    }

    #[test]
    fn de_dict_loads() {
        let _ = try_load("de"); // just check it doesn't panic
    }

    #[test]
    fn fr_dict_loads() {
        let _ = try_load("fr");
    }
}
