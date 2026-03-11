//! Word-list lookup: `HashDictionary` + `LookupDict2` / `LookupDictList`.
//!
//! C equivalents: `HashDictionary()`, `LookupDict2()`, `LookupDictList()`
//! in `dictionary.c`.

use super::file::Dictionary;
use super::flags::{DictFlags1, DictFlags2};
use super::N_WORD_BYTES;
use super::transpose::transpose_alphabet;

// ─────────────────────────────────────────────────────────────────────────────
// Hash function (must be bit-identical to C)
// ─────────────────────────────────────────────────────────────────────────────

/// Compute the 10-bit hash used to index `dict_hashtab`.
///
/// Equivalent to C's `HashDictionary()`:
/// ```c
/// while ((c = (*string++ & 0xff)) != 0) {
///     hash = hash * 8 + c;
///     hash = (hash & 0x3ff) ^ (hash >> 8);
///     chars++;
/// }
/// return (hash + chars) & 0x3ff;
/// ```
pub fn hash_word(word: &[u8]) -> usize {
    let mut hash: u32 = 0;
    let mut chars: u32 = 0;
    for &c in word {
        if c == 0 { break; }
        hash = hash.wrapping_mul(8).wrapping_add(c as u32);
        hash = (hash & 0x3ff) ^ (hash >> 8);
        chars += 1;
    }
    ((hash + chars) & 0x3ff) as usize
}

// ─────────────────────────────────────────────────────────────────────────────
// Lookup result
// ─────────────────────────────────────────────────────────────────────────────

