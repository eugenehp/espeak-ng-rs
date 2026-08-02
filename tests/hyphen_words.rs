// tests/hyphen_words.rs
//
// A hyphen between two letters is a word break, not part of the word (espeak
// TranslateClause).  "well-known" reads as "well known", "X-ray" as "eks ray".
// Previously the port kept the hyphen and joined the parts ("wellknown").

use espeak_ng::Translator;

#[test]
fn hyphen_between_letters_splits_words() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // The hyphenated form reads identically to the space-separated words.
    assert_eq!(ipa("well-known"), ipa("well known"));
    assert_eq!(ipa("X-ray"), ipa("X ray"));
    assert_eq!(ipa("mother-in-law"), ipa("mother in law"));
    // Each part is a separate word (interior space, > 1 stress region).
    assert!(ipa("state-of-the-art").contains(' '), "should split: {}", ipa("state-of-the-art"));
}
