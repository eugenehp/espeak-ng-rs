// tests/replace_table.rs
//
// The rules-file `.replace` section (parsed as `Dictionary::replace_pairs`,
// applied by `apply_replacements` before dict/rule lookup) — the port used to
// parse it but never apply it.  It does language-specific character
// normalisation; here we cover English typographic ligatures (ﬁ→fi, æ→ae),
// which are `.replace` entries in en_dict.  Japanese kana normalisation runs
// through the same path (see tests/japanese_katakana.rs).

use espeak_ng::Translator;

#[test]
fn english_ligatures_are_normalized() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // The ﬁ ligature (U+FB01) reads as "fi": "ﬁsh" == "fish".
    assert_eq!(ipa("ﬁsh"), ipa("fish"));
    // The æ ligature reads as "ae": "encyclopædia" == "encyclopaedia".
    assert_eq!(ipa("encyclopædia"), ipa("encyclopaedia"));
    // A word with the ﬂ ligature (U+FB02).
    assert_eq!(ipa("ﬂag"), ipa("flag"));
}

#[test]
fn languages_without_replace_are_untouched() {
    // A language with no `.replace` section must be byte-identical to before
    // (apply_replacements returns None → no allocation, no change).  German has
    // none; a plain word stays exactly as the rules produce it.
    let Some(t) = Translator::new("de", None).ok() else {
        eprintln!("[SKIP] no de data");
        return;
    };
    let out = t.text_to_ipa("drei").unwrap();
    assert_eq!(out, "drˈaɪ", "de drei unchanged");
}
