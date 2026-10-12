// tests/greek.rs
//
// The Greek (el) voice was non-functional for Greek *words* — multi-letter
// Greek input produced nothing — because the Greek letter-bits offset
// (OFFSET_GREEK = 0x380) was missing, so the letter-to-sound rules couldn't
// classify Greek letters for their context conditions.  Skips without el data.

use espeak_ng::Translator;

fn el() -> Option<Translator> {
    Translator::new("el", None).ok()
}

#[test]
fn greek_words_are_pronounced() {
    let Some(t) = el() else { eprintln!("[SKIP] no el data"); return };
    // Common Greek words now read (previously all produced empty output).
    let cases: &[(&str, &str)] = &[
        ("γεια", "jˈa"),        // "hi"
        ("ναι", "nˈe"),         // "yes"
        // /x/ palatalises to /ç/ before a front vowel, which is what the
        // `κ (Y` / `χ (Y` rules need `LETTERGP_Y` for.
        ("όχι", "ˈoçi"),        // "no"
        ("Αθήνα", "aθˈina"),    // "Athens"
        ("Ελλάδα", "elˈaða"),   // "Greece"
    ];
    for (word, want) in cases {
        let got = t.text_to_ipa(word).unwrap();
        assert!(!got.trim().is_empty(), "el {word:?} produced no phonemes");
        assert!(got.contains(want), "el {word:?}: want {want:?}, got {got:?}");
    }
    // Numbers already worked and still do.
    assert!(!t.text_to_ipa("123").unwrap().trim().is_empty(), "el numbers");
}

/// Greek palatalises /k/ and /x/ before a front vowel, which `el_rules` writes
/// as `κ (Y  c` — so the language's `LETTERGP_Y` has to be defined.  The port
/// set only groups A, C and VOWEL2 for Greek, and the rules never fired.
#[test]
fn velars_palatalise_before_front_vowels() {
    let Some(t) = el() else { eprintln!("[SKIP] no el data"); return };
    assert_eq!(t.text_to_ipa("κόκκινη").unwrap(), "kˈocinˌi");
    assert_eq!(t.text_to_ipa("κύριος").unwrap(), "cˈiriˌos");
    assert_eq!(t.text_to_ipa("όχι").unwrap(), "ˈoçi");
    // …and not before a back vowel.
    assert_eq!(t.text_to_ipa("καλημέρα").unwrap(), "kˌalimˈera");
}

#[test]
fn cyrillic_languages_beyond_russian_read_words() {
    // The Cyrillic offset (0x420) was only set for ru/bg/uk/be/tt; the other
    // Cyrillic-primary voices (mk/kk/ky/ba…) produced empty output for words.
    for (lang, word) in [("mk", "здраво"), ("kk", "сәлем"), ("ky", "салам"), ("ba", "һаумы")] {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        assert!(!t.text_to_ipa(word).unwrap().trim().is_empty(), "{lang} {word:?} should not be empty");
    }
    // Serbian is Latin-primary in espeak — its Latin reading must still work.
    if let Some(t) = Translator::new("sr", None).ok() {
        assert!(t.text_to_ipa("zdravo").unwrap().contains("zdrˈavo"), "sr Latin still reads");
    }
}

#[test]
fn armenian_and_georgian_read_words() {
    // Both needed their script offset (OFFSET_ARMENIAN=0x530, OFFSET_GEORGIAN=
    // 0x10A0) so the rules could find the multi-byte rule chains via groups3.
    let cases: &[(&str, &str, &str)] = &[
        ("hy", "բարեւ", "baɹˈev"),
        ("hy", "Հայաստան", "hˌaɪastˈan"),
        ("ka", "გამარჯობა", "ɡˈamardʒˌoba"),
        ("ka", "საქართველო", "sˈakʰartʰvˌelo"),
    ];
    for (lang, word, want) in cases {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        let got = t.text_to_ipa(word).unwrap();
        assert!(got.contains(want), "{lang} {word:?}: want {want:?}, got {got:?}");
    }
}
