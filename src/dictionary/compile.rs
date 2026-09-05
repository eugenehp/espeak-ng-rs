//! Dictionary compiler — the inverse of the binary-`_dict` reader
//! ([`super::file`] / [`super::lookup`]).  Port of the **list-section** half of
//! `compiledict.c`: it packs word → pronunciation exception entries into the
//! hashed binary format the engine reads.
//!
//! Format written (matching [`super::file::Dictionary::from_bytes`]):
//! ```text
//! [0..4]  u32le N_HASH_DICT (1024)          — magic
//! [4..8]  u32le rules_offset
//! [8..]   1024 hash buckets, each a run of entries terminated by a 0 byte
//! [ro..]  rules section (here: a single RULE_GROUP_END → no rules)
//! ```
//! Each entry: `len(u8)`, `word_info(u8)` (bits0-5 = byte count, bit6 =
//! compressed, bit7 = no-phonemes), the (transpose-compressed) word bytes, a
//! null-terminated phoneme string (unless no-phonemes), then raw flag bytes.
//!
//! The rules-section compiler (the `<lang>_rules` DSL) is **not** ported yet, so
//! a compiled dictionary has only its word list (unknown words don't fall
//! through to letter rules).  No `_rules`/`_list` sources ship with the port, so
//! the compiler is verified by **round-trip** against the reader.

use super::lookup::hash_word;
use super::transpose::{transpose_alphabet, TransposeConfig};
use super::{N_HASH_DICT, RULE_GROUP_END, RULE_GROUP_START, RULE_PHONEMES};
use crate::phoneme::load::PhonemeData;

/// Parse `<lang>_list` DSL source text into [`ListEntry`]s — the source-reader
/// half of `compiledict.c`'s word list (§1.5).  Each non-blank, non-comment
/// line is `word<ws>phonemes[<ws>flags…]`; phoneme **mnemonics** are converted
/// to codes via the active phoneme table (`phdata`).  `//` starts a comment;
/// `$flags` after the phonemes are compiled to their flag bytes
/// ([`dollar_flag_byte`]).
pub fn parse_list_dsl(source: &str, phdata: &PhonemeData) -> Vec<ListEntry> {
    parse_list_dsl_mode(source, phdata, false)
}

/// [`parse_list_dsl`] with the language's own `langopts.textmode`.
///
/// Only Chinese sets it; everywhere else an entry marked `$text` gives
/// *replacement text* rather than phonemes, and the flag records the difference
/// from the language default (C `compile_line`).  Without this, French
/// `aujourd'hui  aujourdui $text` had "aujourdui" read as phoneme mnemonics.
pub fn parse_list_dsl_mode(
    source: &str,
    phdata: &PhonemeData,
    lang_textmode: bool,
) -> Vec<ListEntry> {
    const BITNUM_FLAG_TEXTMODE: u8 = 29;
    const BITNUM_FLAG_ONLY_S: u8 = 0x2f;
    // `$textmode` switches the *file* into replacement-text mode; the language's
    // own `langopts.textmode` only decides whether the difference is flagged.
    let mut file_textmode = false;
    let mut out = Vec::new();
    for line in source.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        // C's line parser pulls the `?N` conditions and the `$keyword` flags out
        // of the line *wherever they appear* and leaves the word and its
        // phoneme/replacement-text column behind.  A line that is nothing but
        // `$textmode` therefore defines no entry — it switches the rest of the
        // file into replacement-text mode.
        let mut flags: Vec<u8> = Vec::new();
        let mut text_not_phonemes = false;
        let mut rest: Vec<&str> = Vec::new();
        for tok in line.split_whitespace() {
            if let Some(cond) = condition_flag_byte(tok) {
                flags.push(cond);
                continue;
            }
            if tok.starts_with('$') {
                match tok {
                    "$textmode" => file_textmode = true,
                    "$phonememode" => file_textmode = false,
                    "$text" => text_not_phonemes = true,
                    _ => {
                        if let Some(byte) = dollar_flag_byte(tok) {
                            flags.push(byte);
                        }
                    }
                }
                continue;
            }
            rest.push(tok);
        }
        text_not_phonemes |= file_textmode;
        let Some((word, tail)) = rest.split_first() else {
            continue;
        };
        let (word, tail) = (*word, tail);
        // Replacement text can be several words ("😀" → "grinning face"), so keep
        // the whole column.  A *phoneme* string is one token: C's
        // `EncodePhonemes` stops at the first space, so Arabic
        // `آ  'alif mamd'u:da` is just `'alif`.
        let joined;
        let mnemonics: Option<&str> = match tail {
            [] => None,
            [one] => Some(one),
            many if text_not_phonemes => {
                joined = many.join(" ");
                // C copies the replacement text with `strncpy0(…, N_WORD_BYTES-4)`.
                let cut = joined
                    .char_indices()
                    .map(|(i, _)| i)
                    .take_while(|&i| i <= 156)
                    .last()
                    .unwrap_or(0);
                Some(&joined[..cut.min(joined.len())])
            }
            many => Some(many[0]),
        };
        if text_not_phonemes != lang_textmode {
            flags.push(BITNUM_FLAG_TEXTMODE);
        }
        let codes = match mnemonics {
            // Replacement text is stored as its own bytes, not encoded.  (C
            // additionally translates a `_…` internal name to phonemes at
            // compile time; that needs a loaded dictionary, so it is left as
            // text here.)
            Some(m) if text_not_phonemes => m.as_bytes().to_vec(),
            Some(m) => {
                let ph = crate::translate::parse_inline_phonemes(m, phdata);
                if ph.contains(&crate::phoneme::PHON_SWITCH) {
                    // Don't match on suffixes when switching languages.
                    flags.push(BITNUM_FLAG_ONLY_S);
                }
                ph
            }
            None => Vec::new(),
        };
        // C lowercases the key (except the `_…` internal names) and notes an
        // all-capitals entry with `$allcaps`.  Without it, `Louis lu:i` and
        // `I aI $u+ …` were stored under keys the lowercase lookup never
        // reaches, so both fell through to a different entry or to the rules.
        let word = if word.starts_with('_') {
            word.to_string()
        } else if let Some(hex) = word.strip_prefix("U+") {
            // `U+xxxx` is the character itself, not a four-letter word.
            match u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                Some(c) => c.to_string(),
                None => word.to_string(),
            }
        } else {
            let has_letter = word.chars().any(char::is_alphabetic);
            if has_letter && word.chars().filter(|c| c.is_alphabetic()).all(char::is_uppercase) {
                const BITNUM_FLAG_ALLCAPS: u8 = 0x2a;
                flags.push(BITNUM_FLAG_ALLCAPS);
            }
            word.to_lowercase()
        };
        out.push(ListEntry { word, phonemes: codes, flags });
    }
    out
}

/// A `$flag` name to the flag byte the `_dict` format stores — `mnem_flags` in
/// `compiledict.c`.
///
/// The reader decodes `< 32` as a flags1 bit, `32..=63` as a flags2 bit, and
/// `0x41..=0x50` as the stress nibble in flags1. Without this the conditional
/// entries all became unconditional: English `a  eI  $atend` then read as the
/// letter name everywhere instead of only at a clause end.
pub fn dollar_flag_byte(tok: &str) -> Option<u8> {
    Some(match tok {
        // Stress placement — these set bits 0-3 of flags1.
        "$1" => 0x41, "$2" => 0x42, "$3" => 0x43, "$4" => 0x44,
        "$5" => 0x45, "$6" => 0x46, "$7" => 0x47,
        "$u" => 0x48, "$u1" => 0x49, "$u2" => 0x4a, "$u3" => 0x4b,
        "$u+" => 0x4c, "$u1+" => 0x4d, "$u2+" => 0x4e, "$u3+" => 0x4f,
        // Numbered bits in dictionary word 1.
        "$pause" => 8, "$strend" => 9, "$strend2" => 10, "$unstressend" => 11,
        "$accent_before" => 12, "$abbrev" => 13, "$double" => 14,
        "$alt" | "$alt1" => 15, "$alt2" => 16, "$alt3" => 17, "$alt4" => 18,
        "$alt5" => 19, "$alt6" => 20, "$alt7" => 21,
        "$combine" => 23, "$dot" => 24, "$hasdot" => 25,
        "$max3" => 27, "$brk" => 28, "$text" => 29,
        // Dictionary word 2.
        "$verbf" => 0x20, "$verbsf" => 0x21, "$nounf" => 0x22, "$pastf" => 0x23,
        "$verb" => 0x24, "$noun" => 0x25, "$past" => 0x26, "$verbextend" => 0x28,
        "$capital" => 0x29, "$allcaps" => 0x2a, "$accent" => 0x2b,
        "$sentence" => 0x2d, "$only" => 0x2e, "$onlys" => 0x2f, "$stem" => 0x30,
        "$atend" => 0x31, "$atstart" => 0x32, "$native" => 0x33,
        // `$textmode` / `$phonememode` are compiler directives, not entry flags.
        _ => return None,
    })
}

