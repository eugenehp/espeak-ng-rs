// tests/dash_reading.rs
//
// An ASCII hyphen-minus '-' directly between two numbers is read as the "-"
// symbol, which espeak-ng only voices for some languages (English "dash",
// German "Strich"); everywhere else it is dropped.  The port used to drop it for
// every language, so "5-10" came out as an ambiguous "five ten"; it now reads
// "five dash ten".  Only a digit on *both* sides triggers it — a letter (x-5),
// an en/em dash (5–10), or a word-joining hyphen (well-known) is untouched.
// Values were confirmed against the espeak-ng 1.52.0 C binary.

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    match Translator::new_default(lang) {
        Ok(t) => Some(t),
        Err(_) => { eprintln!("[SKIP] no {lang} dict"); None }
    }
}

#[test]
fn english_number_hyphen_reads_dash() {
    let Some(t) = tr("en") else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    assert_eq!(ipa("5-10"), "fˈaɪv dˈaʃ tˈɛn");
    assert_eq!(ipa("1-2"), "wˈɒn dˈaʃ tˈuː");
    // A chain gets a dash between every pair.
    assert_eq!(ipa("3-4-5"), "θɹˈiː dˈaʃ fˈɔː dˈaʃ fˈaɪv");
    assert_eq!(ipa("pages 3-7"), "pˈeɪdʒɪz θɹˈiː dˈaʃ sˈɛvən");
}

#[test]
fn german_uses_strich() {
    let Some(t) = tr("de") else { return };
    let ipa = t.text_to_ipa("5-10").unwrap();
    assert!(ipa.contains("ʃtrˈɪç"), "de 5-10 → 'Strich': {ipa}");
}

#[test]
fn only_digit_both_sides_and_only_ascii_hyphen() {
    let Some(t) = tr("en") else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // A letter on either side → no "dash".
    assert!(!ipa("x-5").contains("dˈaʃ"), "x-5 has no dash: {}", ipa("x-5"));
    assert!(!ipa("5-x").contains("dˈaʃ"), "5-x has no dash: {}", ipa("5-x"));
    // En dash / em dash are not the ASCII hyphen espeak reads as "dash".
    assert!(!ipa("5–10").contains("dˈaʃ"), "en-dash is silent: {}", ipa("5–10"));
    assert!(!ipa("5—10").contains("dˈaʃ"), "em-dash is silent: {}", ipa("5—10"));
}

#[test]
fn languages_without_a_dash_word_drop_it() {
    // fr/es/ru have no dash word in espeak-ng: "5-10" reads as two bare numbers.
    for lang in ["fr", "es", "ru"] {
        let Some(t) = tr(lang) else { continue };
        let with = t.text_to_ipa("5-10").unwrap();
        let plain = t.text_to_ipa("5 10").unwrap();
        assert_eq!(with, plain, "{lang} 5-10 should drop the hyphen");
    }
}
