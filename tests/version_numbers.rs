// tests/version_numbers.rs
//
// A dotted number with more than one decimal point ("1.2.3", a version string)
// reads each '.' as "point": "one point two point three".  The port used to
// parse the second ".3" as a fresh leading decimal ("zero point three"), so
// "1.2.3" gained a spurious "zero" ("one point two ZERO point three").  A real
// standalone leading decimal (".5") still reads "zero point five".

use espeak_ng::Translator;

fn tr() -> Option<Translator> {
    match Translator::new_default("en") {
        Ok(t) => Some(t),
        Err(_) => { eprintln!("[SKIP] no en dict"); None }
    }
}

#[test]
fn dotted_version_has_no_spurious_zero() {
    let Some(t) = tr() else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // No "zero" (zˈiəɹəʊ) between the parts, and a "point" for each dot.
    let v = ipa("1.2.3");
    assert!(!v.contains("zˈiəɹəʊ"), "1.2.3 has no spurious 'zero': {v}");
    assert_eq!(v.matches("pɔɪnt").count(), 2, "1.2.3 reads two 'point's: {v}");
    assert!(v.starts_with("wˈɒn") && v.ends_with("θɹˈiː"), "1.2.3 = 'one point two point three': {v}");
    // Four-part version too.
    assert_eq!(ipa("1.2.3.4").matches("pɔɪnt").count(), 3);
}

#[test]
fn standalone_leading_decimal_keeps_zero() {
    let Some(t) = tr() else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // ".5" and "0.5" still read "zero point five" (the leading zero is spoken).
    assert!(ipa(".5").starts_with("zˈiəɹəʊ"), "'.5' = 'zero point five': {}", ipa(".5"));
    assert!(ipa("0.5").starts_with("zˈiəɹəʊ"), "'0.5' = 'zero point five': {}", ipa("0.5"));
    // An ordinary decimal is unchanged.
    assert_eq!(ipa("3.14"), "θɹˈiː pɔɪnt wˈɒn fˈɔː");
}