/// Result of a single dictionary lookup.
#[derive(Clone, Debug, Default)]
pub struct LookupResult {
    /// Phoneme string (internal espeak-ng encoding, up to N_WORD_PHONEMES bytes).
    /// Empty if the entry sets flags only (FLAGS-only entry).
    pub phonemes: Vec<u8>,
    /// Flags word 0 (found, text-mode, stress-end, …).
    pub flags1: DictFlags1,
    /// Flags word 1 (verb, noun, past, …).
    pub flags2: DictFlags2,
    /// Number of additional input words consumed (for multi-word matches).
    pub skipwords: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// LookupDict2
// ─────────────────────────────────────────────────────────────────────────────

/// Per-call context for [`lookup`], mirroring the per-call state that the C
/// code derives from the surrounding `Translator` struct.
///
/// Most fields default to `false`/`0`, which gives a plain unconditional
/// lookup suitable for single-word phonemisation.
#[derive(Clone, Copy, Debug, Default)]
pub struct LookupCtx {
    /// `tr->dict_condition` — bitmask of active conditional flags
    pub dict_condition: u32,
    /// `wflags` — e.g. FLAG_FIRST_UPPER, FLAG_ALL_UPPER, FLAG_HAS_DOT
    pub word_flags: u32,
    /// What the C code calls `end_flags` when called from LookupDictList
    pub end_flags: u32,
    /// True when called from `Lookup()` (not from `LookupDictList`)
    pub lookup_symbol: bool,
    /// `tr->expect_verb`
    pub expect_verb: bool,
    /// `tr->expect_past`
    pub expect_past: bool,
    /// `tr->expect_verb_s`
    pub expect_verb_s: bool,
    /// `tr->expect_noun`
    pub expect_noun: bool,
    /// `tr->langopts.textmode` (reversed-flag mode)
    pub textmode_lang: bool,
    /// `tr->clause_terminator` — for FLAG_SENTENCE check
    pub clause_terminator: u32,
    /// True if at end of clause (for FLAG_ATEND check)
    pub at_clause_end: bool,
    /// True if first word of clause (for FLAG_ATSTART)
    pub is_first_word: bool,
}

pub const CLAUSE_TYPE_SENTENCE: u32 = 0x8000;

/// Look up a single word in the hash table.
///
/// Returns `Some(LookupResult)` if found (including "found flags only"
/// entries whose `phonemes` will be empty), `None` if not in dictionary.
///
/// Corresponds to `LookupDict2` in dictionary.c.
pub fn lookup_dict2(
    dict: &Dictionary,
    word: &[u8],  // word bytes (no null terminator expected)
    ctx: &LookupCtx,
) -> Option<LookupResult> {

    // Apply TransposeAlphabet compression if configured (all Latin-script langs).
    let word_str = std::str::from_utf8(word).unwrap_or("");
    let transposed = transpose_alphabet(word_str, &dict.transpose);
    let compressed_word = &transposed.bytes;
    let wlen = transposed.wlen; // includes bit 6 if compressed

    // The C code's TransposeAlphabet uses `memcpy(text, buf, ix)` which does NOT
    // null-terminate at position ix. So word_buf[ix..] retains the original word
    // characters. HashDictionary then hashes the compressed bytes PLUS the original
    // word's remaining characters until null.
    //
    // Example: "hello" (5 chars) compresses to 4 bytes [32,83,12,60].
    // word_buf after transpose = [32,83,12,60, 'o'=111, '\0', ...]
    // HashDictionary([32,83,12,60,111]) = 44, not 252.
    let hash = {
        let ix = compressed_word.len();
        let mut hash_buf: Vec<u8> = compressed_word.clone();
        // Append original word chars starting at position ix (until implicit null)
        if ix < word.len() {
            hash_buf.extend_from_slice(&word[ix..]);
        }
        hash_word(&hash_buf)
    };
    let bucket_start = dict.hashtab[hash];

    let data = &dict.data;
    let mut pos = bucket_start;

    loop {
        if pos >= data.len() { break; }
        let entry_len = data[pos] as usize;
        if entry_len == 0 { break; } // end of bucket

        let entry_end = pos + entry_len;
        if entry_end > data.len() { break; }

        // byte 1: word length info
        //   bits 0-5: word byte count
        //   bit  6  : compressed flag
        //   bit  7  : no_phonemes flag
        let word_info = data[pos + 1];
        let stored_len = word_info & 0x7f; // bits 0-6 must equal wlen (incl. compressed bit)
        let actual_len = (wlen & 0x3f) as usize; // byte count for memcmp

        if stored_len != wlen
            || pos + 2 + actual_len > data.len()
            || &data[pos + 2..pos + 2 + actual_len] != compressed_word.as_slice()
        {
            pos = entry_end;
            continue;
        }

        // ── Found a matching entry ──────────────────────────────────────────
        let no_phonemes = (word_info & 0x80) != 0;

        // advance past the word bytes
        let mut p = pos + 2 + actual_len;

        // phoneme string (null-terminated), absent if no_phonemes
        let phonemes: Vec<u8>;
        if no_phonemes {
            phonemes = Vec::new();
        } else {
            let ph_start = p;
            while p < entry_end && data[p] != 0 { p += 1; }
            phonemes = data[ph_start..p].to_vec();
            if p < entry_end { p += 1; } // skip null
        }

        // ── Decode flag bytes ───────────────────────────────────────────────
        let mut flags1 = DictFlags1::default();
        let mut flags2 = DictFlags2::default();
        let mut skipwords: usize = 0;
        let mut condition_failed = false;

        while p < entry_end {
            let flag = data[p];
            p += 1;

            if flag >= 100 {
                // Conditional rule
                if flag >= 132 {
                    // fail if this condition IS set
                    if ctx.dict_condition & (1 << (flag - 132)) != 0 {
                        condition_failed = true;
                    }
                } else {
                    // allow only if this condition IS set
                    if ctx.dict_condition & (1 << (flag - 100)) == 0 {
                        condition_failed = true;
                    }
                }
            } else if flag > 80 {
                // Multi-word match: flag = 81..90 means skip (flag-80) words.
                // The remaining bytes in this entry are the following word's text.
                // C code: if (strncmp(word2, p, n_chars) != 0) condition_failed = true;
                // Since LookupCtx has no word2 (standalone single-word lookup),
                // we always fail multi-word entries here.
                skipwords = (flag - 80) as usize;
                condition_failed = true;
                p = entry_end;
            } else if flag > 64 {
                // Stressed syllable: put in bits 0-3 of flags1
                flags1.0 = (flags1.0 & !0xf) | (flag & 0xf) as u32;
                // If bits 2-3 are both set → FLAG_STRESS_END
                if (flag & 0xc) == 0xc {
                    flags1.set(super::FLAG_STRESS_END);
                }
            } else if flag >= 32 {
                flags2.set(1u32 << (flag - 32));
            } else {
                flags1.set(1u32 << flag);
            }
        }

        if condition_failed {
            pos = entry_end;
            continue;
        }

        // ── Apply entry-level guards (mirrors the if/continue block in C) ──
        let end_flags = ctx.end_flags;
        let has_suffix = (end_flags & super::FLAG_SUFX) != 0;

        if !has_suffix && flags2.stem_only() {
            // FLAG_STEM: must have a suffix
            pos = entry_end;
            continue;
        }
        if (end_flags & super::SUFX_P != 0) && (flags2.only_form() || flags2.only_s_form()) {
            // $only or $onlys: don't match if prefix removed
            pos = entry_end;
            continue;
        }
        if has_suffix {
            if flags2.only_form() {
                pos = entry_end;
                continue;
            }
            if flags2.only_s_form() && (end_flags & super::FLAG_SUFX_S == 0) {
                pos = entry_end;
                continue;
            }
        }
        if flags2.is_capital() && (ctx.word_flags & super::FLAG_FIRST_UPPER == 0) {
            pos = entry_end;
            continue;
        }
        if flags2.is_allcaps() && (ctx.word_flags & super::FLAG_ALL_UPPER == 0) {
            pos = entry_end;
            continue;
        }
        if flags1.contains(super::FLAG_NEEDS_DOT) && (ctx.word_flags & super::FLAG_HAS_DOT == 0) {
            pos = entry_end;
            continue;
        }
        if flags2.contains(DictFlags2::ATEND) && !ctx.at_clause_end && !ctx.lookup_symbol {
            pos = entry_end;
            continue;
        }
        if flags2.contains(DictFlags2::ATSTART) && !ctx.is_first_word {
            pos = entry_end;
            continue;
        }
        if flags2.contains(DictFlags2::SENTENCE)
            && (ctx.clause_terminator & CLAUSE_TYPE_SENTENCE == 0)
        {
            pos = entry_end;
            continue;
        }
        if flags2.is_verb() {
            if !ctx.expect_verb && !(ctx.expect_verb_s && (end_flags & super::FLAG_SUFX_S != 0)) {
                pos = entry_end;
                continue;
            }
        }
        if flags2.is_past() && !ctx.expect_past {
            pos = entry_end;
            continue;
        }
        if flags2.is_noun() && (!ctx.expect_noun || (end_flags & super::SUFX_V != 0)) {
            pos = entry_end;
            continue;
        }
        // FLAG_ALT2_TRANS check is language-specific (lang=hu); skip for now.

        // ── Build flags1 return value ───────────────────────────────────────
        flags1.set(super::FLAG_FOUND_ATTRIBUTES);

        if !phonemes.is_empty() {
            flags1.set(super::FLAG_FOUND);
        }

        // textmode flag inversion (if langopts.textmode, the meaning of FLAG_TEXTMODE
        // in the entry is reversed).
        if ctx.textmode_lang {
            flags1.0 ^= super::FLAG_TEXTMODE;
        }

        return Some(LookupResult {
            phonemes,
            flags1,
            flags2,
            skipwords,
        });
    }

    None
}

// ─────────────────────────────────────────────────────────────────────────────
// LookupDictList (simplified)
// ─────────────────────────────────────────────────────────────────────────────

/// High-level look-up used by `Lookup()`.
///
/// Strips a trailing nul/space boundary, hashes the word, returns phonemes
/// and flags.  Does NOT handle abbreviation expansion (a.b.c.) or the
/// double-letter removal suffix logic — those are part of `TranslateWord`.
pub fn lookup(
    dict: &Dictionary,
    word: &str,
    ctx: &LookupCtx,
) -> Option<LookupResult> {
    // Extract the word bytes up to any space or nul
    let word_bytes: Vec<u8> = word.bytes()
        .take_while(|&b| b != 0 && b != b' ')
        .collect();

    if word_bytes.is_empty() || word_bytes.len() >= N_WORD_BYTES {
        return None;
    }

    lookup_dict2(dict, &word_bytes, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn en_dict() -> Option<Dictionary> {
        let dir = PathBuf::from("/usr/share/espeak-ng-data");
        if !dir.join("en_dict").exists() { return None; }
        Some(Dictionary::load("en", &dir).unwrap())
    }

    // ── hash function must match C bit-for-bit ─────────────────────────────

    #[test]
    fn hash_hello() {
        // Verify against Python-computed reference value (hash of raw "hello"):
        // python3: hash_dict("hello") = 48
        assert_eq!(hash_word(b"hello"), 48);
    }

    #[test]
    fn hash_empty() {
        assert_eq!(hash_word(b""), 0);
    }

    #[test]
    fn hash_nul_terminated() {
        // Stops at the first 0 byte
        assert_eq!(hash_word(b"hi\x00junk"), hash_word(b"hi"));
    }

    #[test]
    fn hash_a() {
        // python3: hash_dict("a") = 98
        assert_eq!(hash_word(b"a"), 98);
    }

    // ── lookup tests ───────────────────────────────────────────────────────

    #[test]
    fn lookup_the() {
        let dict = match en_dict() { Some(d) => d, None => return };
        let ctx = LookupCtx { lookup_symbol: true, ..Default::default() };
        let result = lookup(&dict, "the", &ctx);
        assert!(result.is_some(), "'the' should be in en_dict");
        let r = result.unwrap();
        // 'the' is definitely in the dictionary and has phonemes
        assert!(r.flags1.found(), "FLAG_FOUND should be set");
        assert!(!r.phonemes.is_empty(), "'the' should have phonemes");
    }

    #[test]
    fn lookup_notaword() {
        let dict = match en_dict() { Some(d) => d, None => return };
        let ctx = LookupCtx::default();
        let result = lookup(&dict, "xzqfgh", &ctx);
        assert!(result.is_none(), "non-word should not be found");
    }

    #[test]
    fn lookup_a() {
        let dict = match en_dict() { Some(d) => d, None => return };
        let ctx = LookupCtx { lookup_symbol: true, ..Default::default() };
        let result = lookup(&dict, "a", &ctx);
        assert!(result.is_some(), "'a' should be in en_dict");
    }

    #[test]
    fn lookup_and() {
        let dict = match en_dict() { Some(d) => d, None => return };
        let ctx = LookupCtx { lookup_symbol: true, ..Default::default() };
        let result = lookup(&dict, "and", &ctx);
        assert!(result.is_some(), "'and' should be in en_dict");
    }

    #[test]
    fn lookup_is() {
        let dict = match en_dict() { Some(d) => d, None => return };
        let ctx = LookupCtx { lookup_symbol: true, ..Default::default() };
        let result = lookup(&dict, "is", &ctx);
        assert!(result.is_some(), "'is' should be in en_dict");
    }
}
