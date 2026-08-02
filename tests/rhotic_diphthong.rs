// tests/rhotic_diphthong.rs
//
// The English "aI3" rhotic diphthong (fire/hire/wire) had no IPA override, so
// the mnemonic fallback dropped the "3" and rendered a bare "aɪ" — "fire" read
// "faɪ" (like "fie") instead of "faɪə".  The non-rhotic offglide is restored.

use espeak_ng::Translator;

#[test]
fn ire_words_keep_the_offglide() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    assert_eq!(ipa("fire"), "fˈaɪə");
    for w in ["tire", "wire", "hire", "choir", "desire", "empire", "liar"] {
        assert!(ipa(w).ends_with("aɪə"), "{w} should end in aɪə: {}", ipa(w));
    }
    // The 🔥 emoji reads "fire".
    assert_eq!(ipa("🔥"), "fˈaɪə");
    // Medial: the linking ɹ is added before the following sound.
    assert!(ipa("fiery").starts_with("fˈaɪə"), "{}", ipa("fiery"));
}

#[test]
fn non_rhotic_centering_words_unchanged() {
    // Words that already rendered a centering diphthong (as vowel + schwa
    // sequence, or the "@" mnemonic) must be untouched.
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    assert_eq!(ipa("hour"), "ˈaʊə");
    assert_eq!(ipa("higher"), "hˈaɪə");
    assert_eq!(ipa("here"), "hˈiə");
    assert_eq!(ipa("ear"), "ˈiə");
}
