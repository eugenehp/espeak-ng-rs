// tests/large_numbers.rs
//
// Numbers ≥ 10^12 used to overflow the billion group: the scale split stopped
// at 10^9, so `1_000_000_000_000` rendered "ten hundred billion" instead of
// "one trillion".  The split now covers every scale word the language defines
// (up to 10^18 for languages that have the words); a language that stops
// earlier folds the excess into its highest scale rather than dropping it.

use espeak_ng::Translator;

#[test]
fn english_reads_trillion_quadrillion_quintillion() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    let trillion = ipa("1000000000000");
    assert!(trillion.contains("tɹˈɪliən"), "10^12 should be 'trillion': {trillion}");
    // The old bug produced "ten hundred billion" — guard against its return.
    assert!(!trillion.contains("hˈʌndɹɪd"), "10^12 must not read '…hundred billion': {trillion}");

    assert!(ipa("1000000000000000").contains("kwɒdɹ"), "10^15 should be 'quadrillion': {}", ipa("1000000000000000"));
    assert!(ipa("1000000000000000000").contains("kwɪnt"), "10^18 should be 'quintillion': {}", ipa("1000000000000000000"));
    assert_eq!(ipa("2000000000000"), "tˈuː tɹˈɪliən", "two trillion");
}

#[test]
fn billion_and_below_are_unchanged() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // Nothing below the trillion boundary changed.
    assert_eq!(ipa("42"), "fˈɔːti tˈuː");
    assert_eq!(ipa("999"), "nˈaɪnhˈʌndɹɪdən nˈaɪnti nˈaɪn");
    assert_eq!(ipa("1000000"), "wˈɒn mˈɪliən");
    assert_eq!(ipa("1000000000"), "wˈɒn bˈɪliən");
    assert_eq!(ipa("12345"), "twˈɛlv θˈaʊzənd θɹˈiːhˈʌndɹɪdən fˈɔːti fˈaɪv");
}

#[test]
fn german_long_scale_billion_is_ten_to_the_twelfth() {
    // German uses the long scale: 10^12 = "Billion" (not the short-scale
    // trillion), 10^18 = "Trillion".  Both require the higher scale words.
    let Some(t) = Translator::new("de", None).ok() else {
        eprintln!("[SKIP] no de data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    assert!(ipa("1000000000000").contains("bɪljˈoːn"), "de 10^12 = Billion: {}", ipa("1000000000000"));
    assert!(ipa("1000000000000000000").contains("tɾɪljˈoːn"), "de 10^18 = Trillion: {}", ipa("1000000000000000000"));
}

#[test]
fn language_without_the_scale_word_still_conveys_magnitude() {
    // French defines no single word past "milliard" (10^9), so 10^12 folds into
    // it ("dix cent milliards") rather than collapsing to a bare "un".
    let Some(t) = Translator::new("fr", None).ok() else {
        eprintln!("[SKIP] no fr data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    let tr = ipa("1000000000000");
    assert!(tr.contains("miljˈaʁ"), "fr 10^12 should still name 'milliard': {tr}");
    // Must not degrade to just "un" (one).
    assert_ne!(tr.trim(), "œ\u{303}", "fr 10^12 must not collapse to a bare 'un': {tr}");
}
