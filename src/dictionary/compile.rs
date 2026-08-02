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
/// flags after the phonemes are ignored (not yet compiled).
pub fn parse_list_dsl(source: &str, phdata: &PhonemeData) -> Vec<ListEntry> {
    let mut out = Vec::new();
    for line in source.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace().peekable();
        // Leading `?N` / `?!N` tokens are dict-condition guards: the entry is
        // used only when condition N is (un)set (`bs`'s gendered number forms,
        // homographs).  Encoded as flag bytes 100+N (require) / 132+N (forbid),
        // which the reader honours against `LookupCtx::dict_condition`.  Without
        // this, `?4 _0M2 …` was mis-parsed with the word "?4".
        let mut flags: Vec<u8> = Vec::new();
        while let Some(cond) = parts.peek().and_then(|t| condition_flag_byte(t)) {
            flags.push(cond);
            parts.next();
        }
        let Some(word) = parts.next() else {
            continue;
        };
        // The rest is a phoneme string and/or `$flags`.  A `$`-prefixed token is
        // a flag directive, *not* a phoneme mnemonic — parsing it as phonemes
        // turned `ĝis  $u+ $pause` into the bogus phoneme "u".  An entry that is
        // only flags has no phoneme column, so it stays a flags-only entry and
        // the word falls through to the letter rules.
        let mut mnemonics: Option<&str> = None;
        for tok in parts {
            if tok.starts_with('$') {
                continue; // `$`-flag; bit encoding not yet ported
            }
            if mnemonics.is_none() {
                mnemonics = Some(tok);
            }
        }
        let codes = mnemonics
            .map(|m| crate::translate::parse_inline_phonemes(m, phdata))
            .unwrap_or_default();
        out.push(ListEntry { word: word.to_string(), phonemes: codes, flags });
    }
    out
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
    let list_src = std::fs::read_to_string(dir.join(format!("{lang}_list"))).unwrap_or_default();
    let rules_src = std::fs::read_to_string(dir.join(format!("{lang}_rules"))).unwrap_or_default();

    let entries = parse_list_dsl(&list_src, phdata);
    let groups = parse_rules_dsl(&rules_src, phdata);
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
    let mut groups: Vec<RuleGroup> = Vec::new();
    for line in source.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix(".group") {
            groups.push(RuleGroup { name: name.trim().to_string(), rules: Vec::new() });
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
        if parts.len() < 2 {
            continue;
        }
        let mnemonics = parts.pop().unwrap();
        let letter_spec: String = parts.concat();
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
fn encode_context(s: &str, reverse: bool) -> Vec<u8> {
    let mut tokens: Vec<Vec<u8>> = s
        .bytes()
        .map(|c| match c {
            b'_' => vec![super::RULE_SPACE],
            b'A'..=b'Z' => vec![super::RULE_LETTERGP, c],
            other => vec![other],
        })
        .collect();
    if reverse {
        tokens.reverse();
    }
    tokens.into_iter().flatten().collect()
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
    for g in groups {
        out.push(RULE_GROUP_START);
        out.extend_from_slice(g.name.as_bytes());
        out.push(0); // group-name terminator
        for r in &g.rules {
            out.extend_from_slice(r.match_letters.as_bytes());
            if !r.pre_context.is_empty() {
                out.push(super::RULE_PRE);
                out.extend(encode_context(&r.pre_context, true));
            }
            if !r.post_context.is_empty() {
                out.push(super::RULE_POST);
                out.extend(encode_context(&r.post_context, false));
            }
            out.push(RULE_PHONEMES);
            out.extend_from_slice(&r.phonemes);
            out.push(0); // rule terminator (rules are null-terminated strings)
        }
        out.push(RULE_GROUP_END);
    }
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

    for e in entries {
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
            return Err(format!("dict entry for {:?} too long ({} bytes)", e.word, entry.len()));
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
        let groups = vec![RuleGroup { name: "a".into(), rules: vec![rule] }];
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
            },
            RuleGroup { name: "k".into(), rules: vec![Rule::new("", &[k_code])] },
            RuleGroup { name: "b".into(), rules: vec![Rule::new("", &[b_code])] },
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
                rules: vec![Rule { pre_context: "_".into(), phonemes: vec![s_code], ..Default::default() }],
            },
            RuleGroup { name: "b".into(), rules: vec![Rule::new("", &[b_code])] },
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
                rules: vec![Rule { pre_context: "A".into(), phonemes: vec![s_code], ..Default::default() }],
            },
            RuleGroup { name: "a".into(), rules: vec![Rule::new("", &[a_code])] },
            RuleGroup { name: "k".into(), rules: vec![Rule::new("", &[k_code])] },
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
            rules: vec![Rule::new("", &phonemes)],
        }];
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
}