/// Encode a leading `?N` / `?!N` condition token as its flag byte: `?N` →
/// `100 + N` (entry used only when condition N is set), `?!N` → `132 + N`
/// (entry used only when condition N is *not* set).  `None` if `tok` isn't a
/// well-formed condition prefix (so it's treated as the word).
fn condition_flag_byte(tok: &str) -> Option<u8> {
    let rest = tok.strip_prefix('?')?;
    let (base, num) = match rest.strip_prefix('!') {
        Some(n) => (132u8, n),
        None => (100u8, rest),
    };
    let cond: u8 = num.parse().ok()?;
    (cond < 32).then_some(base + cond)
}

/// One translation rule: letters to match *after* the group letter (empty =
/// match the group letter alone), optional pre/post letter context, and the
/// phoneme output.  Conditions and stress markers are not yet compiled.
#[derive(Debug, Clone, Default)]
pub struct Rule {
    /// Letters consumed after the group letter.
    pub match_letters: String,
    /// Letters required *before* the match (stored reversed on compile).
    pub pre_context: String,
    /// Letters required *after* the match (post-context).
    pub post_context: String,
    /// Phoneme codes emitted when the rule matches.
    pub phonemes: Vec<u8>,
    /// `?n` / `?!n` — the rule applies only when the voice's `dictrules`
    /// condition `n` is set (`?n`, stored as `n`) or clear (`?!n`, stored as
    /// `n + 32`).  `0` means unconditional.  English uses 225 of them.
    pub condition: u8,
}

impl Rule {
    /// A no-context rule.
    pub fn new(match_letters: &str, phonemes: &[u8]) -> Self {
        Rule { match_letters: match_letters.to_string(), phonemes: phonemes.to_vec(), ..Default::default() }
    }
}

/// A rule group, keyed by its `name` (a 1-letter name → `groups1[c]`).
#[derive(Debug, Clone)]
pub struct RuleGroup {
    pub name: String,
    pub rules: Vec<Rule>,
    /// A `.Lnn <items…>` letter-group definition instead of a letter group:
    /// the group number and its member strings.  Rules refer to these as `L01`
    /// in a context (`_) c (L01Y` — "c at word start before l or r").
    pub letter_group: Option<(u8, Vec<String>)>,
    /// For a non-Latin script, the group's index into `groups3` — the character's
    /// offset from the language's `letter_bits_offset`, plus one.  C writes such
    /// a group's header as `1, ix` instead of the name, and the reader's
    /// `groups3[]` is what its matcher consults first.
    pub group3_ix: u8,
}

/// Compile a dictionary from eSpeak NG `dictsource`-style source files into the
/// binary `<lang>_dict` format — the top-level `compiledict.c` entry point.
///
/// Reads `<dir>/<lang>_list` (word exceptions) and `<dir>/<lang>_rules`
/// (pronunciation rules), parses them with the DSL parsers, and assembles the
/// binary, resolving phoneme mnemonics via the active table (`phdata`).  Either
/// source file may be absent (treated as empty).
pub fn compile_dictionary(
    dir: &std::path::Path,
    lang: &str,
    phdata: &PhonemeData,
    transpose: &TransposeConfig,
) -> Result<Vec<u8>, String> {
    let rules_src = std::fs::read_to_string(dir.join(format!("{lang}_rules"))).unwrap_or_default();

    // Only Chinese declares `langopts.textmode` / `langopts.listx`.
    let chinese = matches!(lang, "cmn" | "yue" | "zh" | "zhy");
    // C reads the list files in this order (`compile_dictlist_file` calls), and
    // each one *prepends* to its hash chain, so a word defined more than once is
    // resolved from the file read last.
    let order: &[&str] = if chinese {
        &["roots", "list", "listx", "emoji", "extra"]
    } else {
        &["roots", "listx", "list", "emoji", "extra"]
    };
    let mut entries = Vec::new();
    for part in order {
        let src = std::fs::read_to_string(dir.join(format!("{lang}_{part}"))).unwrap_or_default();
        if src.is_empty() {
            continue;
        }
        entries.extend(parse_list_dsl_mode(&src, phdata, chinese));
    }
    let groups = parse_rules_dsl_offset(
        &rules_src,
        phdata,
        super::file::letter_bits_offset_for(lang),
    );
    let replace = parse_replace_dsl(&rules_src);
    let rules = if groups.is_empty() {
        vec![RULE_GROUP_END]
    } else {
        compile_rules(&groups)
    };
    compile_dict_full(&entries, transpose, &rules, &replace)
}

/// Parse the `.replace` section of a `<lang>_rules` source into `from` → `to`
/// character-sequence pairs (whitespace-separated columns).  The section runs
/// from a `.replace` line until the next `.`-directive.  `//` starts a comment.
/// Applied to a word before dict/rule lookup — see [`super::file::Dictionary::
/// apply_replacements`].
pub fn parse_replace_dsl(source: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut in_replace = false;
    for line in source.lines() {
        let line = line.split("//").next().unwrap_or("");
        let trimmed = line.trim();
        if trimmed == ".replace" {
            in_replace = true;
            continue;
        }
        if trimmed.starts_with('.') {
            in_replace = false;
            continue;
        }
        if !in_replace || trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        if let (Some(from), Some(to)) = (parts.next(), parts.next()) {
            pairs.push((from.to_string(), to.to_string()));
        }
    }
    pairs
}

/// Parse `<lang>_rules` DSL source text into [`RuleGroup`]s — the source-reader
/// half of `compiledict.c`'s rules (§1.5).  Handles `.group <name>` directives
/// and rule lines laid out as whitespace-separated columns `[pre)] match
/// [(post]  <phonemes>` (the group letter is stripped from the match, since it's
/// pre-consumed by the group).  Pre/post contexts use the `_` word-boundary and
/// `A`–`Z` letter-group markers ([`encode_context`]).  `//` starts a comment.
///
/// Not yet handled: other directives (`.L…`, `.replace`), rule conditions, and
/// `$`-flags — those lines are skipped.
pub fn parse_rules_dsl(source: &str, phdata: &PhonemeData) -> Vec<RuleGroup> {
    parse_rules_dsl_offset(source, phdata, 0)
}

