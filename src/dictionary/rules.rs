//! Translation rule engine: `MatchRule` + `TranslateRules`.
//!
//! Faithful port of `MatchRule()` (dictionary.c:1475) and
//! `TranslateRules()` (dictionary.c:2084).
//!
//! # Rule structure
//! A compiled rule string has three sections:
//! ```text
//! PRE-context  (RULE_PRE)     — match backwards from the current position
//! MATCH                       — literal bytes consumed from the input
//! POST-context (RULE_POST)    — match forwards without consuming
//! RULE_PHONEMES               — output phoneme string (null-terminated)
//! ```
//! A rule string ends with `0x00`; `RULE_GROUP_END` marks the end of a group.

use super::{
    RULE_PRE, RULE_POST, RULE_PHONEMES, RULE_PH_COMMON, RULE_CONDITION,
    RULE_GROUP_END, RULE_PRE_ATSTART, RULE_LINENUM,
    RULE_LETTERGP, RULE_LETTERGP2, RULE_NOTVOWEL, RULE_DIGIT, RULE_NONALPHA,
    RULE_DOUBLE, RULE_DOLLAR, RULE_SYLLABLE, RULE_NOVOWELS, RULE_SKIPCHARS,
    RULE_INC_SCORE, RULE_DEC_SCORE, RULE_DEL_FWD, RULE_ENDING, RULE_NO_SUFFIX,
    RULE_STRESSED, RULE_CAPITAL, RULE_IFVERB, RULE_SPELLING,
    RULE_SPACE,
    DOLLAR_UNPR, DOLLAR_NOPREFIX, DOLLAR_LIST,
    BITNUM_FLAG_ALT,
    SUFX_UNPRON, SUFX_P,
    FLAG_SUFFIX_VOWEL, FLAG_UNPRON_TEST, FLAG_PREFIX_REMOVED, FLAG_SUFFIX_REMOVED,
    FLAG_HYPHEN, FLAG_HYPHEN_AFTER, FLAG_FIRST_UPPER,
    LETTERGP_VOWEL2, LETTERGP_C,
    REPLACED_E,
};
use super::file::Dictionary;
// ─────────────────────────────────────────────────────────────────────────────
// IsLetter / IsAlpha  (letter_bits based)
// ─────────────────────────────────────────────────────────────────────────────

/// Simplified letter-class check.  In the real C code this consults the
/// `Translator.letter_bits` array and `letter_bits_offset`.  Here we use a
/// portable approximation: the `letter_bits` slice (256 bytes) directly.
///
/// `letter_bits[c & 0xff]` — bit N set iff the character belongs to group N.
/// Group 0 = VOWEL (A), group 2 = C (consonant), group 7 = VOWEL2 (stressable vowel).
/// Check if Unicode codepoint `c` belongs to letter group `group`.
///
/// `letter_bits` is the per-language bitmask table (indexed by
/// `c - letter_bits_offset` for non-Latin scripts, or `c` for Latin).
/// The context's `letter_bits_offset` controls how the index is computed,
/// but since `RuleContext` only stores `letter_bits`, the caller must ensure
/// the table is pre-built with the correct offset subtracted.
///
/// For Cyrillic (letter_bits_offset=0x420): index = c - 0x420.
/// For Latin (letter_bits_offset=0): index = c.
pub fn is_letter(letter_bits: &[u8; 256], c: u32, group: usize) -> bool {
    // The letter_bits table is already indexed appropriately; the table was
    // built in Dictionary::build_letter_bits() with offset baked in.
    // We use c directly as the index — for Cyrillic, the table stores data
    // at indices 0x10..0x50 (codepoint - 0x420), and the caller already
    // passes `wc - letter_bits_offset` as the effective index via
    // `is_letter_with_offset`.
    if c > 0x7fff { return false; }
    let idx = (c as usize) & 0xff;
    (letter_bits[idx] >> group) & 1 != 0
}

/// Letter-group check that applies `letter_bits_offset` correctly.
///
/// Use this instead of `is_letter` when you have a raw Unicode codepoint
/// and the dictionary's `letter_bits_offset`.
pub fn is_letter_wc(letter_bits: &[u8; 256], wc: u32, letter_bits_offset: u32, group: usize) -> bool {
    let idx = if letter_bits_offset > 0 {
        if wc < letter_bits_offset { return false; }
        let ix = wc - letter_bits_offset;
        if ix >= 128 { return false; }
        ix as usize
    } else {
        if wc >= 256 { return false; }
        wc as usize
    };
    (letter_bits[idx] >> group) & 1 != 0
}

pub fn is_alpha(c: u32) -> bool {
    // Use unicode alphanumeric as a portable approximation.
    (c as u8).is_ascii_alphabetic() || (c >= 0xc0 && c < 0x2c0)
}

pub fn is_digit(c: u32) -> bool {
    (c >= b'0' as u32 && c <= b'9' as u32) || (c >= 0x660 && c <= 0x669) // Arabic-Indic
}

// ─────────────────────────────────────────────────────────────────────────────
// IsLetterGroup  (letter_groups based)
// ─────────────────────────────────────────────────────────────────────────────

/// Check whether the bytes at `text_pos` (in `text`) match any string in the
/// letter group `ix`.  `backwards` = true scans `text` as a pre-context
/// (reading backwards).
///
/// Returns the number of bytes matched (≥ 1) or -1 on failure.
/// Equivalent to C's `IsLetterGroup()`.
pub fn is_letter_group(
    dict: &Dictionary,
    text: &[u8],
    text_pos: usize,  // position of the character to test
    group_ix: usize,
    backwards: bool,
) -> i32 {
    let group_data = match dict.letter_group(group_ix) {
        Some(d) => d,
        None => return -1,
    };

    let mut g_pos = 0usize;
    loop {
        if g_pos >= group_data.len() { break; }
        // The C code terminates groups at RULE_GROUP_END (0x07) or null.
        // Also handle '~' meaning "any character matches" (return 0).
        let gb = group_data[g_pos];
        if gb == 0 { break; }
        if gb == super::RULE_GROUP_END { break; } // 0x07
        if gb == b'~' { return 0; }               // match any

        // Each entry in the letter group is a null-terminated UTF-8 string.
        let entry_start = g_pos;
        while g_pos < group_data.len() && group_data[g_pos] != 0
            && group_data[g_pos] != super::RULE_GROUP_END
        {
            g_pos += 1;
        }
        let entry = &group_data[entry_start..g_pos];
        if g_pos < group_data.len() { g_pos += 1; } // skip null

        let n = entry.len();
        if n == 0 { continue; }

        let matches = if backwards {
            // Match the entry at text_pos backwards (entry is forward-oriented)
            text_pos + 1 >= n
                && &text[text_pos + 1 - n..=text_pos] == entry
        } else {
            text_pos + n <= text.len()
                && &text[text_pos..text_pos + n] == entry
        };

        if matches {
            return n as i32;
        }
    }
    -1
}

