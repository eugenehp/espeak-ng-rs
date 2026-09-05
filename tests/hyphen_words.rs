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
    // A hyphen between two letters breaks the word for *lookup* — each part is
    // translated on its own, so "well" and "known" get their own stress — but
    // the parts are spoken as one word, with no space between them.  That is
    // upstream's reading: `wˈɛlnˈəʊn`, not `wˈɛl nˈəʊn`.
    let joined = |s: &str| ipa(s).replace(' ', "");
    assert_eq!(ipa("well-known"), joined("well known"));
    assert_eq!(ipa("X-ray"), joined("X ray"));
    assert_eq!(ipa("mother-in-law"), joined("mother in law"));
    assert_eq!(ipa("well-known"), "wˈɛlnˈəʊn");
    assert!(!ipa("state-of-the-art").contains(' '), "one word: {}", ipa("state-of-the-art"));
    // …and each part still carries its own stress mark.
    assert_eq!(ipa("state-of-the-art").matches('ˈ').count(), 2);
}