/// [`parse_rules_dsl`] with the language's `letter_bits_offset`, which decides
/// whether a group is written by name or by `groups3` index.
pub fn parse_rules_dsl_offset(
    source: &str,
    phdata: &PhonemeData,
    letter_bits_offset: u32,
) -> Vec<RuleGroup> {
    let mut groups: Vec<RuleGroup> = Vec::new();
    for line in source.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix(".group") {
            let name = name.trim();
            // Non-Latin scripts key their groups on a codepoint offset.
            let group3_ix = match (letter_bits_offset > 0, name.chars().next()) {
                (true, Some(c)) => match (c as u32).checked_sub(letter_bits_offset) {
                    Some(ix) if ix < 128 => ix as u8 + 1,
                    _ => 0,
                },
                _ => 0,
            };
            // `.group 0xNNNN` names the group by character code.
            let name = match name.strip_prefix("0x").and_then(|h| u32::from_str_radix(h, 16).ok()) {
                Some(code) if code > 0x100 => {
                    String::from_utf8_lossy(&[(code >> 8) as u8, code as u8]).into_owned()
                }
                Some(code) => String::from_utf8_lossy(&[code as u8]).into_owned(),
                None if group3_ix != 0 => name.to_string(),
                None => {
                    // A group name is at most two *bytes*; Danish's `.group øs`
                    // and `.group ør` are both the two-byte `ø`, and C merges
                    // them with the plain `ø` group.  Without the truncation the
                    // reader saw three colliding three-byte groups and Danish
                    // words containing `ø` or `æ` stopped translating.
                    match name.len() > 2 {
                        // Keep the two-byte prefix when it is itself valid text
                        // (one two-byte letter, or two ASCII ones); otherwise
                        // fall back to the first character.
                        true => std::str::from_utf8(&name.as_bytes()[..2])
                            .map(str::to_string)
                            .unwrap_or_else(|_| name.chars().next().into_iter().collect()),
                        false => name.to_string(),
                    }
                }
            };
            groups.push(RuleGroup {
                name,
                rules: Vec::new(),
                letter_group: None,
                group3_ix,
            });
            continue;
        }
        // `.Lnn <items…>` defines a letter group that rule contexts refer to as
        // `Lnn`.  Ten of them are used by the shipped English rules, and a rule
        // whose group is undefined never matches — `_) c (L01Y` is how `cl`/`cr`
        // keep a hard `c`.
        if let Some(rest) = line.strip_prefix(".L") {
            let mut it = rest.split_whitespace();
            if let Some(num) = it.next() {
                let (digits, first) = num.split_at(num.len().min(2));
                if let Ok(n) = digits.parse::<u8>() {
                    let mut items: Vec<String> = Vec::new();
                    if !first.is_empty() {
                        items.push(first.replace('_', " "));
                    }
                    items.extend(it.map(|w| w.replace('_', " ")));
                    if n > 0 && (n as usize) < 26 && !items.is_empty() {
                        groups.push(RuleGroup {
                            name: String::new(),
                            rules: Vec::new(),
                            letter_group: Some((n, items)),
                            group3_ix: 0,
                        });
                    }
                }
            }
            continue;
        }
        // Skip other directives.
        if line.starts_with('.') {
            continue;
        }
        let Some(group) = groups.last_mut() else { continue };
        // A rule line is whitespace-separated columns: `[pre)] match [(post]
        // <phonemes>`.  espeak lays the context and match parts out in separate
        // columns for readability, so the **last** token is the phoneme string
        // and everything before it (joined, spaces removed) is the letter/context
        // spec — reading only the first two tokens grabbed the match letter as
        // the phonemes (`A) r (A  R` → wrongly used `r`, not `R`).
        let mut parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        // A leading `?n` / `?!n` is the rule's voice condition, not part of the
        // letter spec.
        let mut condition = 0u8;
        if let Some(rest) = parts[0].strip_prefix('?') {
            let (neg, digits) = match rest.strip_prefix('!') {
                Some(d) => (32u8, d),
                None => (0, rest),
            };
            if let Ok(n) = digits.parse::<u8>() {
                if n > 0 && (n as u32 + neg as u32) < 255 {
                    condition = n + neg;
                }
            }
            parts.remove(0);
            if parts.is_empty() {
                continue;
            }
        }
        // Split the columns into the letter spec and the phoneme string.  A
        // token belongs to the spec if it is the first one, if it opens a
        // post-context, or if the spec so far ended with the pre-context's `)`.
        // Everything after is phonemes — which may be **empty**: `c (q` means
        // "c is silent before q", and taking the last token as the phonemes
        // turned that into "c is pronounced /q/".
        let mut split = 1;
        while split < parts.len() {
            let t = parts[split];
            let spec_so_far: String = parts[..split].concat();
            if t.starts_with('(') || spec_so_far.ends_with(')') {
                split += 1;
            } else {
                break;
            }
        }
        let letter_spec: String = parts[..split].concat();
        let mnemonics: String = parts[split..].concat();
        let mnemonics = mnemonics.as_str();
        // `[pre)] match [(post]` — `)` ends the pre-context, `(` starts the post.
        let (pre, rest) = letter_spec.split_once(')').unwrap_or(("", letter_spec.as_str()));
        let (matched, post) = rest.split_once('(').unwrap_or((rest, ""));
        // The group letter is pre-consumed; the stored match is what follows it.
        let match_letters = matched.strip_prefix(group.name.as_str()).unwrap_or(matched).to_string();
        let phonemes = crate::translate::parse_inline_phonemes(mnemonics, phdata);
        group.rules.push(Rule {
            match_letters,
            pre_context: pre.to_string(),
            post_context: post.to_string(),
            phonemes,
            condition,
        });
    }
    groups
}

/// Encode a context string into rule bytes (`compiledict.c` letter handling):
///
/// * `_` → `RULE_SPACE` — the word-boundary marker (the reader rewards it when it
///   hits the space bracketing the word), so `_)a` = "a at word start".
/// * uppercase `A`–`Z` → a letter-group reference `RULE_LETTERGP <letter>`; the
///   reader computes `group = letter - 'A'` (e.g. `A` = vowels, `C` = consonants
///   in the English `letter_bits`), matching any member of that group.
/// * any other char → a literal match on that byte.
///
/// `reverse` (for pre-context, which is matched backward through the word) stores
/// the *tokens* nearest-match-first while keeping each multi-byte token's internal
/// order intact — the reader still reads `RULE_LETTERGP` then its group byte
/// forward through the rule.
/// Encode one side of a rule's letter context — `compile_rule`'s per-character
/// switch in `compiledict.c`.
///
/// Most characters stand for themselves, but the uppercase letters and a handful
/// of punctuation marks are *commands*, and only seven of the uppercase letters
/// are letter groups: `A B C F G H` and `Y` (which is `LETTERGP_Y`, reached in C
/// by first rewriting `Y` to `I`).  `K` is "not a vowel", `D` a digit, `V`
/// "if verb", `Z` non-alpha, and so on — encoding those as letter groups, as
/// this used to, silently disabled every rule that used one.  English "world"
/// depends on `w) or (K` → `3:`.
///
/// A two-byte command in a *pre*-context stores its operand first, because the
/// reader walks a pre-context backwards.
fn encode_context(s: &str, reverse: bool) -> Vec<u8> {
    use super::{
        RULE_CAPITAL, RULE_DEC_SCORE, RULE_DEL_FWD, RULE_DIGIT, RULE_DOLLAR, RULE_DOUBLE,
        RULE_ENDING, RULE_IFVERB, RULE_INC_SCORE, RULE_LETTERGP, RULE_LETTERGP2, RULE_NONALPHA,
        RULE_NOTVOWEL, RULE_NOVOWELS, RULE_NO_SUFFIX, RULE_SKIPCHARS, RULE_SPACE, RULE_SPELLING,
        RULE_STRESSED, RULE_SYLLABLE, SUFX_A, SUFX_D, SUFX_E, SUFX_I, SUFX_M, SUFX_P, SUFX_V,
    };
    // C's `lettergp_letters[]`, indexed by `letter - 'A'`.  Entries 3 and 4
    // (`D`, `E`) are unused — `D` is the digit command and `E` is not a group.
    const LETTERGP: [u8; 9] = [0, 1, 2, 0, 0, 4, 5, 3, 6];

    let bytes = s.as_bytes();
    let mut tokens: Vec<Vec<u8>> = Vec::with_capacity(bytes.len());
    // A two-byte command: operand first in a pre-context, command first after.
    let pair = |cmd: u8, operand: u8| -> Vec<u8> {
        if reverse { vec![operand, cmd] } else { vec![cmd, operand] }
    };
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        i += 1;
        let token = match c {
            b'_' => vec![RULE_SPACE],
            b'A' | b'B' | b'C' | b'F' | b'G' | b'H' => {
                pair(RULE_LETTERGP, LETTERGP[(c - b'A') as usize] + b'A')
            }
            // `Y` is rewritten to `I` before the group lookup, giving LETTERGP_Y.
            b'Y' => pair(RULE_LETTERGP, LETTERGP[8] + b'A'),
            b'D' => vec![RULE_DIGIT],
            b'K' => vec![RULE_NOTVOWEL],
            b'N' => vec![RULE_NO_SUFFIX],
            b'V' => vec![RULE_IFVERB],
            b'Z' => vec![RULE_NONALPHA],
            b'+' => vec![RULE_INC_SCORE],
            b'<' => vec![RULE_DEC_SCORE],
            b'@' => vec![RULE_SYLLABLE],
            b'&' => vec![RULE_STRESSED],
            b'%' => vec![RULE_DOUBLE],
            b'#' => vec![RULE_DEL_FWD],
            b'!' => vec![RULE_CAPITAL],
            b'W' => vec![RULE_SPELLING],
            b'X' => vec![RULE_NOVOWELS],
            b'J' => vec![RULE_SKIPCHARS],
            // `T` is `$w_alt1` spelled short; C emits it command-first either way.
            b'T' => vec![RULE_DOLLAR, 0x11],
            b'L' if i + 1 < bytes.len()
                && bytes[i].is_ascii_digit()
                && bytes[i + 1].is_ascii_digit() =>
            {
                let n = (bytes[i] - b'0') * 10 + (bytes[i + 1] - b'0');
                i += 2;
                pair(RULE_LETTERGP2, b'A' + n)
            }
            b'$' => {
                // `$unpr`, `$noprefix`, `$list`, `$w_altN`, `$p_altN`.  Longer
                // names first, as C's table is ordered.
                const NAMES: &[(&str, u8)] = &[
                    ("unpr", 0x01), ("noprefix", 0x02), ("list", 0x03),
                    ("w_alt1", 0x11), ("w_alt2", 0x12), ("w_alt3", 0x13),
                    ("w_alt4", 0x14), ("w_alt5", 0x15), ("w_alt6", 0x16),
                    ("w_alt", 0x11),
                    ("p_alt1", 0x21), ("p_alt2", 0x22), ("p_alt3", 0x23),
                    ("p_alt4", 0x24), ("p_alt5", 0x25), ("p_alt6", 0x26),
                    ("p_alt", 0x21),
                ];
                let rest = &s[i..];
                match NAMES.iter().find(|(n, _)| rest.starts_with(n)) {
                    Some((n, v)) => {
                        i += n.len();
                        pair(RULE_DOLLAR, *v)
                    }
                    // Unrecognised: C reports an error and emits value 0; keep
                    // the rule inert rather than matching everything.
                    None => pair(RULE_DOLLAR, 0),
                }
            }
            b'P' | b'S' => {
                // `Snn<letters>` — a suffix (or, for `P`, prefix) removal rule.
                // The letters are flag names and the digits the number of
                // characters to remove; the three bytes after RULE_ENDING are
                // the flags' top two bytes and the count with bit 7 set.
                // C seeds this with `0x808000` "to ensure non-zero bytes": the
                // two flag bytes are written into a NUL-terminated record, so
                // neither may be zero.
                let mut sxflags: u32 = 0x0080_8000 | if c == b'P' { SUFX_P } else { 0 };
                let mut value: u32 = 0;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    match bytes[i] {
                        b'e' => sxflags |= SUFX_E,
                        b'i' => sxflags |= SUFX_I,
                        b'p' => sxflags |= SUFX_P,
                        b'v' => sxflags |= SUFX_V,
                        b'd' => sxflags |= SUFX_D,
                        b'f' => sxflags |= 0x2000,  // SUFX_F
                        b'q' => sxflags |= 0x4000,  // SUFX_Q
                        b't' => sxflags |= 0x10000, // SUFX_T
                        b'b' => sxflags |= 0x20000, // SUFX_B
                        b'a' => sxflags |= SUFX_A,
                        b'm' => sxflags |= SUFX_M,
                        d if d.is_ascii_digit() => value = value * 10 + (d - b'0') as u32,
                        _ => {}
                    }
                    i += 1;
                }
                vec![
                    RULE_ENDING,
                    (sxflags >> 16) as u8,
                    (sxflags >> 8) as u8,
                    (value | 0x80) as u8,
                ]
            }
            other => vec![other],
        };
        tokens.push(token);
    }
    let mut out: Vec<u8> = tokens.into_iter().flatten().collect();
    if reverse {
        // C reverses the finished pre-context *byte by byte*
        // (`for (ix = strlen(rule_pre)-1; ix >= start; ix--)`), which is why
        // `copy_rule_string` writes a two-byte command operand-first there: the
        // reversal puts the command back in front for the reader.
        out.reverse();
    }
    out
}

