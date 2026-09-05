// tests/sinhala.rs
//
// Sinhala (si) reading.  Sinhala is an abugida in its own (non-ISCII) block
// (U+0D80–U+0DFF).  Two pieces make it work: `letter_bits_offset = 0x0D80` so
// groups3 finds the si_rules jamo/consonant groups, and the built-in letter
// group **B** on the dependent vowel signs + virama (U+0DCA) so a consonant
// followed by a sign drops its inherent vowel (`ක (B → k`), exactly as the
// si_rules expect.  Previously all Sinhala produced empty output; a first cut
// with only the offset read the vowel signs aloud as their names.

use espeak_ng::Translator;

#[test]
fn sinhala_words_read_with_vowel_signs_and_virama() {
    let Some(t) = Translator::new("si", None).ok() else {
        eprintln!("[SKIP] no si data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    // Consonant + inherent vowel.
    assert_eq!(ipa("කම"), "kˈɐmə"); // kama
    assert_eq!(ipa("මම"), "mˈɐmə"); // mama (I)

    // A dependent vowel sign replaces the inherent vowel (no stray "pilla" name).
    for (word, want) in [("කි", "kˈi"), ("කු", "kˈu"), ("කො", "kˈo")] {
        assert_eq!(ipa(word), want, "si {word}");
    }
    // The virama (al-lakuna) drops the inherent vowel entirely.
    assert_eq!(ipa("ක්"), "k");

    // Real words read cleanly (no diacritic names leaking in).
    for (word, want) in [("ආයුබෝවන්", "ˈaːjubˌoːwən"), ("ලංකා", "lˈɐŋkaː")] {
        assert_eq!(ipa(word), want, "si {word}");
    }
    // Sanity: no leftover Sinhala combining marks and non-empty.
    let thanks = ipa("ස්තූතියි");
    assert!(!thanks.trim().is_empty() && !thanks.contains("pill"), "si thanks: {thanks}");
}
