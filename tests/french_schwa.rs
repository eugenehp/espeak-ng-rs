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

    // Upstream keeps the `-` of the reducible schwa `@-` in its IPA output
    // (`WritePhMnemonic` maps the mnemonic character by character), so `le` is
    // `lˈə-`, not `lˈə`.
    assert_eq!(ipa("le"), "lˈə-");
    // Every one of these clitics ends in a *reducible* vowel, whose mnemonic
    // carries a trailing `-`; `WritePhMnemonic` maps the mnemonic character by
    // character, so upstream prints the `-` too (`lˈə-`, `dˈy-`).
    for w in ["le", "la", "les", "du", "je", "de", "ne", "ce", "me", "te", "se"] {
        let got = ipa(w);
        assert!(got.ends_with('-'), "fr {w:?} should keep the reducible mark: {got:?}");
        assert_eq!(got.matches('-').count(), 1, "fr {w:?}: {got:?}");
    }
    // In a phrase the clitics are clean too.
    assert_eq!(ipa("je le vois"), "ʒə- lə- vwˈa");
    assert!(ipa("le petit chat").starts_with("lə-"), "{}", ipa("le petit chat"));
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