/// Compile rule groups into the binary rules-section format — the inverse of
/// `build_groups` / `match_rule`, and the rules-DSL half of `compiledict.c`.
/// Supports pre/post letter context: the `_` word-boundary marker and `A`–`Z`
/// letter-group references (see [`encode_context`]); rule conditions/stress and
/// the `@`/`&` markers are not yet emitted.
///
/// Layout: for each group `RULE_GROUP_START`, the name bytes, `0`, then each
/// rule (`<match letters> RULE_PHONEMES <phonemes> 0`), then `RULE_GROUP_END`.
pub fn compile_rules(groups: &[RuleGroup]) -> Vec<u8> {
    let mut out = Vec::new();
    // Letter-group definitions come first, so a rule referring to one is
    // resolvable when the reader walks the section.
    for g in groups {
        let Some((num, items)) = &g.letter_group else { continue };
        out.push(RULE_GROUP_START);
        out.push(super::RULE_LETTERGP2);
        out.push(b'A' + num);
        // C writes the items longest-first so the reader's greedy match finds
        // the longest member.
        let max = items.iter().map(|i| i.len()).max().unwrap_or(0);
        for len in (1..=max).rev() {
            for item in items.iter().filter(|i| i.len() == len) {
                out.extend_from_slice(item.as_bytes());
                out.push(0);
            }
        }
        out.push(RULE_GROUP_END);
    }
    // C sorts the groups — long names before short ones, then alphabetically —
    // and merges consecutive groups with the same name under one header.  The
    // order is *load-bearing*: the reader indexes the multi-byte groups by their
    // first byte and expects every group sharing that byte to be contiguous, so
    // emitting them in source order left Czech `ý` and Polish `ę` unreachable
    // and their words truncated (`dobrý` → `d'obR`).
    let mut order: Vec<usize> = (0..groups.len())
        .filter(|&i| groups[i].letter_group.is_none())
        .collect();
    order.sort_by(|&a, &b| {
        let (na, nb) = (groups[a].name.as_bytes(), groups[b].name.as_bytes());
        nb.len().cmp(&na.len()).then(na.cmp(nb)).then(a.cmp(&b))
    });
    let mut prev_name: Option<&str> = None;
    for (n, &gi) in order.iter().enumerate() {
        let g = &groups[gi];
        if prev_name != Some(g.name.as_str()) {
            if n > 0 {
                out.push(RULE_GROUP_END);
            }
            out.push(RULE_GROUP_START);
            if g.group3_ix != 0 {
                out.push(1);
                out.push(g.group3_ix);
            } else {
                out.extend_from_slice(g.name.as_bytes());
            }
            out.push(0); // group-name terminator
            prev_name = Some(g.name.as_str());
        }
        // Build each rule's "tail" — everything the reader walks before it
        // reaches the phonemes: the match letters (the group's own name already
        // removed), then the condition, then the contexts.  This is C's
        // `compile_rule` order.
        let mut recs: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(g.rules.len());
        for r in &g.rules {
            let mut tail = r.match_letters.as_bytes().to_vec();
            if r.condition != 0 {
                tail.push(super::RULE_CONDITION);
                tail.push(r.condition);
            }
            if !r.pre_context.is_empty() {
                // A pre-context that starts at the word boundary is written as
                // `RULE_PRE_ATSTART` with the `_` dropped — that marker also
                // matches the true start of a word, which a literal space does
                // not.
                let (marker, body) = match r.pre_context.strip_prefix('_') {
                    Some(rest) => (super::RULE_PRE_ATSTART, rest),
                    None => (super::RULE_PRE, r.pre_context.as_str()),
                };
                tail.push(marker);
                tail.extend(encode_context(body, true));
            }
            if !r.post_context.is_empty() {
                tail.push(super::RULE_POST);
                tail.extend(encode_context(&r.post_context, false));
            }
            recs.push((r.phonemes.clone(), tail));
        }
        // C sorts a group's rules by their phoneme string (then by the rest of
        // the record) so that rules sharing a pronunciation end up adjacent and
        // can share one copy of it.  The order also decides which of two
        // equally-scoring rules wins, so it is not merely a size optimisation.
        recs.sort();
        let mut common: Option<&[u8]> = None;
        for i in 0..recs.len() {
            let (ph, tail) = &recs[i];
            if !ph.is_empty() && common == Some(ph.as_slice()) {
                // Same phonemes as the rule that declared `RULE_PH_COMMON`.
                out.extend_from_slice(tail);
                out.push(0);
                continue;
            }
            // C guards the shared-phoneme run with `common[0] != 0`, so a rule
            // with *no* phonemes never becomes the shared one.
            if !ph.is_empty() && recs.get(i + 1).is_some_and(|(next, _)| next == ph) {
                out.push(super::RULE_PH_COMMON);
                common = Some(ph.as_slice());
            }
            out.extend_from_slice(tail);
            out.push(RULE_PHONEMES);
            out.extend_from_slice(ph);
            out.push(0); // rule terminator (rules are null-terminated strings)
        }
    }
    if !order.is_empty() {
        out.push(RULE_GROUP_END);
    }
    out.push(0); // end of the rules section
    out
}

