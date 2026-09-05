// tests/dict_compiler.rs
//
// End-to-end checks on `--compile`: the port's `compiledict.c`.  The shipped
// `*_dict` files are compiled by upstream, so anything this compiler writes has
// to be read by the same reader and behave the same way — which makes a few
// details of the binary layout load-bearing rather than cosmetic:
//
//   * a word's list entries are written in **reverse** source order, because C
//     builds each hash chain by prepending;
//   * a group's rules are sorted by their phoneme string, and consecutive rules
//     with the same pronunciation share one copy via `RULE_PH_COMMON`;
//   * groups are sorted long-name-first and same-named groups are merged, so
//     that every multi-byte group sharing a first byte is contiguous;
//   * uppercase letters in a rule context are *commands* (`K` = not-a-vowel),
//     not letter groups — only `A B C F G H Y` are groups.

use std::path::{Path, PathBuf};

use espeak_ng::dictionary::compile::{compile_rules, parse_list_dsl, parse_rules_dsl};
use espeak_ng::dictionary::{
    RULE_GROUP_END, RULE_GROUP_START, RULE_PHONEMES, RULE_PH_COMMON, RULE_POST, RULE_PRE,
    RULE_PRE_ATSTART,
};
use espeak_ng::phoneme::PhonemeData;

