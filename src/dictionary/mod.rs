//! Dictionary lookup, rule-based translation, and stress assignment.
//!
//! Rust port of `dictionary.c`, `compiledict.c`, and the constant definitions
//! in `translate.h`.
//!
//! # Binary format of `<lang>_dict`
//! ```text
//! bytes  0-3 : u32le = N_HASH_DICT (1024) — integrity check
//! bytes  4-7 : u32le = rules_offset — byte offset to translation rules
//! bytes  8.. : hash buckets (N_HASH_DICT × variable-length entries)
//! bytes  rules_offset.. : translation rule groups
//! ```
//!
//! Each word entry in a hash bucket:
//! ```text
//! byte 0   : total entry length (including this byte)
//! byte 1   : (word_bytes & 0x3f) | (compressed << 6) | (no_phonemes << 7)
//! bytes 2..: word UTF-8 bytes
//! then     : phoneme string (null-terminated), absent if no_phonemes
//! then     : flag bytes until end of entry
//! bucket terminator: 0x00 byte
//! ```

// This module re-exports public types and re-declares the C constants that
// are spread across translate.h.  The constants are an internal implementation
// detail; their names follow the C naming convention verbatim.
#![allow(missing_docs)]

pub mod flags;
pub mod file;
pub mod lookup;
pub mod rules;
pub mod phonemes;
pub mod stress;
pub mod transpose;

pub use flags::{DictFlags1, DictFlags2};
pub use file::Dictionary;
pub use lookup::{hash_word, lookup, LookupCtx, LookupResult};
pub use phonemes::{encode_phonemes, decode_phonemes};
pub use transpose::{TransposeConfig, transpose_alphabet};

// ---------------------------------------------------------------------------
// Sizes / limits
// ---------------------------------------------------------------------------

pub const N_HASH_DICT:      usize = 1024;
pub const N_LETTER_GROUPS:  usize = 95;    // max 127-32
pub const N_RULE_GROUP2:    usize = 120;
pub const N_WORD_PHONEMES:  usize = 200;
pub const N_WORD_BYTES:     usize = 160;
pub const N_PHONEME_BYTES:  usize = 160;

// ---------------------------------------------------------------------------
// Rule byte codes  (values that appear inside compiled rule strings)
// ---------------------------------------------------------------------------

pub const RULE_PRE:          u8 = 1;
pub const RULE_POST:         u8 = 2;
pub const RULE_PHONEMES:     u8 = 3;
pub const RULE_PH_COMMON:    u8 = 4;
pub const RULE_CONDITION:    u8 = 5;
pub const RULE_GROUP_START:  u8 = 6;
pub const RULE_GROUP_END:    u8 = 7;
pub const RULE_PRE_ATSTART:  u8 = 8;
pub const RULE_LINENUM:      u8 = 9;
pub const RULE_STRESSED:     u8 = 10;
pub const RULE_DOUBLE:       u8 = 11;
pub const RULE_INC_SCORE:    u8 = 12;
pub const RULE_DEL_FWD:      u8 = 13;
pub const RULE_ENDING:       u8 = 14;
pub const RULE_DIGIT:        u8 = 15;
pub const RULE_NONALPHA:     u8 = 16;
pub const RULE_LETTERGP:     u8 = 17;
pub const RULE_LETTERGP2:    u8 = 18;
pub const RULE_CAPITAL:      u8 = 19;
pub const RULE_REPLACEMENTS: u8 = 20;
pub const RULE_SYLLABLE:     u8 = 21;
pub const RULE_SKIPCHARS:    u8 = 23;
pub const RULE_NO_SUFFIX:    u8 = 24;
pub const RULE_NOTVOWEL:     u8 = 25;
pub const RULE_IFVERB:       u8 = 26;
pub const RULE_DOLLAR:       u8 = 28;
pub const RULE_NOVOWELS:     u8 = 29;
pub const RULE_SPELLING:     u8 = 31;
pub const RULE_SPACE:        u8 = 32; // ascii space
pub const RULE_DEC_SCORE:    u8 = 60; // '<'

/// All rule codes ≤ this value are "control" codes, not literal match chars.
pub const RULE_LAST_RULE:    u8 = 31;

// ---------------------------------------------------------------------------
// Letter group indices  (LETTERGP_*)
// ---------------------------------------------------------------------------

pub const LETTERGP_A:      usize = 0;
pub const LETTERGP_B:      usize = 1;
pub const LETTERGP_C:      usize = 2;
pub const LETTERGP_H:      usize = 3;
pub const LETTERGP_F:      usize = 4;
pub const LETTERGP_G:      usize = 5;
pub const LETTERGP_Y:      usize = 6;
pub const LETTERGP_VOWEL2: usize = 7;

// ---------------------------------------------------------------------------
// Dollar sub-commands  (appear after RULE_DOLLAR in a rule)
// ---------------------------------------------------------------------------

pub const DOLLAR_UNPR:     u8 = 0x01;
pub const DOLLAR_NOPREFIX: u8 = 0x02;
pub const DOLLAR_LIST:     u8 = 0x03;

// ---------------------------------------------------------------------------
// Stress position rules  (LANGUAGE_OPTIONS::stress_rule)
// ---------------------------------------------------------------------------

pub const STRESSPOSN_1L:          i32 = 0;  // first vowel
pub const STRESSPOSN_1R:          i32 = 1;  // last vowel
pub const STRESSPOSN_2R:          i32 = 2;  // penultimate vowel (default)
pub const STRESSPOSN_3R:          i32 = 3;  // antepenultimate
pub const STRESSPOSN_2L:          i32 = 4;  // second vowel
pub const STRESSPOSN_SYLCOUNT:    i32 = 5;  // Russian-style
pub const STRESSPOSN_2LLH:        i32 = 6;
pub const STRESSPOSN_1RH:         i32 = 7;  // last heaviest
pub const STRESSPOSN_1RU:         i32 = 8;  // Turkish
pub const STRESSPOSN_ALL:         i32 = 9;
pub const STRESSPOSN_GREENLANDIC: i32 = 10;
pub const STRESSPOSN_1SL:         i32 = 11;
pub const STRESSPOSN_EU:          i32 = 12;

