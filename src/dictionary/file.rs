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

    /// The voice's `dictrules` bitmask (C's `tr->dict_condition`), which selects
    /// between `?n` variant entries — European `pt` takes `?1 _14 k&t'o*zy`,
    /// Brazilian `pt-BR` takes `?2 _14 kat'o*zy`.  Set by the translator after
    /// loading; `0` means no conditional entry applies.
    pub dict_condition: u32,

    /// Compression configuration for word hashing and lookup.
    /// Most Latin-script languages use `TransposeConfig::LATIN`.
    pub transpose: TransposeConfig,

    /// Base Unicode codepoint for `groups3` indexing.
    ///
    /// Mirrors `Translator::letter_bits_offset` in C's `tr_languages.c`.
    /// When non-zero, `groups3[wc - letter_bits_offset]` maps a Unicode
    /// codepoint to its rule chain.
    ///
    /// | Script | Offset | Languages |
    /// |--------|--------|-----------|
    /// | Latin (default) | 0 | most |
    /// | Cyrillic | 0x420 | ru, bg, tt, uk, be, mk, mn, kk, ky, ba, cv, tg, os, cu |
    /// | Greek | 0x380 | el, grc |
    /// | Armenian | 0x530 | hy, hyw |
    /// | Georgian | 0x10A0 | ka |
    /// | Indic (ISCII layout) | block start | Devanagari hi/mr/ne/sa/kok 0x900, Bengali bn/as 0x980, Gurmukhi pa 0xA00, Gujarati gu 0xA80, Oriya or 0xB00, Tamil ta 0xB80, Telugu te 0xC00, Kannada kn 0xC80, Malayalam ml 0xD00 |
    /// | Arabic | 0x600 | ar, fa |
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

    /// Decoded `.replace` table (`from` → `to` character-sequence pairs),
    /// applied to a word before dict/rule lookup.  Empty when the rules file
    /// has no `.replace` section.  See [`Dictionary::apply_replacements`].
    pub replace_pairs: Vec<(Vec<char>, String)>,
}

/// The base codepoint a language's `groups3` rule index is relative to — C's
/// `Translator::letter_bits_offset`.  `0` for the Latin-script languages, which
/// key their rules on the letters themselves.
pub fn letter_bits_offset_for(lang: &str) -> u32 {
    match lang {
            // Cyrillic-script: OFFSET_CYRILLIC
            "ru" | "bg" | "tt" | "uk" | "be" | "mk" | "mn" | "kk" | "ky"
            | "ba" | "cv" | "tg" | "os" | "cu" => 0x420u32,
            // Greek: OFFSET_GREEK
            "el" | "grc" => 0x380u32,
            // Armenian: OFFSET_ARMENIAN
            "hy" | "hyw" => 0x530u32,
            // Georgian: OFFSET_GEORGIAN
            "ka" => 0x10A0u32,
            // Indic scripts (block start = offset).  They share the ISCII-derived
            // relative layout, so one letter-group classification serves all.
            "hi" | "mr" | "ne" | "sa" | "kok" => 0x900u32, // Devanagari
            "bn" | "as" | "bpy" => 0x980u32,               // Bengali/Assamese/Bishnupriya
            "pa" => 0xA00u32,                              // Gurmukhi
            "gu" => 0xA80u32,                              // Gujarati
            "or" => 0xB00u32,                              // Oriya
            "ta" => 0xB80u32,                              // Tamil
            "te" => 0xC00u32,                              // Telugu
            "kn" => 0xC80u32,                              // Kannada
            "ml" => 0xD00u32,                              // Malayalam
            // Sinhala — its own (non-ISCII) block; the si_rules carry the
            // abugida handling via rule-file letter groups (.L01…) and virama
            // (U+0DCA) context, so only the groups3 offset is needed here.
            "si" => 0xD80u32,                              // Sinhala
            // Arabic script: Arabic, Farsi/Persian, Urdu, Sindhi — key their
            // rules on the U+0600 block, so index groups3 from there.  (Uyghur
            // `ug` also uses Arabic script but resolves via groups2 and works
            // without the offset, so it's intentionally left off.)
            "ar" | "fa" | "ur" | "sd" => 0x600u32,
            // Korean: the Hangul-syllable input is decomposed to conjoining jamo
            // (U+1100 leading, U+1161 vowel, U+11A7 trailing) before lookup, and
            // ko_rules keys its groups on those jamo — so index groups3 from the
            // jamo block start.
            "ko" => 0x1100u32,
            // All other languages: no offset
            _ => 0u32,
    }
}

