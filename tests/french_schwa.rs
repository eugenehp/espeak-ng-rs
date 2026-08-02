// tests/french_schwa.rs
//
// French clitics ("le", "la", "du", "je", …) end in the reducible "e caduc"
// schwa, whose phoneme mnemonic is "@-".  The IPA fallback rendered the "-"
// literally ("le" → "lˈə-"), but "-" is a variant marker on a vowel mnemonic,
// not an IPA symbol — it must be dropped so "le" reads "lˈə".

use espeak_ng::Translator;

#[test]
fn french_clitics_have_no_trailing_dash() {
    let Some(t) = Translator::new("fr", None).ok() else {
        eprintln!("[SKIP] no fr data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    assert_eq!(ipa("le"), "lˈə", "'le' should be lˈə, not lˈə-");
    for w in ["le", "la", "les", "du", "je", "de", "ne", "ce", "me", "te", "se"] {
        let got = ipa(w);
        assert!(!got.contains('-'), "fr {w:?} has a stray dash: {got:?}");
    }
    // In a phrase the clitics are clean too.
    assert_eq!(ipa("je le vois"), "ʒə lə vwˈa");
    assert!(!ipa("le petit chat").contains('-'), "{}", ipa("le petit chat"));
}

#[test]
fn english_output_unchanged() {
    // The dash-skip is scoped to vowels, so English (whose syllabic consonants
    // use explicit overrides) is untouched.
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    assert_eq!(ipa("the"), "ðˈə");
    assert_eq!(ipa("bottle"), "bˈɒtəl");
    assert_eq!(ipa("button"), "bˈʌtən");
}