// ─────────────────────────────────────────────────────────────────────────────
// MatchRecord
// ─────────────────────────────────────────────────────────────────────────────

/// Mirrors C's `MatchRecord` struct.
#[derive(Clone, Debug, Default)]
/// Internal record of the best-scoring rule match so far.
pub struct MatchRecord {
    /// Rule score (higher = better match).
    pub points:   i32,
    /// Byte offset into `dict.data` of the phoneme string, or `usize::MAX` = none.
    pub phonemes_offset: usize,
    /// Suffix/ending type if the rule fires `RULE_ENDING`.
    pub end_type: u32,
    /// Byte offset of a forward 'e' to delete (`RULE_DEL_FWD`), or `usize::MAX`.
    pub del_fwd: usize,
}

impl MatchRecord {
    const NONE: usize = usize::MAX;

    fn reset() -> Self {
        MatchRecord {
            points: 1,
            phonemes_offset: Self::NONE,
            end_type: 0,
            del_fwd: Self::NONE,
        }
    }

    fn empty_best() -> Self {
        MatchRecord {
            points: 0,
            phonemes_offset: Self::NONE,
            end_type: 0,
            del_fwd: Self::NONE,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RuleContext — state passed into MatchRule
// ─────────────────────────────────────────────────────────────────────────────

/// Per-call context for [`match_rule`], mirroring the C `Translator` fields
/// consumed by `MatchRule()`.
pub struct RuleContext<'a> {
    /// The full word buffer (space-padded on both ends), as a byte slice.
    pub word: &'a [u8],

    /// Index into `word` of the current character group being matched.
    pub word_pos: usize,

    /// Number of bytes consumed by the current character group (1 or 2).
    pub group_length: usize,

    /// Word-level flags (FLAG_FIRST_UPPER, FLAG_ALL_UPPER, FLAG_SUFFIX_REMOVED, ...)
    pub word_flags: u32,

    /// Dictionary flags from a prior lookup (dict_flags0 in C).
    pub dict_flags0: u32,

    /// Condition bitmask.
    pub dict_condition: u32,

    /// Number of vowels seen so far in this word.
    pub word_vowel_count: i32,

    /// Number of stressable vowels seen so far.
    pub word_stressed_count: i32,

    /// Letter-bits table for letter classification.
    pub letter_bits: &'a [u8; 256],

    /// Base codepoint for `letter_bits` indexing (mirrors `letter_bits_offset`).
    /// For Latin scripts = 0, for Cyrillic = 0x420, for Arabic = 0x600.
    pub letter_bits_offset: u32,

    /// Whether this is an "unpronounceable" test run.
    pub unpron_test: bool,

    /// Whether a prefix was removed.
    pub expect_verb: bool,

    pub suffix_option: u32, // tr->langopts.param[LOPT_SUFFIX]
}

// ─────────────────────────────────────────────────────────────────────────────
// MatchRule
// ─────────────────────────────────────────────────────────────────────────────

/// Match the rules in a rule group against the current word position.
/// Updates `word_pos` (advances over consumed characters) and returns the
/// best-matching `MatchRecord`.
///
/// Corresponds to `MatchRule()` in dictionary.c.
///
/// `rules` — the rule-group slice starting at the first rule (i.e. what
///   C calls `rule` at entry to `MatchRule`).
pub fn match_rule(
    dict: &Dictionary,
    ctx: &RuleContext<'_>,
    rules: &[u8],           // slice starting at first rule in group
    rules_abs_offset: usize, // abs offset of `rules[0]` into dict.data
    word_pos_out: &mut usize,
) -> MatchRecord {
    let word = ctx.word;
    let group_length = ctx.group_length;
    let word_flags = ctx.word_flags;
    let _dict_flags0 = ctx.dict_flags0; // reserved for future use (dollar-list etc.)
    let dict_condition = ctx.dict_condition;

    let unpron_ignore = ctx.unpron_test;

    let mut total_consumed = 0usize;
    let mut common_phonemes: Option<usize> = None; // abs offset into dict.data

    let mut best = MatchRecord::empty_best();

    let mut r_pos = 0usize; // position within `rules`

    // Each rule is a null-terminated string.
    // We loop through rules until RULE_GROUP_END.
    while r_pos < rules.len() && rules[r_pos] != RULE_GROUP_END {
        let mut check_atstart = false;
        let mut consumed = 0usize; // letters consumed from input
        let mut distance_left: i32  = -2;
        let mut distance_right: i32 = -6;
        let mut failed: i32 = 0;
        let _rule_start = r_pos; // start of this rule (kept for future tracing)
        let mut unpron_ignore_local: bool = unpron_ignore; // local shadow for RULE_PRE_ATSTART

        let mut match_type: u8 = 0; // 0=consume, RULE_PRE, RULE_POST
        let mut match_ = MatchRecord::reset();
        let mut letter_w: u32 = 0;
        #[allow(unused_assignments)]
        let mut last_letter_w: u32 = 0; // updated on each step, read by RULE_DOUBLE

        // working pointers (as indices into `word`)
        let mut pre_ptr  = ctx.word_pos;     // points one PAST the pre-char (backs up on read)
        let mut post_ptr = ctx.word_pos + group_length; // reads forward

        while failed == 0 {
            if r_pos >= rules.len() { break; }
            let rb = rules[r_pos];
            r_pos += 1;
            let mut add_points: i32 = 0;

            if rb <= RULE_LINENUM {
                // Control codes 0..RULE_LINENUM (0..9):
                // These are pre-match control codes handled before context matching.
                // Bytes RULE_STRESSED(10)..RULE_LAST_RULE(31) are handled in the
                // match_type context switch below (same as C code's structure).
                match rb {
                    0 => {
                        // No phoneme string for this rule — use common_phonemes
                        if let Some(cp_off) = common_phonemes {
                            // scan past condition/linenum bytes to reach phonemes
                            let mut cp = cp_off - rules_abs_offset;
                            loop {
                                if cp >= rules.len() { break; }
                                let b = rules[cp];
                                cp += 1;
                                if b == 0 || b == RULE_PHONEMES { break; }
                                if b == RULE_CONDITION { cp += 1; }
                                if b == RULE_LINENUM { cp += 2; }
                            }
                            // cp is now AFTER the RULE_PHONEMES byte (or at null/end).
                            // The phoneme string starts here.
                            match_.phonemes_offset = cp + rules_abs_offset;
                        } else {
                            match_.phonemes_offset = MatchRecord::NONE;
                        }
                        r_pos -= 1; // still pointing at 0
                        failed = 2; // matched OK
                    }
                    RULE_PRE_ATSTART => {
                        check_atstart = true;
                        unpron_ignore_local = false;
                        match_type = RULE_PRE;
                    }
                    RULE_PRE => {
                        match_type = RULE_PRE;
                        if unpron_ignore_local { failed = 1; }
                    }
                    RULE_POST => {
                        match_type = RULE_POST;
                    }
                    RULE_PHONEMES => {
                        match_.phonemes_offset = rules_abs_offset + r_pos;
                        failed = 2; // matched OK
                    }
                    RULE_PH_COMMON => {
                        common_phonemes = Some(rules_abs_offset + r_pos);
                    }
                    RULE_CONDITION => {
                        let cond_num = rules[r_pos];
                        r_pos += 1;
                        if cond_num >= 32 {
                            if dict_condition & (1 << (cond_num - 32)) != 0 { failed = 1; }
                        } else {
                            if dict_condition & (1 << cond_num) == 0 { failed = 1; }
                        }
                        if failed == 0 { match_.points += 1; }
                    }
                    RULE_LINENUM => {
                        r_pos += 2;
                    }
                    _ => {
                        // Other low codes: RULE_STRESSED, RULE_DOUBLE, etc.
                        // They appear as `rb` values ≤ RULE_LAST_RULE in the match sections
                        // but their case is handled in the main switch below.
                        // Here we just skip them (they'll be reached via `match_type`).
                    }
                }
                continue;
            }

            // ── Letter-match byte ─────────────────────────────────────────
            match match_type {
                0 => {
                    // consume from post direction
                    if post_ptr >= word.len() { failed = 1; break; }
                    let letter = word[post_ptr];
                    post_ptr += 1;

                    if letter == rb || (letter == REPLACED_E && rb == b'e') {
                        if (letter & 0xc0) != 0x80 { add_points = 21; }
                        consumed += 1;
                    } else {
                        failed = 1;
                    }
                }

                RULE_POST => {
                    // match forward in post-context
                    distance_right += 6;
                    if distance_right > 18 { distance_right = 19; }
                    last_letter_w = letter_w;

                    // C checks: if post_ptr[-1] == 0, we're past end of text
                    if post_ptr == 0
                        || post_ptr >= word.len()
                        || (post_ptr > 0 && word[post_ptr - 1] == 0)
                    {
                        failed = 1;
                        break;
                    }

                    let (wc, xbytes) = utf8_decode(word, post_ptr);
                    letter_w = wc;
                    let letter = word[post_ptr];
                    post_ptr += 1;

                    match rb {
                        RULE_LETTERGP => {
                            let lg = letter_group_no(&mut r_pos, rules);
                            if is_letter_wc(ctx.letter_bits, letter_w, ctx.letter_bits_offset, lg) {
                                let lg_pts = if lg == LETTERGP_C { 19 } else { 20 };
                                add_points = lg_pts - distance_right;
                                post_ptr += xbytes;
                            } else { failed = 1; }
                        }
                        RULE_LETTERGP2 => {
                            let lg = letter_group_no(&mut r_pos, rules);
                            let n = is_letter_group(dict, word, post_ptr - 1, lg, false);
                            if n >= 0 {
                                add_points = 20 - distance_right;
                                post_ptr += (n as usize).saturating_sub(1);
                            } else { failed = 1; }
                        }
                        RULE_NOTVOWEL => {
                            if is_letter_wc(ctx.letter_bits, letter_w, ctx.letter_bits_offset, LETTERGP_VOWEL2)
                                || (letter_w == RULE_SPACE as u32
                                    && word_flags & FLAG_SUFFIX_VOWEL != 0)
                            {
                                failed = 1;
                            } else {
                                add_points = 20 - distance_right;
                                post_ptr += xbytes;
                            }
                        }
                        RULE_DIGIT => {
                            if is_digit(letter_w) {
                                add_points = 20 - distance_right;
                                post_ptr += xbytes;
                            } else { failed = 1; }
                        }
                        RULE_NONALPHA => {
                            if !is_alpha(letter_w) {
                                add_points = 21 - distance_right;
                                post_ptr += xbytes;
                            } else { failed = 1; }
                        }
                        RULE_DOUBLE => {
                            if letter_w == last_letter_w {
                                add_points = 21 - distance_right;
                                post_ptr += xbytes;
                            } else { failed = 1; }
                        }
                        RULE_DOLLAR => {
                            let command = rules[r_pos];
                            r_pos += 1;
                            post_ptr -= 1;
                            if command == DOLLAR_UNPR {
                                match_.end_type = SUFX_UNPRON;
                            } else if command == DOLLAR_NOPREFIX {
                                if word_flags & FLAG_PREFIX_REMOVED != 0 { failed = 1; }
                                else { add_points = 1; }
                            } else if (command & 0xf0) == 0x10 {
                                // $w_alt: dict_flags must have the alt flag set
                                let flag_bit = (BITNUM_FLAG_ALT + (command & 0x0f) as u32) as usize;
                                if ctx.dict_flags0 & (1 << flag_bit) != 0 {
                                    add_points = 23;
                                } else {
                                    failed = 1;
                                }
                            } else if (command & 0xf0) == 0x20 || command == DOLLAR_LIST {
                                // DollarRule: not fully implemented; skip the condition
                                // (treat as failed to avoid false positives)
                                failed = 1;
                            }
                        }
                        b'-' => {
                            if letter == b'-'
                                || (letter == b' ' && word_flags & FLAG_HYPHEN_AFTER != 0)
                            {
                                add_points = 22 - distance_right;
                            } else { failed = 1; }
                        }
                        RULE_SYLLABLE => {
                            // count vowels to the right
                            let mut p2 = post_ptr + xbytes;
                            let mut vowel_count = 0i32;
                            let mut syllable_count = 1i32;
                            while r_pos < rules.len() && rules[r_pos] == RULE_SYLLABLE {
                                r_pos += 1;
                                syllable_count += 1;
                            }
                            let mut lw = letter_w;
                            let mut vowel_flag = false;
                            while lw != RULE_SPACE as u32 && lw != 0 {
                                if !vowel_flag && is_letter_wc(ctx.letter_bits, lw, ctx.letter_bits_offset, LETTERGP_VOWEL2) {
                                    vowel_count += 1;
                                }
                                vowel_flag = is_letter_wc(ctx.letter_bits, lw, ctx.letter_bits_offset, LETTERGP_VOWEL2);
                                let (nw, _) = utf8_decode(word, p2);
                                lw = nw;
                                p2 += 1;
                                if p2 >= word.len() { break; }
                            }
                            if syllable_count <= vowel_count {
                                add_points = 18 + syllable_count - distance_right;
                            } else { failed = 1; }
                        }
                        RULE_NOVOWELS => {
                            let mut p2 = post_ptr + xbytes;
                            let mut lw = letter_w;
                            loop {
                                if lw == RULE_SPACE as u32 || lw == 0 { break; }
                                if is_letter_wc(ctx.letter_bits, lw, ctx.letter_bits_offset, LETTERGP_VOWEL2) {
                                    failed = 1;
                                    break;
                                }
                                let (nw, _) = utf8_decode(word, p2);
                                lw = nw;
                                p2 += 1;
                                if p2 >= word.len() { break; }
                            }
                            if failed == 0 { add_points = 19 - distance_right; }
                        }
                        RULE_SKIPCHARS => {
                            // Skip forward until we find the char(s) indicated by the rule
                            let target = rules[r_pos] as u32;
                            let mut p2 = post_ptr.saturating_sub(1);
                            let mut p2_prev = p2;
                            let mut found_lw = letter_w;
                            while found_lw != target
                                && found_lw != RULE_SPACE as u32
                                && found_lw != 0
                            {
                                p2_prev = p2;
                                let (nw, _) = utf8_decode(word, p2);
                                found_lw = nw;
                                p2 += 1;
                                if p2 >= word.len() { break; }
                            }
                            if found_lw == target { post_ptr = p2_prev; }
                        }
                        RULE_INC_SCORE => {
                            post_ptr -= 1;
                            add_points = 20;
                        }
                        RULE_DEC_SCORE => {
                            post_ptr -= 1;
                            add_points = -20;
                        }
                        RULE_DEL_FWD => {
                            // find the next 'e' in the consumed section and mark it
                            let search_start = ctx.word_pos + group_length;
                            let search_end = post_ptr;
                            for k in search_start..search_end {
                                if k < word.len() && word[k] == b'e' {
                                    match_.del_fwd = rules_abs_offset + k; // approximate
                                    break;
                                }
                            }
                        }
                        RULE_ENDING => {
                            // next 3 bytes encode the end_type
                            if r_pos + 2 < rules.len() {
                                let end_type = (rules[r_pos] as u32) << 16
                                    | ((rules[r_pos + 1] & 0x7f) as u32) << 8
                                    | (rules[r_pos + 2] & 0x7f) as u32;
                                r_pos += 3;

                                if ctx.word_vowel_count == 0
                                    && (end_type & SUFX_P == 0)
                                    && (ctx.suffix_option & 1 != 0)
                                {
                                    failed = 1;
                                } else {
                                    match_.end_type = end_type;
                                }
                            }
                        }
                        RULE_NO_SUFFIX => {
                            if word_flags & FLAG_SUFFIX_REMOVED != 0 { failed = 1; }
                            else {
                                post_ptr -= 1;
                                add_points = 1;
                            }
                        }
                        RULE_SPELLING => {
                            // language-specific; just advance past
                        }
                        _ => {
                            // literal char match in post-context
                            if letter == rb {
                                if (letter & 0xc0) != 0x80 {
                                    add_points = 21 - distance_right;
                                }
                            } else { failed = 1; }
                        }
                    }
                }

                RULE_PRE => {
                    // match backward in pre-context
                    distance_left += 2;
                    if distance_left > 18 { distance_left = 19; }

                    if pre_ptr == 0 || pre_ptr > word.len() { failed = 1; break; }

                    let (cur_lw, _) = utf8_decode_backwards(word, pre_ptr);
                    last_letter_w = cur_lw; // save before we move back more

                    // Decode the character to the left of pre_ptr
                    let (lw, xbytes) = utf8_decode_backwards(word, pre_ptr);
                    letter_w = lw;
                    let letter = if pre_ptr > 0 { word[pre_ptr - 1] } else { 0 };
                    if pre_ptr > 0 { pre_ptr -= 1; }

                    match rb {
                        RULE_LETTERGP => {
                            let lg = letter_group_no(&mut r_pos, rules);
                            if is_letter_wc(ctx.letter_bits, letter_w, ctx.letter_bits_offset, lg) {
                                let lg_pts = if lg == LETTERGP_C { 19 } else { 20 };
                                add_points = lg_pts - distance_left;
                                pre_ptr = pre_ptr.saturating_sub(xbytes);
                            } else { failed = 1; }
                        }
                        RULE_LETTERGP2 => {
                            let lg = letter_group_no(&mut r_pos, rules);
                            let n = is_letter_group(dict, word, pre_ptr, lg, true);
                            if n >= 0 {
                                add_points = 20 - distance_right;
                                pre_ptr = pre_ptr.saturating_sub((n as usize).saturating_sub(1));
                            } else { failed = 1; }
                        }
                        RULE_NOTVOWEL => {
                            if !is_letter_wc(ctx.letter_bits, letter_w, ctx.letter_bits_offset, LETTERGP_VOWEL2) {
                                add_points = 20 - distance_left;
                                pre_ptr = pre_ptr.saturating_sub(xbytes);
                            } else { failed = 1; }
                        }
                        RULE_DOUBLE => {
                            if letter_w == last_letter_w {
                                add_points = 21 - distance_left;
                                pre_ptr = pre_ptr.saturating_sub(xbytes);
                            } else { failed = 1; }
                        }
                        RULE_DIGIT => {
                            if is_digit(letter_w) {
                                add_points = 21 - distance_left;
                                pre_ptr = pre_ptr.saturating_sub(xbytes);
                            } else { failed = 1; }
                        }
                        RULE_NONALPHA => {
                            if !is_alpha(letter_w) {
                                add_points = 21 - distance_right;
                                pre_ptr = pre_ptr.saturating_sub(xbytes);
                            } else { failed = 1; }
                        }
                        RULE_DOLLAR => {
                            let command = rules[r_pos];
                            r_pos += 1;
                            if pre_ptr < word.len() { pre_ptr += 1; }
                            if (command & 0xf0) == 0x10 {
                                // $w_alt
                                let flag_bit = (BITNUM_FLAG_ALT + (command & 0x0f) as u32) as usize;
                                if ctx.dict_flags0 & (1 << flag_bit) != 0 {
                                    add_points = 23;
                                } else {
                                    failed = 1;
                                }
                            } else if (command & 0xf0) == 0x20 || command == DOLLAR_LIST {
                                failed = 1;
                            }
                        }
                        RULE_SYLLABLE => {
                            let mut syllable_count = 1i32;
                            while r_pos < rules.len() && rules[r_pos] == RULE_SYLLABLE {
                                r_pos += 1;
                                syllable_count += 1;
                            }
                            if syllable_count <= ctx.word_vowel_count {
                                add_points = 18 + syllable_count - distance_left;
                            } else { failed = 1; }
                        }
                        RULE_STRESSED => {
                            if pre_ptr < word.len() { pre_ptr += 1; }
                            if ctx.word_stressed_count > 0 { add_points = 19; }
                            else { failed = 1; }
                        }
                        RULE_NOVOWELS => {
                            let mut p2 = pre_ptr;
                            let mut lw2 = letter_w;
                            loop {
                                if lw2 == RULE_SPACE as u32 { break; }
                                if is_letter_wc(ctx.letter_bits, lw2, ctx.letter_bits_offset, LETTERGP_VOWEL2) {
                                    failed = 1;
                                    break;
                                }
                                if p2 == 0 { break; }
                                let (nw, nb) = utf8_decode_backwards(word, p2);
                                lw2 = nw;
                                p2 = p2.saturating_sub(nb + 1);
                            }
                            if failed == 0 { add_points = 3; }
                        }
                        RULE_IFVERB => {
                            if pre_ptr < word.len() { pre_ptr += 1; }
                            if ctx.expect_verb { add_points = 1; }
                            else { failed = 1; }
                        }
                        RULE_CAPITAL => {
                            if pre_ptr < word.len() { pre_ptr += 1; }
                            if word_flags & FLAG_FIRST_UPPER != 0 { add_points = 1; }
                            else { failed = 1; }
                        }
                        b'.' => {
                            // dot in pre-section: match any dot before this position
                            let mut k = pre_ptr;
                            let mut found_dot = false;
                            loop {
                                if k >= word.len() || word[k] == 0 || word[k] == b' ' { break; }
                                if word[k] == b'.' { add_points = 50; found_dot = true; break; }
                                if k == 0 { break; }
                                k -= 1;
                            }
                            if !found_dot { failed = 1; }
                        }
                        b'-' => {
                            if letter == b'-'
                                || (letter == b' ' && word_flags & FLAG_HYPHEN != 0)
                            {
                                add_points = 22 - distance_right;
                            } else { failed = 1; }
                        }
                        RULE_SKIPCHARS => {
                            let target = rules[r_pos];
                            let mut p2 = (pre_ptr + 1).min(word.len().saturating_sub(1));
                            let mut p2_prev = p2;
                            loop {
                                if p2 >= word.len() { break; }
                                if word[p2] == target { break; }
                                if word[p2] == RULE_SPACE || word[p2] == 0 { break; }
                                p2_prev = p2;
                                p2 = p2.saturating_sub(1);
                            }
                            if p2 < word.len() && word[p2] == target {
                                pre_ptr = p2_prev;
                            }
                        }
                        _ => {
                            // literal char in pre-context
                            if letter == rb {
                                add_points = if letter == RULE_SPACE { 4 }
                                    else if (letter & 0xc0) != 0x80 { 21 - distance_left }
                                    else { 0 };
                            } else { failed = 1; }
                        }
                    }
                }

                _ => { failed = 1; }
            }

            if failed == 0 {
                match_.points += add_points;
            }
        }

        // ── Evaluate this rule ────────────────────────────────────────────────
        if failed == 2 && !unpron_ignore {
            let at_word_start = !check_atstart || (pre_ptr == 0 || word[pre_ptr.saturating_sub(1)] == b' ');
            if at_word_start {
                if check_atstart { match_.points += 4; }
                if match_.points >= best.points {
                    if std::env::var("ESPEAK_DEBUG_RULES").is_ok() {
                        let ph_off = match_.phonemes_offset;
                        eprintln!("  [RULE MATCH] rule_start={} abs={} points={} ph_off={:?}",
                            _rule_start, rules_abs_offset + _rule_start, match_.points, 
                            if ph_off == usize::MAX { None } else { Some(ph_off) });
                    }
                    best = match_.clone();
                    total_consumed = consumed;
                }
            }
        }

        // Skip to end of this rule (null terminator)
        while r_pos < rules.len() && rules[r_pos] != 0 {
            r_pos += 1;
        }
        if r_pos < rules.len() { r_pos += 1; } // skip null
    }

    // Advance word_pos by consumed letters + group_length
    total_consumed += group_length;
    if total_consumed == 0 { total_consumed = 1; }
    *word_pos_out = ctx.word_pos + total_consumed;

    best
}

// ─────────────────────────────────────────────────────────────────────────────
// TranslateRules result
// ─────────────────────────────────────────────────────────────────────────────

/// Output of [`translate_rules`] / [`translate_rules_phdata`].
#[derive(Clone, Debug, Default)]
pub struct RulesResult {
    /// Phoneme output (internal encoded bytes, null-terminated).
    /// When `end_type != 0`, this contains ONLY the stem phonemes.
    pub phonemes: Vec<u8>,
    /// Suffix phonemes from the `RULE_ENDING` rule that fired.
    /// Only valid when `end_type != 0`.
    pub end_phonemes: Vec<u8>,
    /// Suffix / ending type.  Non-zero if a suffix rule (`RULE_ENDING`) fired.
    pub end_type: u32,
    /// Byte offset in the original word where the matched suffix begins.
    /// Relative to `word_start`.  Only valid when `end_type != 0`.
    pub suffix_start: usize,
    /// True when the matched rule sets the spell-word flag.
    pub spellword: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// TranslateRules (simplified)
// ─────────────────────────────────────────────────────────────────────────────

/// Translate a word into phonemes using dictionary rules.
///
/// `word_buf` must be a space-padded, null-terminated byte slice in the format:
///   `' ' word_bytes ' '`
///
/// Returns `RulesResult` with the appended phoneme string.
///
/// This is a simplified port of `TranslateRules()` in dictionary.c.
/// Missing features: digit handling, language switches, unpronounceable
/// detection, alphabet switching — these will be added as needed.
pub fn translate_rules(
    dict: &Dictionary,
    word_buf: &[u8],
    word_start: usize,  // index of the first non-space char in word_buf
    word_flags: u32,
    dict_flags: u32,
    letter_bits: &[u8; 256],
    dict_condition: u32,
    word_vowel_count: &mut i32,
    word_stressed_count: &mut i32,
) -> RulesResult {
    translate_rules_phdata(dict, word_buf, word_start, word_flags, dict_flags,
        letter_bits, dict_condition, word_vowel_count, word_stressed_count, None)
}

/// Like [`translate_rules`] but also accepts optional [`PhonemeData`] for
/// IPA-level phoneme resolution (used for language-specific overrides).
///
/// [`PhonemeData`]: crate::phoneme::load::PhonemeData
pub fn translate_rules_phdata(
    dict: &Dictionary,
    word_buf: &[u8],
    word_start: usize,
    word_flags: u32,
    dict_flags: u32,
    letter_bits: &[u8; 256],
    dict_condition: u32,
    word_vowel_count: &mut i32,
    word_stressed_count: &mut i32,
    phdata: Option<&crate::phoneme::load::PhonemeData>,
) -> RulesResult {
    if dict.rules().is_empty() {
        return RulesResult::default();
    }

    let mut out_phonemes: Vec<u8> = Vec::new();
    let mut out_end_phonemes: Vec<u8> = Vec::new();
    let mut out_end_type = 0u32;
    let mut out_suffix_start = 0usize;
    let mut out_stem_ph_len = 0usize; // length of stem phonemes when suffix rule fired
    let mut spellword = false;

    // word_buf is indexed with word_start pointing to the first char.
    // We scan until we hit a space or null.
    let mut pos = word_start; // current position in word_buf

    let rules_abs_base = dict.rules_offset;

    while pos < word_buf.len() {
        let c = word_buf[pos];
        if c == 0 || c == b' ' { break; }

        let (wc, wc_bytes) = utf8_decode(word_buf, pos);

        // ── groups3 lookup (multi-byte / non-Latin scripts) ───────────────
        //
        // In C, groups3 is indexed by `(wc - letter_bits_offset)` where
        // `letter_bits_offset` is the script-specific base codepoint
        // (e.g. OFFSET_CYRILLIC = 0x420, OFFSET_ARABIC = 0x600).
        //
        // The rule group name in the binary is `[0x01, c2]` where
        //   c2 = (wc - letter_bits_offset) + 1  (1-indexed)
        // so  groups3[c2 - 1] = groups3[wc - letter_bits_offset].
        let mut found = false;
        let mut match1 = MatchRecord::empty_best();
        let mut next_pos1 = pos + 1;

        // NOTE on group_length vs wc_bytes:
        //
        // Rust's `utf8_decode` returns (codepoint, extra_bytes) where
        //   extra_bytes = total_UTF8_bytes - 1
        // C's `utf8_in` returns TOTAL bytes.  MatchRule uses group_length as
        //   post_ptr = word_pos + group_length  (must point PAST the group chars)
        //   total_consumed += group_length       (bytes advanced in input)
        // So we must pass TOTAL bytes = wc_bytes + 1 as group_length.
        let group_length_total = wc_bytes + 1; // = total UTF-8 bytes for this char

        let lbo = dict.letter_bits_offset as u64;
        if lbo > 0 && (wc as u64) >= lbo {
            let g3_idx = (wc as u64 - lbo) as usize;
            if g3_idx < 128 {
                let c2 = (g3_idx + 1) as u8; // c2 used for groups3[c2-1]
                if let Some(g3_rules) = dict.group3(c2) {
                    let g3_abs = dict.groups.groups3[g3_idx].unwrap_or(0);
                    // Use total bytes (group_length_total) to match C's group_length
                    let ctx = make_ctx(word_buf, pos, group_length_total, word_flags, dict_flags,
                        dict_condition, letter_bits, dict.letter_bits_offset, *word_vowel_count, *word_stressed_count);
                    let mut np = pos;
                    match1 = match_rule(dict, &ctx, g3_rules, g3_abs, &mut np);
                    next_pos1 = np;
                    found = match1.points > 0 || np > pos;
                }
            }
        }

        // Fall back to groups3 using raw first byte (legacy Latin-script path,
        // kept for any languages that happen to set it this way)
        if !found {
            if let Some(g3_rules) = dict.group3(c) {
                let g3_abs = dict.groups.groups3[(c.wrapping_sub(1)) as usize].unwrap_or(0);
                let ctx = make_ctx(word_buf, pos, group_length_total, word_flags, dict_flags,
                    dict_condition, letter_bits, dict.letter_bits_offset, *word_vowel_count, *word_stressed_count);
                let mut np = pos;
                match1 = match_rule(dict, &ctx, g3_rules, g3_abs, &mut np);
                next_pos1 = np;
                found = match1.points > 0 || np > pos;
            }
        }

        // Check groups2 (two-letter chains)
        let n = dict.groups.groups2_count[c as usize] as usize;
        if !found && n > 0 && pos + 1 < word_buf.len() {
            let c2 = word_buf[pos + 1];
            let key = (c as u16) | ((c2 as u16) << 8);
            let g1 = dict.groups.groups2_start[c as usize] as usize;
            let g_end = (g1 + n).min(dict.groups.groups2.len());

            for g in g1..g_end {
                if dict.groups.groups2[g].key == key {
                    found = true;
                    let entry = &dict.groups.groups2[g];
                    let g2_rules = &dict.data[entry.offset..];
                    let ctx2 = make_ctx(word_buf, pos, 2, word_flags, dict_flags,
                        dict_condition, letter_bits, dict.letter_bits_offset, *word_vowel_count, *word_stressed_count);
                    let mut np2 = pos;
                    let mut m2 = match_rule(dict, &ctx2, g2_rules, entry.offset, &mut np2);
                    if m2.points > 0 { m2.points += 35; }

                    // Also try single-letter chain
                    let ctx1 = make_ctx(word_buf, pos, 1, word_flags, dict_flags,
                        dict_condition, letter_bits, dict.letter_bits_offset, *word_vowel_count, *word_stressed_count);
                    if let Some(g1_rules) = dict.group1(c) {
                        let g1_abs = dict.groups.groups1[c as usize].unwrap_or(rules_abs_base);
                        let mut np1 = pos;
                        match1 = match_rule(dict, &ctx1, g1_rules, g1_abs, &mut np1);
                        next_pos1 = np1;
                    }

                    if m2.points >= match1.points {
                        match1 = m2;
                        next_pos1 = np2;
                    }
                    break;
                }
            }
        }

        if !found {
            // Single-letter chain
            if let Some(g1_rules) = dict.group1(c) {
                let g1_abs = dict.groups.groups1[c as usize].unwrap_or(rules_abs_base);
                let ctx = make_ctx(word_buf, pos, 1, word_flags, dict_flags,
                    dict_condition, letter_bits, dict.letter_bits_offset, *word_vowel_count, *word_stressed_count);
                let mut np = pos;
                match1 = match_rule(dict, &ctx, g1_rules, g1_abs, &mut np);
                next_pos1 = np;
            } else {
                // Default group
                if let Some(def_rules) = dict.group1(0) {
                    let def_abs = dict.groups.groups1[0].unwrap_or(rules_abs_base);
                    let ctx = make_ctx(word_buf, pos, 0, word_flags, dict_flags,
                        dict_condition, letter_bits, dict.letter_bits_offset, *word_vowel_count, *word_stressed_count);
                    let mut np = pos;
                    match1 = match_rule(dict, &ctx, def_rules, def_abs, &mut np);
                    next_pos1 = np;
                }

                if match1.points == 0 && is_alpha(wc) {
                    // Unrecognised character → spell the word
                    spellword = true;
                    break;
                }
            }
        }

        if match1.points > 0 {
            // Append phonemes from best match
            if match1.phonemes_offset != MatchRecord::NONE {
                let ph_start = match1.phonemes_offset;
                let mut ph_end = ph_start;
                while ph_end < dict.data.len() && dict.data[ph_end] != 0 {
                    ph_end += 1;
                }
                // Update vowel/stress counts from appended phonemes
                if let Some(ph) = phdata {
                    let mut unstress_mark = false;
                    for &code in &dict.data[ph_start..ph_end] {
                        if let Some(phoneme) = ph.get(code) {
                            if phoneme.typ == 1 {
                                // stress-type phoneme: check std_length
                                if phoneme.std_length < 4 {
                                    unstress_mark = true;
                                }
                            } else if phoneme.typ == 2 {
                                // vowel
                                if (phoneme.phflags & 2) == 0 && !unstress_mark {
                                    *word_stressed_count += 1;
                                }
                                unstress_mark = false;
                                *word_vowel_count += 1;
                            } else {
                                unstress_mark = false;
                            }
                        }
                    }
                }
                let before_len = out_phonemes.len();
                out_phonemes.extend_from_slice(&dict.data[ph_start..ph_end]);
                // If this rule has RULE_ENDING, record stem/suffix split
                if match1.end_type != 0 && out_end_type == 0 {
                    out_stem_ph_len = before_len;
                    let suffix_slice = &out_phonemes[before_len..];
                    out_end_phonemes.extend_from_slice(suffix_slice);
                }
            }
            if match1.end_type != 0 {
                out_end_type = match1.end_type;
                out_suffix_start = pos; // pos points to start of suffix in word_buf
            }
        }

        // Advance position.

        // Advance
        if next_pos1 <= pos {
            // Avoid infinite loop
            let (_, adv) = utf8_decode(word_buf, pos);
            pos += adv + 1;
        } else {
            pos = next_pos1;
        }
    }

    // If a suffix rule fired, truncate out_phonemes to stem only
    if out_end_type != 0 && out_stem_ph_len <= out_phonemes.len() {
        out_phonemes.truncate(out_stem_ph_len);
    }
    out_phonemes.push(0);
    out_end_phonemes.push(0);
    RulesResult {
        phonemes: out_phonemes,
        end_phonemes: out_end_phonemes,
        end_type: out_end_type,
        suffix_start: out_suffix_start,
        spellword,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn make_ctx<'a>(
    word: &'a [u8],
    pos: usize,
    group_length: usize,
    word_flags: u32,
    dict_flags: u32,
    dict_condition: u32,
    letter_bits: &'a [u8; 256],
    letter_bits_offset: u32,
    vowel_count: i32,
    stressed_count: i32,
) -> RuleContext<'a> {
    RuleContext {
        word,
        word_pos: pos,
        group_length,
        word_flags,
        dict_flags0: dict_flags,
        dict_condition,
        word_vowel_count: vowel_count,
        word_stressed_count: stressed_count,
        letter_bits,
        letter_bits_offset,
        unpron_test: word_flags & FLAG_UNPRON_TEST != 0,
        expect_verb: false,
        suffix_option: 0,
    }
}

/// Decode a UTF-8 codepoint starting at `buf[pos]`.
/// Returns `(codepoint, extra_bytes)` where `extra_bytes` = num_bytes - 1.
pub fn utf8_decode(buf: &[u8], pos: usize) -> (u32, usize) {
    if pos >= buf.len() { return (0, 0); }
    let b0 = buf[pos] as u32;
    if b0 < 0x80 {
        return (b0, 0);
    } else if b0 < 0xe0 {
        if pos + 1 < buf.len() {
            let b1 = buf[pos + 1] as u32;
            return (((b0 & 0x1f) << 6) | (b1 & 0x3f), 1);
        }
    } else if b0 < 0xf0 {
        if pos + 2 < buf.len() {
            let b1 = buf[pos + 1] as u32;
            let b2 = buf[pos + 2] as u32;
            return (((b0 & 0x0f) << 12) | ((b1 & 0x3f) << 6) | (b2 & 0x3f), 2);
        }
    } else if pos + 3 < buf.len() {
        let b1 = buf[pos + 1] as u32;
        let b2 = buf[pos + 2] as u32;
        let b3 = buf[pos + 3] as u32;
        return (((b0 & 0x07) << 18) | ((b1 & 0x3f) << 12) | ((b2 & 0x3f) << 6) | (b3 & 0x3f), 3);
    }
    (b0, 0)
}

/// Decode a UTF-8 codepoint ending AT `buf[pos]` (backwards scan).
/// Returns `(codepoint, extra_bytes)` where `extra_bytes` = num_bytes - 1.
fn utf8_decode_backwards(buf: &[u8], pos: usize) -> (u32, usize) {
    if pos == 0 { return (0, 0); }
    // Walk backwards over continuation bytes
    let mut start = pos - 1;
    while start > 0 && (buf[start] & 0xc0) == 0x80 {
        start -= 1;
    }
    let (c, _) = utf8_decode(buf, start);
    (c, pos - 1 - start)
}

/// Read the letter-group index byte from rules, advancing r_pos.
/// The byte encodes `index = byte - 'A'`.
fn letter_group_no(r_pos: &mut usize, rules: &[u8]) -> usize {
    if *r_pos >= rules.len() { return 0; }
    let b = rules[*r_pos];
    *r_pos += 1;
    // may be negative wrap-around
    (b as i16 - b'A' as i16).rem_euclid(N_LETTER_GROUPS as i16) as usize
}

use super::N_LETTER_GROUPS;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use crate::dictionary::Dictionary;

    fn en_dict() -> Option<Dictionary> {
        let dir = PathBuf::from("/usr/share/espeak-ng-data");
        if !dir.join("en_dict").exists() { return None; }
        Some(Dictionary::load("en", &dir).unwrap())
    }

    fn default_letter_bits() -> [u8; 256] {
        // Very basic: vowels (aeiou) in group 0 and group 7
        let mut bits = [0u8; 256];
        for c in b"aeiouAEIOU".iter() {
            bits[*c as usize] |= (1 << LETTERGP_VOWEL2) | 1;
        }
        for c in b"bcdfghjklmnpqrstvwxyzBCDFGHJKLMNPQRSTVWXYZ".iter() {
            bits[*c as usize] |= 1 << LETTERGP_C;
        }
        bits
    }

    #[test]
    fn utf8_decode_ascii() {
        assert_eq!(utf8_decode(b"hello", 0), (b'h' as u32, 0));
        assert_eq!(utf8_decode(b"hello", 4), (b'o' as u32, 0));
    }

    #[test]
    fn utf8_decode_two_byte() {
        // U+00E9 = 0xc3 0xa9
        let buf: &[u8] = &[0xc3, 0xa9];
        let (c, xb) = utf8_decode(buf, 0);
        assert_eq!(c, 0xe9);
        assert_eq!(xb, 1);
    }

    #[test]
    fn hash_word_in_rules() {
        // If dict loads, verify that the hash of "the" is used by lookup correctly
        let dict = match en_dict() { Some(d) => d, None => return };
        let h = super::super::lookup::hash_word(b"the");
        let bucket_start = dict.hashtab[h];
        // The bucket should start within the word-list region
        assert!(bucket_start >= 8 && bucket_start < dict.rules_offset,
            "bucket for 'the' should be in word-list region");
    }

    #[test]
    fn translate_rules_short_word() {
        let dict = match en_dict() { Some(d) => d, None => return };
        let letter_bits = default_letter_bits();
        // Wrap "a" in spaces: " a "
        let word_buf = b" a ";
        let mut vcount = 0i32;
        let mut scount = 0i32;
        let result = translate_rules(
            &dict, word_buf, 1, 0, 0, &letter_bits, 0,
            &mut vcount, &mut scount,
        );
        // We just want it not to panic; the output may be empty for 'a'
        // because 'a' is in the dictionary list, not rules — that's OK
        let _ = result;
    }

    #[test]
    fn translate_rules_hello() {
        let dict = match en_dict() { Some(d) => d, None => return };
        let letter_bits = default_letter_bits();
        // " hello "
        let word_buf = b" hello ";
        let mut vcount = 0i32;
        let mut scount = 0i32;
        let result = translate_rules(
            &dict, word_buf, 1, 0, 0, &letter_bits, 0,
            &mut vcount, &mut scount,
        );
        // Should produce some non-empty phoneme output for "hello"
        // (Even if imperfect — we're testing it doesn't crash)
        assert!(!result.spellword,
            "hello should not trigger spellword; phonemes={:?}", result.phonemes);
    }
}