impl Dictionary {
    /// Load `<data_dir>/<lang>_dict`.
    pub fn load(lang: &str, data_dir: &Path) -> Result<Self, Error> {
        // Accept both the flat `<lang>_dict` and the `dicts/<lang>_dict` layout
        // upstream #2261 introduces.
        let path = crate::translate::dict_path(data_dir, lang)
            .unwrap_or_else(|| data_dir.join(format!("{}_dict", lang)));
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
        // transpose (word hashing compression) is chosen by script; keep the
        // offsets in sync with `TransposeConfig::for_lang`.
        let transpose = TransposeConfig::for_lang(lang);
        let letter_bits_offset = letter_bits_offset_for(lang);

        let letter_bits = build_letter_bits(lang);

        let replace_pairs = groups.replace_chars
            .map(|off| decode_replace_table(&data, off))
            .unwrap_or_default();

        Ok(Dictionary {
            dict_condition: 0,
            data,
            rules_offset,
            hashtab,
            groups,
            lang: lang.to_owned(),
            transpose,
            letter_bits_offset,
            letter_bits,
            replace_pairs,
        })
    }

    /// Apply the `.replace` table to `word`, returning the substituted string
    /// (or `None` when nothing matched, so callers can avoid an allocation).
    ///
    /// Mirrors C's `SubstituteChar`/`FindReplacementChars`: scan left-to-right,
    /// and at each position use the **first** table entry whose `from` sequence
    /// matches the input there, emitting its (1–2 char) `to`.  Used e.g. by
    /// Japanese to fold katakana→hiragana and decompose precomposed voiced kana
    /// (`だ`→`た`+゙) into the form the pronunciation rules match.
    pub fn apply_replacements(&self, word: &str) -> Option<String> {
        if self.replace_pairs.is_empty() {
            return None;
        }
        let chars: Vec<char> = word.chars().collect();
        let mut out = String::with_capacity(word.len());
        let mut i = 0;
        let mut changed = false;
        while i < chars.len() {
            let mut matched = false;
            for (from, to) in &self.replace_pairs {
                if from.len() <= chars.len() - i && chars[i..i + from.len()] == from[..] {
                    out.push_str(to);
                    i += from.len();
                    matched = true;
                    changed = true;
                    break;
                }
            }
            if !matched {
                out.push(chars[i]);
                i += 1;
            }
        }
        changed.then_some(out)
    }

    /// Return the slice containing all translation rules.
    #[inline]
    pub fn rules(&self) -> &[u8] {
        &self.data[self.rules_offset..]
    }