/// A word-list entry to compile: a headword, its pronunciation (phoneme codes;
/// empty for a flags-only entry), and raw flag bytes (encoded as the reader
/// decodes them — `<32` → flags1 bit, `32..=63` → flags2 bit, `65..=80` → stress).
#[derive(Debug, Clone)]
pub struct ListEntry {
    pub word: String,
    pub phonemes: Vec<u8>,
    pub flags: Vec<u8>,
}

impl ListEntry {
    /// A plain word → phonemes entry (no flags).
    pub fn new(word: &str, phonemes: &[u8]) -> Self {
        ListEntry { word: word.to_string(), phonemes: phonemes.to_vec(), flags: Vec::new() }
    }
}

/// Compile word-list `entries` into a binary `_dict` buffer (empty rules
/// section).  `transpose` must match the target language's alphabet compression
/// (`TransposeConfig::LATIN` for Latin-script languages).
///
/// Returns `Err` if a single entry would exceed 255 bytes (the format's
/// one-byte length field).
pub fn compile_dict(entries: &[ListEntry], transpose: &TransposeConfig) -> Result<Vec<u8>, String> {
    // Empty rules section: a single RULE_GROUP_END.
    compile_dict_with_rules(entries, transpose, &[RULE_GROUP_END])
}

/// Like [`compile_dict`], but with a caller-supplied rules section (see
/// [`compile_rules`]) instead of an empty one.
pub fn compile_dict_with_rules(
    entries: &[ListEntry],
    transpose: &TransposeConfig,
    rules: &[u8],
) -> Result<Vec<u8>, String> {
    compile_dict_full(entries, transpose, rules, &[])
}

/// Encode a rules-file `.replace` section (`from` → `to` pairs) as a
/// `RULE_REPLACEMENTS` group, placed so that the reader's alignment calculation
/// (`(rules_offset + relpos(REPLACEMENTS) + 4) & ~3`, with `REPLACEMENTS` at
/// rules-relative offset 1) lands on the pair table.  `rules_offset` is the
/// absolute offset of the rules section (where this group is emitted first).
fn compile_replace_section(pairs: &[(String, String)], rules_offset: usize) -> Vec<u8> {
    let mut out = vec![RULE_GROUP_START, super::RULE_REPLACEMENTS];
    let aligned = ((rules_offset + 1 + 4) & !3) - rules_offset; // ≥ 2
    while out.len() < aligned {
        out.push(0);
    }
    for (from, to) in pairs {
        out.extend_from_slice(from.as_bytes());
        out.push(0);
        out.extend_from_slice(to.as_bytes());
        out.push(0);
    }
    out.extend_from_slice(&[0, 0, 0, 0]); // 4-null end-of-table (is_str_totally_null)
    out.push(RULE_GROUP_END);
    out
}