// ---------------------------------------------------------------------------
// Stress level values  (used in vowel_stress arrays)
// ---------------------------------------------------------------------------

pub const STRESS_IS_DIMINISHED:  i8 = 0;
pub const STRESS_IS_UNSTRESSED:  i8 = 1;
pub const STRESS_IS_NOT_STRESSED:i8 = 2;
pub const STRESS_IS_SECONDARY:   i8 = 3;
pub const STRESS_IS_PRIMARY:     i8 = 4;
pub const STRESS_IS_PRIORITY:    i8 = 5;

// ---------------------------------------------------------------------------
// S_* stress flags  (LANGUAGE_OPTIONS::stress_flags)
// ---------------------------------------------------------------------------

pub const S_NO_AUTO_2:        u32 = 0x001;
pub const S_2_TO_HEAVY:       u32 = 0x002;
pub const S_FINAL_NO_2:       u32 = 0x004;
pub const S_FINAL_VOWEL_UNSTRESSED: u32 = 0x008;
pub const S_1_SHORT:          u32 = 0x010;
pub const S_INITIAL_2:        u32 = 0x2000;
pub const S_2_SYL_2:          u32 = 0x1000;
pub const S_PRIORITY_STRESS:  u32 = 0x20000;
pub const S_FINAL_LONG:       u32 = 0x80000;
pub const S_FINAL_SPANISH:    u32 = 0x200;
pub const S_FIRST_PRIMARY:    u32 = 0x8000;

// ---------------------------------------------------------------------------
// Word / suffix flags  (appear in rules and dictionary entries)
// ---------------------------------------------------------------------------

pub const FLAG_SUFFIX_VOWEL: u32 = 0x0800_0000;
pub const FLAG_HYPHEN:       u32 = 0x0000_0080;
pub const FLAG_HYPHEN_AFTER: u32 = 0x0000_4000;
pub const FLAG_FIRST_UPPER:  u32 = 0x0000_0002;
pub const FLAG_ALL_UPPER:    u32 = 0x0000_0001;
pub const FLAG_HAS_DOT:      u32 = 0x0001_0000;
pub const FLAG_FIRST_WORD:   u32 = 0x0000_0200;
pub const FLAG_PREFIX_REMOVED:u32 = 0x0080_0000;
pub const FLAG_SUFFIX_REMOVED:u32 = 0x0000_2000;
pub const FLAG_UNPRON_TEST:  u32 = 0x8000_0000;
pub const FLAG_NO_TRACE:     u32 = 0x1000_0000;

pub const SUFX_E:   u32 = 0x0100;
pub const SUFX_I:   u32 = 0x0200;
pub const SUFX_P:   u32 = 0x0400;
pub const SUFX_V:   u32 = 0x0800;
pub const SUFX_D:   u32 = 0x1000;
pub const SUFX_A:   u32 = 0x40000;
pub const SUFX_S:   u32 = 0x0008; // FLAG_SUFX_S
pub const SUFX_UNPRON: u32 = 0x8000;

pub const FLAG_SUFX:   u32 = 0x04;
pub const FLAG_SUFX_S: u32 = 0x08;
pub const FLAG_SUFX_E_ADDED: u32 = 0x10;
pub const FLAG_ALLOW_TEXTMODE: u32 = 0x02;

// Dictionary entry flags (word 1)
pub const FLAG_FOUND:           u32 = 0x8000_0000;
pub const FLAG_FOUND_ATTRIBUTES:u32 = 0x4000_0000;
pub const FLAG_TEXTMODE:        u32 = 0x2000_0000;
pub const FLAG_NEEDS_DOT:       u32 = 0x0200_0000;
pub const FLAG_MAX3:            u32 = 0x0800_0000;
pub const FLAG_SKIPWORDS:       u32 = 0x0000_0080;
pub const FLAG_SPELLWORD:       u32 = 0x0000_1000;
pub const FLAG_STRESS_END:      u32 = 0x0000_0200;
pub const FLAG_STRESS_END2:     u32 = 0x0000_0400;

// Dictionary entry flags (word 2)
pub const FLAG_VERB:    u32 = 0x10;
pub const FLAG_NOUN:    u32 = 0x20;
pub const FLAG_PAST:    u32 = 0x40;
pub const FLAG_CAPITAL: u32 = 0x200;
pub const FLAG_ALLCAPS: u32 = 0x400;
pub const FLAG_ATEND:   u32 = 0x20000;
pub const FLAG_ATSTART: u32 = 0x40000;
pub const FLAG_SENTENCE:u32 = 0x2000;
pub const FLAG_ONLY:    u32 = 0x4000;
pub const FLAG_ONLY_S:  u32 = 0x8000;
pub const FLAG_STEM:    u32 = 0x10000;
pub const FLAG_NATIVE:  u32 = 0x80000;
pub const FLAG_LOOKUP_SYMBOL: u32 = 0x4000_0000;
pub const FLAG_ACCENT:  u32 = 0x800;
pub const FLAG_ALT_TRANS:  u32 = 0x8000;
pub const FLAG_ALT2_TRANS: u32 = 0x10000;
pub const BITNUM_FLAG_ALT: u32 = 14;

pub const REPLACED_E: u8 = b'E'; // 'e' replaced by silent e marker