    #[cfg(test)]
    pub fn replace_pairs_for_test(&self) -> &[(Vec<char>, String)] {
        &self.replace_pairs
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

/// Decode the `.replace` table at `offset` into `from`→`to` pairs.
///
/// Layout (from `compiledict.c`): a run of entries, each a NUL-terminated UTF-8
/// `from` string followed by a NUL-terminated UTF-8 `to` string.  The table
/// ends where the next four bytes are all NUL (C's `is_str_totally_null(p, 4)`),
/// i.e. at an empty `from`.
fn decode_replace_table(data: &[u8], offset: usize) -> Vec<(Vec<char>, String)> {
    let read_cstr = |pos: &mut usize| -> &[u8] {
        let start = *pos;
        while *pos < data.len() && data[*pos] != 0 {
            *pos += 1;
        }
        let s = &data[start..*pos];
        if *pos < data.len() {
            *pos += 1; // skip the terminating NUL
        }
        s
    };

    let mut pairs = Vec::new();
    let mut pos = offset;
    while pos + 4 <= data.len() && data[pos..pos + 4].iter().any(|&b| b != 0) {
        let from = String::from_utf8_lossy(read_cstr(&mut pos)).into_owned();
        let to = String::from_utf8_lossy(read_cstr(&mut pos)).into_owned();
        if from.is_empty() {
            break;
        }
        pairs.push((from.chars().collect(), to));
    }
    pairs
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
            // The table starts at `(&RULE_REPLACEMENTS + 4) & ~3` — C rounds the
            // (memory) address *down* to a 4-byte boundary (`((intptr_t)p+4) &
            // ~3` in dictionary.c); `data` is itself 4-aligned, so the same
            // arithmetic on the file offset gives the identical position.
            let abs = rules_offset + pos + 4;
            let aligned = abs & !3;
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
        "ru" | "bg" | "uk" | "be" | "tt" | "mk" | "mn" | "kk" | "ky"
        | "ba" | "cv" | "tg" | "os" | "cu" => {
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
            const CYRL_IVOWELS: &[usize] = &[0x2c, 0x2e, 0x2f, 0x31];
            //   Russian and Ukrainian go through `Translator_Russian`, which
            //   *adds* "е и є ї" to the group.  Applying those to every Cyrillic
            //   language palatalised Bulgarian's `в` before `е`
            //   (`zdr'av;ej` for `zdr'avej`).
            const RU_EXTRA_IVOWELS: &[usize] = &[0x15, 0x18, 0x34, 0x37];

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
            if matches!(lang, "ru" | "uk") {
                set_bits(&mut bits, RU_EXTRA_IVOWELS, 6);
            }
            set_bits(&mut bits, RU_VOWELS,     7); // LETTERGP_VOWEL2
        }
        "el" | "grc" => {
            // Greek letter groups — `letter_bits_offset = OFFSET_GREEK = 0x380`.
            // Index = (codepoint - 0x380) & 0x7f.
            let set = |bits: &mut [u8; 256], chars: &str, group: u8| {
                for c in chars.chars() {
                    let idx = ((c as u32).wrapping_sub(0x380) & 0x7f) as usize;
                    bits[idx] |= 1 << group;
                }
            };
            // Vowels (mono/dia-critics included), upper- and lowercase.
            let vowels = "αεηιουωΑΕΗΙΟΥΩάέήίόύώϊϋΐΰ";
            // All consonants.
            let consonants = "βγδζθκλμνξπρσςτφχψΒΓΔΖΘΚΛΜΝΞΠΡΣΤΦΧΨ";
            set(&mut bits, vowels, 0);     // LETTERGP_A
            set(&mut bits, consonants, 2); // LETTERGP_C
            set(&mut bits, vowels, 7);     // LETTERGP_VOWEL2
        }
        "hi" | "mr" | "ne" | "sa" | "kok" | "bn" | "as" | "bpy" | "pa" | "gu" | "or"
        | "ta" | "te" | "kn" | "ml" => {
            // Indic scripts share the ISCII-derived layout, so the built-in
            // groups apply at the same *offset-relative* positions regardless of
            // script.  Groups (from the espeak `*_rules`): A = vowel letters +
            // signs, B = signs + virama, C = consonants, Y = vowels + signs.
            // Group **B** is the key — a consonant followed by group B drops its
            // inherent vowel, which makes conjuncts (`स्त`) collapse correctly.
            // The table is indexed by `(codepoint - offset) & 0x7f`, and all
            // Indic offsets are the block start, so index = position within block.
            let set = |bits: &mut [u8; 256], lo: usize, hi: usize, group: u8| {
                for i in lo..=hi { bits[i] |= 1 << group; }
            };
            set(&mut bits, 0x15, 0x39, 2); // consonants → C
            set(&mut bits, 0x58, 0x5F, 2); // nukta consonant forms → C
            set(&mut bits, 0x05, 0x14, 0); // independent vowels → A
            set(&mut bits, 0x05, 0x14, 6); //                       + Y
            set(&mut bits, 0x3E, 0x4C, 0); // dependent vowel signs → A
            set(&mut bits, 0x3E, 0x4C, 1); //                        + B
            set(&mut bits, 0x3E, 0x4C, 6); //                        + Y
            bits[0x4D] |= 1 << 1;          // virama → B (suppresses inherent vowel)
        }
        "si" => {
            // Sinhala (own block, offset 0x0D80).  Same abugida mechanism as the
            // ISCII scripts — a consonant followed by group **B** (a dependent
            // vowel sign or the virama/al-lakuna) drops its inherent vowel; the
            // si_rules key the drop on group B (`ක (B → k`).  Layout differs
            // from ISCII, so the positions are Sinhala-specific.
            let set = |bits: &mut [u8; 256], lo: usize, hi: usize, group: u8| {
                for i in lo..=hi { bits[i] |= 1 << group; }
            };
            set(&mut bits, 0x1A, 0x46, 2); // consonants ක–ෆ → C
            set(&mut bits, 0x05, 0x16, 0); // independent vowels අ–ඖ → A
            set(&mut bits, 0x4F, 0x5F, 1); // dependent vowel signs → B
            set(&mut bits, 0x4F, 0x5F, 0); //                        + A
            set(&mut bits, 0x72, 0x73, 1); // vocalic rr/ll signs → B
            bits[0x4A] |= 1 << 1;          // virama (al-lakuna) → B
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