fn phdata(table: &str) -> Option<PhonemeData> {
    let dir = Path::new("espeak-ng-data");
    if !dir.join("phontab").exists() {
        return None;
    }
    let mut p = PhonemeData::load(dir).ok()?;
    p.select_table_by_name(table).ok()?;
    Some(p)
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Uppercase letters in a context: `K` is "not a vowel", `A` is the vowel group.
#[test]
fn uppercase_context_letters_are_commands() {
    let Some(p) = phdata("en") else { return };
    let groups = parse_rules_dsl("\
.group w
        w          w
     w) wor (K     w
", &p);
    let bytes = compile_rules(&groups);
    assert!(
        bytes.contains(&espeak_ng::dictionary::RULE_NOTVOWEL),
        "`K` must compile to RULE_NOTVOWEL: {bytes:?}"
    );
    // …and it must *not* have become a letter-group reference.
    let lettergp = espeak_ng::dictionary::RULE_LETTERGP;
    assert!(
        !bytes.windows(2).any(|w| w[0] == lettergp && w[1] == b'K'),
        "`K` is not letter group 'K'"
    );

    let groups = parse_rules_dsl(".group c\n     c (A     k\n", &p);
    let bytes = compile_rules(&groups);
    assert!(
        bytes.windows(2).any(|w| w[0] == lettergp && w[1] == b'A'),
        "`A` is the vowel letter group"
    );
}

/// `_)` at the start of a pre-context becomes `RULE_PRE_ATSTART`, which also
/// matches the true start of a word — a literal space would not.
#[test]
fn word_start_context_uses_pre_atstart() {
    let Some(p) = phdata("en") else { return };
    let bytes = compile_rules(&parse_rules_dsl(".group k\n   _) k (n     \n", &p));
    assert!(bytes.contains(&RULE_PRE_ATSTART), "expected RULE_PRE_ATSTART: {bytes:?}");
    assert!(!bytes.contains(&RULE_PRE), "the `_` is implied, not written");
    assert!(bytes.contains(&RULE_POST));
}

/// Rules are sorted by their phoneme string and identical pronunciations are
/// written once, behind `RULE_PH_COMMON`.
#[test]
fn identical_pronunciations_are_shared() {
    let Some(p) = phdata("en") else { return };
    let src = "\
.group w
        wa         w
        wb         k
        wc         w
";
    let bytes = compile_rules(&parse_rules_dsl(src, &p));
    assert_eq!(bytes.iter().filter(|&&b| b == RULE_PH_COMMON).count(), 1);
    // Three rules, but only two copies of a phoneme string.
    assert_eq!(bytes.iter().filter(|&&b| b == RULE_PHONEMES).count(), 2);
}

/// Groups come out long-name-first and same-named groups merge into one header.
#[test]
fn groups_are_sorted_and_merged() {
    let Some(p) = phdata("en") else { return };
    let src = "\
.group a
        a          a
.group ab
        ab         a
.group b
        b          b
.group a
        ax         a
";
    let bytes = compile_rules(&parse_rules_dsl(src, &p));
    // Collect the group names in emission order.
    let mut names = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == RULE_GROUP_START {
            let start = i + 1;
            let end = start + bytes[start..].iter().position(|&b| b == 0).unwrap();
            names.push(String::from_utf8_lossy(&bytes[start..end]).into_owned());
            i = end + 1;
        } else {
            i += 1;
        }
    }
    assert_eq!(names, vec!["ab", "a", "b"], "long names first, `a` merged once");
    assert_eq!(bytes.iter().filter(|&&b| b == RULE_GROUP_END).count(), 3);
}

/// A word's list entries are written in reverse source order, so the *last*
/// definition in the file is the first one `Lookup` considers.
#[test]
fn list_entries_are_written_in_reverse_source_order() {
    let Some(p) = phdata("en") else { return };
    let entries = parse_list_dsl("a\t$nounf\na\teI\t$atend\n", &p);
    assert_eq!(entries.len(), 2);
    // The parser keeps source order; the *writer* reverses it.  Compile and read
    // the first entry out of its bucket.
    let bytes = espeak_ng::dictionary::compile::compile_dict(
        &entries,
        &espeak_ng::dictionary::transpose::TransposeConfig::LATIN,
    )
    .expect("compile");
    let dict = espeak_ng::dictionary::file::Dictionary::from_bytes("en", bytes).expect("dict");
    let ctx = espeak_ng::dictionary::lookup::LookupCtx {
        at_clause_end: true,
        ..Default::default()
    };
    let r = espeak_ng::dictionary::lookup::lookup(&dict, "a", &ctx).expect("found");
    assert!(!r.phonemes.is_empty(), "the `$atend` entry wins at a clause end");
}

/// A capitalised key is stored lowercased, and `U+xxxx` names a character.
#[test]
fn list_keys_are_normalised() {
    let Some(p) = phdata("en") else { return };
    let entries = parse_list_dsl("Louis\tlu:i\nU+0041\ta\n", &p);
    assert_eq!(entries[0].word, "louis");
    assert_eq!(entries[1].word, "A");
}

/// `--compile` round-trips upstream-style sources into a dictionary the engine
/// can actually speak from.
#[test]
fn compiled_dictionary_speaks() {
    let data = Path::new("espeak-ng-data");
    if !data.join("phontab").exists() {
        return;
    }
    let dir = scratch("dict_compiler_e2e");
    for f in ["phontab", "phonindex", "phondata", "phondata-manifest", "intonations"] {
        let _ = std::fs::copy(data.join(f), dir.join(f));
    }
    for sub in ["lang", "voices"] {
        let _ = std::process::Command::new("cp")
            .arg("-R")
            .arg(data.join(sub))
            .arg(&dir)
            .status();
    }
    std::fs::write(
        dir.join("en_rules"),
        "\
.group h
        h          h
.group e
        e          E
     h) e (l       @
.group l
        l          l
.group o
        o          oU
",
    )
    .unwrap();
    std::fs::write(dir.join("en_list"), "hello\th@l'oU\n").unwrap();

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_espeak_cli"))
        .arg("--compile=en")
        .arg("--path")
        .arg(&dir)
        .output()
        .expect("run --compile");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(dir.join("en_dict").exists());

    let spoken = std::process::Command::new(env!("CARGO_BIN_EXE_espeak_cli"))
        .args(["-v", "en", "-x", "-q", "hello"])
        .env("ESPEAK_DATA_PATH", &dir)
        .output()
        .expect("speak");
    assert_eq!(
        String::from_utf8_lossy(&spoken.stdout).trim(),
        "h@l'oU",
        "the list entry is found in the compiled dictionary"
    );

    // A word not in the list goes through the compiled rules.
    let spoken = std::process::Command::new(env!("CARGO_BIN_EXE_espeak_cli"))
        .args(["-v", "en", "-x", "-q", "hell"])
        .env("ESPEAK_DATA_PATH", &dir)
        .output()
        .expect("speak");
    assert_eq!(String::from_utf8_lossy(&spoken.stdout).trim(), "h'@ll");
}