/// [`compile_dict_with_rules`] plus a `.replace` table (emitted first in the
/// rules section, correctly aligned).  Empty `replace` reproduces the plain
/// behaviour.
pub fn compile_dict_full(
    entries: &[ListEntry],
    transpose: &TransposeConfig,
    rules: &[u8],
    replace: &[(String, String)],
) -> Result<Vec<u8>, String> {
    let mut buckets: Vec<Vec<u8>> = vec![Vec::new(); N_HASH_DICT];

    // C builds each hash chain by *prepending* (`compile_dictlist_file`) and
    // then writes it head-first, so the entries for a word come out in reverse
    // source order — and `Lookup` takes the first acceptable one.  English
    // depends on it: `a $nounf` precedes `a eI $atend` in en_list, and reading
    // them in source order made the flags-only entry win, so "a" fell through to
    // the letter rules (`'a`) instead of reading `'eI`.
    for e in entries.iter().rev() {
        let t = transpose_alphabet(&e.word, transpose);
        let compressed = &t.bytes;
        let wlen = t.wlen; // low 6 bits = byte count, bit 6 = compressed flag

        // Hash the compressed bytes plus the word's uncompressed tail, exactly
        // as the reader does (see `lookup_dict2`).
        let ix = compressed.len();
        let mut hash_buf = compressed.clone();
        let wb = e.word.as_bytes();
        if ix < wb.len() {
            hash_buf.extend_from_slice(&wb[ix..]);
        }
        let hash = hash_word(&hash_buf);

        let no_phonemes = e.phonemes.is_empty();
        let word_info = (wlen & 0x7f) | if no_phonemes { 0x80 } else { 0 };

        let mut entry = vec![0u8, word_info]; // [0] = length (filled below), [1] = word_info
        entry.extend_from_slice(compressed);
        if !no_phonemes {
            entry.extend_from_slice(&e.phonemes);
            entry.push(0); // null-terminate the phoneme string
        }
        entry.extend_from_slice(&e.flags);

        if entry.len() > u8::MAX as usize {
            // C reports the overflow and keeps the entry *without* its phonemes
            // (`dict_line[1] |= 0x80`), so the word still carries its flags and
            // falls through to the rules.  A few emoji names are this long.
            entry.truncate(2 + compressed.len());
            entry[1] = (wlen & 0x7f) | 0x80;
            entry.extend_from_slice(&e.flags);
            if entry.len() > u8::MAX as usize {
                return Err(format!(
                    "dict entry for {:?} too long ({} bytes)",
                    e.word,
                    entry.len()
                ));
            }
        }
        entry[0] = entry.len() as u8;
        buckets[hash].extend_from_slice(&entry);
    }

    let mut out = Vec::new();
    out.extend_from_slice(&(N_HASH_DICT as u32).to_le_bytes()); // magic
    out.extend_from_slice(&[0u8; 4]); // rules_offset placeholder
    for bucket in &buckets {
        out.extend_from_slice(bucket);
        out.push(0); // bucket terminator (entry_len == 0)
    }
    let rules_offset = out.len();
    // The `.replace` table (if any) is the first thing in the rules section, so
    // the header's rules_offset points at it and the reader parses it first.
    if !replace.is_empty() {
        out.extend(compile_replace_section(replace, rules_offset));
    }
    out.extend_from_slice(rules); // caller-supplied rules section
    out[4..8].copy_from_slice(&(rules_offset as u32).to_le_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::file::Dictionary;
    use crate::dictionary::lookup::{lookup, LookupCtx};

    #[test]
    fn round_trip_list_entries() {
        let entries = vec![
            ListEntry::new("hello", &[72, 108, 60]),
            ListEntry::new("world", &[50, 51, 52, 53]),
            ListEntry::new("a", &[147]),
            ListEntry::new("cat", &[75, 97, 116]),
        ];
        let bytes = compile_dict(&entries, &TransposeConfig::LATIN).expect("compile");
        let dict = Dictionary::from_bytes("en", bytes).expect("valid dict");

        let ctx = LookupCtx { lookup_symbol: true, ..Default::default() };
        for e in &entries {
            let r = lookup(&dict, &e.word, &ctx)
                .unwrap_or_else(|| panic!("compiled entry {:?} not found", e.word));
            assert!(r.flags1.found(), "{:?} should be FLAG_FOUND", e.word);
            assert_eq!(r.phonemes, e.phonemes, "phonemes for {:?}", e.word);
        }
        // A word absent from the list is not found (no rules section).
        assert!(lookup(&dict, "zzqxyw", &ctx).is_none());
    }

    #[test]
    fn list_dsl_parse_compile_read() {
        // Needs the en phoneme table to resolve mnemonics → codes.
        let dir = std::path::PathBuf::from("espeak-ng-data");
        if !dir.join("phontab").exists() {
            eprintln!("[SKIP] no local phoneme data");
            return;
        }
        let mut phdata = PhonemeData::load(&dir).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");

        let source = "\
// a small word list
hello   h@l'oU
world   w'3:ld
";
        let entries = parse_list_dsl(source, &phdata);
        assert_eq!(entries.len(), 2, "comment + 2 words");
        assert_eq!(entries[0].word, "hello");
        assert!(!entries[0].phonemes.is_empty(), "mnemonics resolved to codes");

        // End-to-end: DSL source → entries → compile → read back → lookup.
        let bytes = compile_dict(&entries, &TransposeConfig::LATIN).expect("compile");
        let dict = Dictionary::from_bytes("en", bytes).expect("valid dict");
        let r = lookup(&dict, "hello", &LookupCtx::default()).expect("'hello' found");
        assert_eq!(r.phonemes, entries[0].phonemes, "round-trip phonemes");
    }

    #[test]
    fn rules_dsl_parse_and_compile() {
        let dir = std::path::PathBuf::from("espeak-ng-data");
        if !dir.join("phontab").exists() {
            eprintln!("[SKIP] no local phoneme data");
            return;
        }
        let mut phdata = PhonemeData::load(&dir).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");

        let source = "\
// two groups with simple rules
.group a
\ta\t@
\tat\teI
.group b
\tb\tb
";
        let groups = parse_rules_dsl(source, &phdata);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "a");
        assert_eq!(groups[0].rules.len(), 2);
        // The group letter is stripped from the match.
        assert_eq!(groups[0].rules[0].match_letters, ""); // "a" − group "a"
        assert_eq!(groups[0].rules[1].match_letters, "t"); // "at" − group "a"
        assert!(!groups[0].rules[0].phonemes.is_empty(), "mnemonics resolved");

        // Compile → the reader registers both groups.
        let rules = compile_rules(&groups);
        let bytes = compile_dict_with_rules(&[], &TransposeConfig::LATIN, &rules).expect("compile");
        let dict = Dictionary::from_bytes("en", bytes).expect("valid dict");
        assert!(dict.groups.groups1[b'a' as usize].is_some(), "group 'a' registered");
        assert!(dict.groups.groups1[b'b' as usize].is_some(), "group 'b' registered");
    }

    #[test]
    fn compile_dictionary_from_source_files() {
        let data = std::path::PathBuf::from("espeak-ng-data");
        if !data.join("phontab").exists() {
            eprintln!("[SKIP] no local phoneme data");
            return;
        }
        let mut phdata = PhonemeData::load(&data).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");

        // Write a synthetic dictsource dir (<lang>_list + <lang>_rules).
        let src = std::env::temp_dir().join("espeak_rs_dictsource");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("tx_list"), "hello\th@l'oU\nworld\tw'3:ld\n").unwrap();
        std::fs::write(src.join("tx_rules"), ".group a\n\ta\t@\n").unwrap();

        let bytes = compile_dictionary(&src, "tx", &phdata, &TransposeConfig::LATIN).expect("compile");
        let dict = Dictionary::from_bytes("en", bytes).expect("valid dict");

        // The list entry compiled and is looked up.
        let r = lookup(&dict, "hello", &LookupCtx::default()).expect("'hello' found");
        assert!(!r.phonemes.is_empty(), "list phonemes empty");
        // The rules group compiled and is registered.
        assert!(dict.groups.groups1[b'a' as usize].is_some(), "rule group 'a' registered");

        let _ = std::fs::remove_dir_all(&src);
    }

    #[test]
    fn compiled_rule_actually_translates() {
        use crate::dictionary::stress::StressOpts;
        use crate::translate::{word_to_phonemes, LangOptions};

        let dir = std::path::PathBuf::from("espeak-ng-data");
        if !dir.join("phontab").exists() {
            eprintln!("[SKIP] no local phoneme data");
            return;
        }
        let mut phdata = PhonemeData::load(&dir).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");

        // A phoneme code to emit from the rule.
        let s_code = phdata.lookup_phoneme("s");
        assert!(s_code != 0, "'s' phoneme should exist");

        // Group 't', one no-context rule matching the group letter → phoneme /s/.
        let groups = vec![RuleGroup {
            name: "t".into(),
            rules: vec![Rule::new("", &[s_code])],
            letter_group: None,
            group3_ix: 0,
        }];
        let rules = compile_rules(&groups);
        let bytes = compile_dict_with_rules(&[], &TransposeConfig::LATIN, &rules).expect("compile");
        let dict = Dictionary::from_bytes("en", bytes).expect("valid dict");

        // Translate "t" through the real engine: the empty dict list falls through
        // to our compiled rule, which must fire and emit /s/.
        let stress = StressOpts::for_lang("en");
        let opts = LangOptions::for_lang("en");
        let wr = word_to_phonemes("t", &dict, &phdata, &stress, &opts);
        assert!(
            wr.phonemes.contains(&s_code),
            "compiled rule did not fire (phonemes = {:?})",
            wr.phonemes
        );
    }

    #[test]
    fn rules_dsl_parses_context_syntax() {
        let dir = std::path::PathBuf::from("espeak-ng-data");
        if !dir.join("phontab").exists() {
            return;
        }
        let mut phdata = PhonemeData::load(&dir).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");

        // `k)a` = pre-context 'k'; `a(b` = post-context 'b'.
        let source = ".group a\n\tk)a\t@\n\ta(b\ts\n";
        let groups = parse_rules_dsl(source, &phdata);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].rules.len(), 2);
        assert_eq!(groups[0].rules[0].pre_context, "k");
        assert_eq!(groups[0].rules[0].match_letters, ""); // "a" − group "a"
        assert_eq!(groups[0].rules[1].post_context, "b");
        assert_eq!(groups[0].rules[1].match_letters, "");
    }

    #[test]
    fn rules_dsl_reads_phonemes_from_last_column() {
        use crate::dictionary::stress::StressOpts;
        use crate::translate::{word_to_phonemes, LangOptions};

        let dir = std::path::PathBuf::from("espeak-ng-data");
        if !dir.join("phontab").exists() {
            return;
        }
        let mut phdata = PhonemeData::load(&dir).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");
        let s_code = phdata.lookup_phoneme("s");
        let b_code = phdata.lookup_phoneme("b");

        // Real espeak layout: whitespace-separated columns
        // `[pre)] match [(post]   <phonemes>`.  The phonemes are the LAST column;
        // reading the 2nd token instead wrongly used the match letter as the
        // phonemes (the bug that made every eo `r`/`n` mistranslate).  An
        // out-of-range letter group (`S)` = group 18) must not panic the reader.
        let source = ".group a\n\t_) a (_\ts\n\tS) a\tb\n\ta\tb\n";
        let groups = parse_rules_dsl(source, &phdata);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].rules[0].pre_context, "_");
        assert_eq!(groups[0].rules[0].post_context, "_");
        assert_eq!(groups[0].rules[0].phonemes, vec![s_code], "phonemes from last column");
        assert_eq!(groups[0].rules[2].phonemes, vec![b_code]);

        // End-to-end: compile + read.  "a" alone hits the whole-word rule → /s/;
        // the S-group rule can't match but must not crash.
        let rules = compile_rules(&groups);
        let bytes = compile_dict_with_rules(&[], &TransposeConfig::LATIN, &rules).expect("compile");
        let dict = Dictionary::from_bytes("en", bytes).expect("valid dict");
        let stress = StressOpts::for_lang("en");
        let opts = LangOptions::for_lang("en");
        let hit = word_to_phonemes("a", &dict, &phdata, &stress, &opts);
        assert!(hit.phonemes.contains(&s_code), "whole-word 'a' → /s/: {:?}", hit.phonemes);
    }

    #[test]
    fn list_entry_dollar_flags_are_not_phonemes() {
        let dir = std::path::PathBuf::from("espeak-ng-data");
        if !dir.join("phontab").exists() {
            return;
        }
        let mut phdata = PhonemeData::load(&dir).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");
        // Flags-only entry (`$u+ $pause`) → no phonemes, so the word falls
        // through to the rules (previously "$u+" was mis-parsed as the vowel "u").
        let flags_only = parse_list_dsl("gxis\t$u+ $pause\n", &phdata);
        assert_eq!(flags_only.len(), 1);
        assert!(flags_only[0].phonemes.is_empty(), "flags-only entry must have no phonemes");
        // A real phoneme column is still parsed even with a trailing flag.
        let with_ph = parse_list_dsl("cat\tk'at\t$verbf\n", &phdata);
        assert!(!with_ph[0].phonemes.is_empty(), "phonemes parsed when present");
    }

    #[test]
    fn list_entry_condition_prefix_gates_lookup() {
        use crate::dictionary::lookup::{lookup, LookupCtx};

        let dir = std::path::PathBuf::from("espeak-ng-data");
        if !dir.join("phontab").exists() {
            return;
        }
        let mut phdata = PhonemeData::load(&dir).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");

        // `?4 gramx  k'at` → an entry usable only when dict-condition 4 is set;
        // the leading `?4` is a guard, not the word (it was mis-parsed as "?4").
        let entries = parse_list_dsl("?4 gramx\tk'at\n", &phdata);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].word, "gramx", "the word is `gramx`, not `?4`");
        assert_eq!(entries[0].flags, vec![104], "?4 → flag byte 100+4");

        let bytes = compile_dict(&entries, &TransposeConfig::LATIN).expect("compile");
        let dict = Dictionary::from_bytes("en", bytes).expect("valid dict");
        // Without condition 4 active, the entry is skipped.
        assert!(lookup(&dict, "gramx", &LookupCtx::default()).is_none(), "gated off by default");
        // With condition 4 active, it matches.
        let ctx = LookupCtx { dict_condition: 1 << 4, ..Default::default() };
        assert!(lookup(&dict, "gramx", &ctx).is_some(), "matches when condition 4 set");
    }

    #[test]
    fn replace_section_compiles_decodes_and_applies() {
        use crate::dictionary::stress::StressOpts;
        use crate::translate::{word_to_phonemes, LangOptions};

        let dir = std::path::PathBuf::from("espeak-ng-data");
        if !dir.join("phontab").exists() {
            return;
        }
        let mut phdata = PhonemeData::load(&dir).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");
        let s_code = phdata.lookup_phoneme("s");

        // `.replace` maps z→q; group q → /s/.  So "z" folds to "q" then reads /s/.
        let src = ".replace\n\tz\tq\n.group q\n\tq\ts\n";
        let replace = parse_replace_dsl(src);
        assert_eq!(replace, vec![("z".to_string(), "q".to_string())]);
        let groups = parse_rules_dsl(src, &phdata);
        let rules = compile_rules(&groups);
        let bytes = compile_dict_full(&[], &TransposeConfig::LATIN, &rules, &replace).expect("compile");
        let dict = Dictionary::from_bytes("en", bytes).expect("valid dict");
        // The table decodes back to the one pair.
        assert_eq!(dict.replace_pairs.len(), 1);
        // …and applies before rule lookup: "z" → "q" → /s/.
        let opts = LangOptions::for_lang("en");
        let stress = StressOpts::for_lang("en");
        let out = word_to_phonemes("z", &dict, &phdata, &stress, &opts);
        assert!(out.phonemes.contains(&s_code), "z→q→/s/ via .replace: {:?}", out.phonemes);
    }

    #[test]
    fn compiled_rule_post_context_gates_match() {
        use crate::dictionary::stress::StressOpts;
        use crate::translate::{word_to_phonemes, LangOptions};

        let data = std::path::PathBuf::from("espeak-ng-data");
        if !data.join("phontab").exists() {
            eprintln!("[SKIP] no local phoneme data");
            return;
        }
        let mut phdata = PhonemeData::load(&data).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");
        let s_code = phdata.lookup_phoneme("s");

        // Group 'a': match the group letter *when followed by 'b'* → /s/.
        let rule = Rule {
            post_context: "b".into(),
            phonemes: vec![s_code],
            ..Default::default()
        };
        let groups = vec![RuleGroup { name: "a".into(), rules: vec![rule] , letter_group: None, group3_ix: 0 }];
        let rules = compile_rules(&groups);
        let bytes = compile_dict_with_rules(&[], &TransposeConfig::LATIN, &rules).expect("compile");
        let dict = Dictionary::from_bytes("en", bytes).expect("valid dict");

        let stress = StressOpts::for_lang("en");
        let opts = LangOptions::for_lang("en");
        // Post-context present → the rule fires.
        let hit = word_to_phonemes("ab", &dict, &phdata, &stress, &opts);
        assert!(hit.phonemes.contains(&s_code), "post-context rule should fire on 'ab': {:?}", hit.phonemes);
        // Post-context absent → the rule must NOT fire.
        let miss = word_to_phonemes("ax", &dict, &phdata, &stress, &opts);
        assert!(!miss.phonemes.contains(&s_code), "post-context rule wrongly fired on 'ax': {:?}", miss.phonemes);
    }

    #[test]
    fn compiled_rule_pre_context_gates_match() {
        use crate::dictionary::stress::StressOpts;
        use crate::translate::{word_to_phonemes, LangOptions};

        let data = std::path::PathBuf::from("espeak-ng-data");
        if !data.join("phontab").exists() {
            return;
        }
        let mut phdata = PhonemeData::load(&data).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");
        let s_code = phdata.lookup_phoneme("s");
        let k_code = phdata.lookup_phoneme("k");
        let b_code = phdata.lookup_phoneme("b");

        // 'a' → /s/ *when preceded by 'k'*; 'k'/'b' get trivial rules so the
        // preceding letter translates and the engine advances to 'a'.
        let groups = vec![
            RuleGroup {
                name: "a".into(),
                rules: vec![Rule { pre_context: "k".into(), phonemes: vec![s_code], ..Default::default() }],
                letter_group: None,
                group3_ix: 0,
            },
            RuleGroup { name: "k".into(), rules: vec![Rule::new("", &[k_code])], letter_group: None, group3_ix: 0 },
            RuleGroup { name: "b".into(), rules: vec![Rule::new("", &[b_code])], letter_group: None, group3_ix: 0 },
        ];
        let rules = compile_rules(&groups);
        let bytes = compile_dict_with_rules(&[], &TransposeConfig::LATIN, &rules).expect("compile");
        let dict = Dictionary::from_bytes("en", bytes).expect("valid dict");

        let stress = StressOpts::for_lang("en");
        let opts = LangOptions::for_lang("en");
        let hit = word_to_phonemes("ka", &dict, &phdata, &stress, &opts);
        assert!(hit.phonemes.contains(&s_code), "pre-context rule should fire on 'ka': {:?}", hit.phonemes);
        let miss = word_to_phonemes("ba", &dict, &phdata, &stress, &opts);
        assert!(!miss.phonemes.contains(&s_code), "pre-context rule wrongly fired on 'ba': {:?}", miss.phonemes);
    }

    #[test]
    fn compiled_word_boundary_context_gates_match() {
        use crate::dictionary::stress::StressOpts;
        use crate::translate::{word_to_phonemes, LangOptions};

        let data = std::path::PathBuf::from("espeak-ng-data");
        if !data.join("phontab").exists() {
            return;
        }
        let mut phdata = PhonemeData::load(&data).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");
        let s_code = phdata.lookup_phoneme("s");
        let b_code = phdata.lookup_phoneme("b");

        // 'a' → /s/ *only at word start* (pre-context `_`); 'b' translates so the
        // engine advances past it in "ba" and reaches the (non-initial) 'a'.
        let groups = vec![
            RuleGroup {
                name: "a".into(),
                rules: vec![Rule { pre_context: "_".into(), phonemes: vec![s_code], ..Default::default() }], letter_group: None, group3_ix: 0 },
            RuleGroup { name: "b".into(), rules: vec![Rule::new("", &[b_code])], letter_group: None, group3_ix: 0 },
        ];
        let rules = compile_rules(&groups);
        let bytes = compile_dict_with_rules(&[], &TransposeConfig::LATIN, &rules).expect("compile");
        let dict = Dictionary::from_bytes("en", bytes).expect("valid dict");

        let stress = StressOpts::for_lang("en");
        let opts = LangOptions::for_lang("en");
        // 'a' at word start → boundary precedes it → fires.
        let hit = word_to_phonemes("a", &dict, &phdata, &stress, &opts);
        assert!(hit.phonemes.contains(&s_code), "word-start rule should fire on 'a': {:?}", hit.phonemes);
        // 'a' after 'b' → no boundary → must NOT fire.
        let miss = word_to_phonemes("ba", &dict, &phdata, &stress, &opts);
        assert!(!miss.phonemes.contains(&s_code), "word-start rule wrongly fired on 'ba': {:?}", miss.phonemes);
    }

    #[test]
    fn compiled_letter_group_context_gates_match() {
        use crate::dictionary::stress::StressOpts;
        use crate::translate::{word_to_phonemes, LangOptions};

        let data = std::path::PathBuf::from("espeak-ng-data");
        if !data.join("phontab").exists() {
            return;
        }
        let mut phdata = PhonemeData::load(&data).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");
        let s_code = phdata.lookup_phoneme("s");
        let a_code = phdata.lookup_phoneme("a");
        let k_code = phdata.lookup_phoneme("k");

        // 'b' → /s/ *when preceded by a vowel* (letter-group `A` = English group 0,
        // `aeiou`); 'a'/'k' get trivial rules so the preceding letter translates.
        let groups = vec![
            RuleGroup {
                name: "b".into(),
                rules: vec![Rule { pre_context: "A".into(), phonemes: vec![s_code], ..Default::default() }], letter_group: None, group3_ix: 0 },
            RuleGroup { name: "a".into(), rules: vec![Rule::new("", &[a_code])], letter_group: None, group3_ix: 0 },
            RuleGroup { name: "k".into(), rules: vec![Rule::new("", &[k_code])], letter_group: None, group3_ix: 0 },
        ];
        let rules = compile_rules(&groups);
        let bytes = compile_dict_with_rules(&[], &TransposeConfig::LATIN, &rules).expect("compile");
        let dict = Dictionary::from_bytes("en", bytes).expect("valid dict");

        let stress = StressOpts::for_lang("en");
        let opts = LangOptions::for_lang("en");
        // 'b' after the vowel 'a' → group matches → fires.
        let hit = word_to_phonemes("ab", &dict, &phdata, &stress, &opts);
        assert!(hit.phonemes.contains(&s_code), "vowel-group rule should fire on 'ab': {:?}", hit.phonemes);
        // 'b' after the consonant 'k' → not a vowel → must NOT fire.
        let miss = word_to_phonemes("kb", &dict, &phdata, &stress, &opts);
        assert!(!miss.phonemes.contains(&s_code), "vowel-group rule wrongly fired on 'kb': {:?}", miss.phonemes);
    }

    #[test]
    fn round_trip_rules_group() {
        use crate::dictionary::RULE_PHONEMES;
        // A group `a` with one no-context rule: match the group letter → phonemes.
        let phonemes = vec![100u8, 101, 102];
        let groups = vec![RuleGroup {
            name: "a".into(),
            rules: vec![Rule::new("", &phonemes)], letter_group: None, group3_ix: 0 }];
        let rules = compile_rules(&groups);
        let dict_bytes = compile_dict_with_rules(&[], &TransposeConfig::LATIN, &rules).expect("compile");
        let dict = Dictionary::from_bytes("en", dict_bytes).expect("valid dict");

        // The reader's `build_groups` registered group 'a' (groups1['a']).
        let off = dict.groups.groups1[b'a' as usize].expect("group 'a' not registered");
        // The rule bytes at that offset: RULE_PHONEMES, phonemes…, 0.
        assert_eq!(dict.data[off], RULE_PHONEMES);
        assert_eq!(&dict.data[off + 1..off + 1 + phonemes.len()], phonemes.as_slice());
        assert_eq!(dict.data[off + 1 + phonemes.len()], 0, "rule not null-terminated");
    }

    #[test]
    fn flags_only_entry_round_trips() {
        // A flags-only entry (no phonemes) with one flags2 bit set.
        let entry = ListEntry { word: "the".into(), phonemes: vec![], flags: vec![32 + 3] };
        let bytes = compile_dict(&[entry], &TransposeConfig::LATIN).expect("compile");
        let dict = Dictionary::from_bytes("en", bytes).expect("valid dict");
        let r = lookup(&dict, "the", &LookupCtx::default()).expect("found");
        assert!(r.phonemes.is_empty(), "flags-only entry has no phonemes");
        assert!(r.flags2.contains(1 << 3), "flags2 bit 3 should be set");
    }

    /// `$flags` on a list entry compile to their flag bytes, so a conditional
    /// entry stays conditional (`a  eI  $atend` is the letter name only at a
    /// clause end).
    #[test]
    fn list_entry_dollar_flags_are_compiled() {
        let dir = std::path::PathBuf::from("espeak-ng-data");
        if !dir.join("phontab").exists() {
            return;
        }
        let mut phdata = PhonemeData::load(&dir).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");
        let entries = parse_list_dsl("a\teI\t$atend\nto\ttu:\t$u $allcaps\n", &phdata);
        assert_eq!(entries.len(), 2);
        // `$atend` is dictionary word 2, bit 0x11 → flag byte 0x31.
        assert_eq!(entries[0].flags, vec![0x31]);
        // `$u` is a stress directive (0x48); `$allcaps` is word 2 bit 0x0a.
        assert_eq!(entries[1].flags, vec![0x48, 0x2a]);
        assert!(!entries[0].phonemes.is_empty(), "the phoneme column still parses");
    }

    /// Unknown or directive-only `$` tokens are ignored rather than becoming
    /// phonemes.
    #[test]
    fn unknown_dollar_flags_are_ignored() {
        let dir = std::path::PathBuf::from("espeak-ng-data");
        if !dir.join("phontab").exists() {
            return;
        }
        let mut phdata = PhonemeData::load(&dir).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");
        // An unknown `$flag` is dropped; `$textmode` is a directive, so it
        // contributes no flag byte of its own — but it does put the rest of the
        // file (this entry included) into replacement-text mode, which the entry
        // records as `BITNUM_FLAG_TEXTMODE` (29) because the language's own
        // default is phonemes.
        let entries = parse_list_dsl("x\ta\t$textmode $nosuchflag\n", &phdata);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].flags, vec![29]);
        assert_eq!(entries[0].phonemes, b"a", "the column is text, not phonemes");

        // `$phonememode` switches back, and a plain unknown flag is still
        // ignored.
        let entries = parse_list_dsl("x\ta\t$nosuchflag\n", &phdata);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].flags.is_empty(), "no flag bytes for an unknown name");
        assert!(!entries[0].phonemes.is_empty());

        // A line that is *only* `$textmode` defines no entry.
        let entries = parse_list_dsl("$textmode\nx\ta\n$phonememode\ny\ta\n", &phdata);
        assert_eq!(entries.len(), 2, "the directives are not entries");
        assert_eq!(entries[0].phonemes, b"a", "before $phonememode: text");
        assert_ne!(entries[1].phonemes, b"a", "after $phonememode: phonemes");
    }

    /// A rule with a post-context but **no** phoneme column is a *silent* rule.
    /// Reading the last column as phonemes turned `c (q` (c is silent before q)
    /// into "c is pronounced /q/", which mispronounced "cat".
    #[test]
    fn rule_without_phonemes_is_silent() {
        let dir = std::path::PathBuf::from("espeak-ng-data");
        if !dir.join("phontab").exists() {
            return;
        }
        let mut phdata = PhonemeData::load(&dir).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");
        let groups = parse_rules_dsl(".group c\n        c          k\n        c (q\n", &phdata);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].rules.len(), 2);
        assert!(!groups[0].rules[0].phonemes.is_empty(), "`c  k` has phonemes");
        assert_eq!(groups[0].rules[1].post_context, "q");
        assert!(groups[0].rules[1].phonemes.is_empty(), "`c (q` is silent");
    }

    /// `.Lnn <items…>` defines a letter group that rule contexts refer to as
    /// `Lnn`.  Ten of them are used by the shipped English rules, and a rule
    /// whose group is undefined never matches.
    #[test]
    fn letter_groups_compile_and_are_referenced() {
        let dir = std::path::PathBuf::from("espeak-ng-data");
        if !dir.join("phontab").exists() {
            return;
        }
        let mut phdata = PhonemeData::load(&dir).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");

        let groups = parse_rules_dsl(
            ".L01  l r\n.L02  i y\n.group c\n        c          k\n     _) c (L01Y    k\n",
            &phdata,
        );
        // Two definitions and one letter group.
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].letter_group, Some((1, vec!["l".into(), "r".into()])));
        assert_eq!(groups[1].letter_group, Some((2, vec!["i".into(), "y".into()])));
        assert_eq!(groups[2].name, "c");

        let rules = compile_rules(&groups);
        // Definitions are emitted first, each as `START, RULE_LETTERGP2, 'A'+n`.
        assert_eq!(rules[0], super::super::RULE_GROUP_START);
        assert_eq!(rules[1], super::super::RULE_LETTERGP2);
        assert_eq!(rules[2], b'A' + 1);
        // …and the rule's context refers to group 1 the same way.
        let refs = rules
            .windows(2)
            .filter(|w| w[0] == super::super::RULE_LETTERGP2 && w[1] == b'A' + 1)
            .count();
        assert!(refs >= 2, "the definition and the reference both encode group 1");
    }

    /// A `.Lnn` line with a malformed number is ignored rather than producing a
    /// bogus group.
    #[test]
    fn malformed_letter_group_is_ignored() {
        let dir = std::path::PathBuf::from("espeak-ng-data");
        if !dir.join("phontab").exists() {
            return;
        }
        let mut phdata = PhonemeData::load(&dir).expect("phdata");
        phdata.select_table_by_name("en").expect("en table");
        for src in [".Lxx a b\n", ".L99\n", ".L\n"] {
            let groups = parse_rules_dsl(src, &phdata);
            assert!(
                groups.iter().all(|g| g.letter_group.is_none()),
                "{src:?} should not define a group"
            );
        }
    }
}
