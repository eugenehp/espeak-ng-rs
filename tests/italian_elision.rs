// tests/italian_elision.rs
//
// Italian tens elide their final vowel before a vowel-initial unit (uno=1,
// otto=8): trenta+uno → "trentuno", ottanta+otto → "ottantotto".  The port
// previously only did this for the 20s (via dedicated `_21`/`_28` entries), so
// 31/38/41/… read the un-elided "trenta uno".  Verified vs the C oracle
// (`espeak-ng -v it -q --ipa 31` → "trˈeːntˈuno").

use espeak_ng::Translator;

#[test]
fn tens_elide_before_uno_and_otto() {
    let Some(t) = Translator::new("it", None).ok() else {
        eprintln!("[SKIP] no it data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // The elided compound is a single word with no stray "a"/space before uno/otto.
    assert_eq!(ipa("31"), "trˈeːntˈuno");
    assert_eq!(ipa("38"), "trˈeːntˈɔtːo");
    for n in ["41", "48", "81", "88", "91", "98"] {
        let got = ipa(n);
        // Must not contain the un-elided "…a uno/otto" (a separate vowel then stress).
        assert!(!got.contains("aˈuno") && !got.contains("aˈɔ"), "{n} not elided: {got}");
    }
    // Within hundreds too: 131 = "cento trentuno".
    assert!(ipa("131").ends_with("trˈeːntˈuno"), "{}", ipa("131"));
}

#[test]
fn consonant_initial_units_are_not_elided() {
    let Some(t) = Translator::new("it", None).ok() else {
        eprintln!("[SKIP] no it data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // 32 = "trentadue" keeps the 'a' (due starts with a consonant).
    assert!(ipa("32").contains("tadˈue"), "{}", ipa("32"));
    // Round tens are unchanged.
    assert_eq!(ipa("30"), "trˈeːnta");
}

#[test]
fn other_languages_unaffected() {
    // English and Spanish keep their (non-eliding) composition.
    if let Some(t) = Translator::new("en", None).ok() {
        assert_eq!(t.text_to_ipa("31").unwrap(), "θˈɜːti wˈɒn");
    }
    if let Some(t) = Translator::new("es", None).ok() {
        assert!(t.text_to_ipa("31").unwrap().contains("tɾˈeɪnta"), "{}", t.text_to_ipa("31").unwrap());
    }
}
