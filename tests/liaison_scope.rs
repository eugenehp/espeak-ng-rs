// tests/liaison_scope.rs
//
// Word-final "liaison" suppression (drop a phoneme whose mnemonic ends in 2/3
// before a non-vowel) is a French feature.  It was applied to every language,
// so Hungarian "öt" (ö + the "t2" phoneme) lost its final consonant word-finally
// — `5` → "ö" instead of "öt".  The suppression is now French-only.

use espeak_ng::Translator;

#[test]
fn hungarian_numbers_keep_final_consonant() {
    let Some(t) = Translator::new("hu", None).ok() else {
        eprintln!("[SKIP] no hu data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // 5/6/7/15/25 all end in "t" and must keep it.
    assert_eq!(ipa("5"), "ˈøt", "hu 5 = öt");
    assert_eq!(ipa("6"), "hˈɑt", "hu 6 = hat");
    assert_eq!(ipa("7"), "hˈeːt", "hu 7 = hét");
    assert!(ipa("15").ends_with("øt"), "hu 15 = tizenöt: {}", ipa("15"));
    // In a decimal the final digit keeps its consonant too.
    assert!(ipa("3,5").ends_with("øt"), "hu 3,5 ends in öt: {}", ipa("3,5"));
}

#[test]
fn french_liaison_still_works() {
    let Some(t) = Translator::new("fr", None).ok() else {
        eprintln!("[SKIP] no fr data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // Liaison consonant is pronounced before a vowel, silent before a consonant.
    assert!(ipa("les amis").contains("lez"), "liaison z before vowel: {}", ipa("les amis"));
    assert!(!ipa("les chats").contains('z'), "no liaison before consonant: {}", ipa("les chats"));
    assert!(ipa("deux ans").contains("øz"), "deux+ans liaison: {}", ipa("deux ans"));
    assert!(!ipa("deux chats").contains('z'), "deux+chats no liaison: {}", ipa("deux chats"));
}
