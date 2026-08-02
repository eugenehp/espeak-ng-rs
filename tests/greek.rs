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
        ("όχι", "ˈoxi"),        // "no"
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
        ("hy", "բարեւ", "bˈaɹev"),
        ("hy", "Հայաստան", "haɪˈastan"),
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
