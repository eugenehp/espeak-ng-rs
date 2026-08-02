// tests/leading_zero_numbers.rs
//
// A number with a leading zero is a code, not a quantity (C `TranslateNumber`):
// a short one (≤3 digits) speaks its leading zeros then the number, a longer one
// is spoken digit-by-digit.  Previously the leading zeros were dropped ("007" →
// "seven").

use espeak_ng::Translator;

#[test]
fn leading_zero_numbers_spell_the_zeros() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    let zero = ipa("0");
    let seven = ipa("7");

    // ≤3 digits: leading zeros, then the number.
    assert_eq!(ipa("007"), format!("{zero} {zero} {seven}"));
    assert_eq!(ipa("05"), format!("{zero} {}", ipa("5")));
    assert_eq!(ipa("000"), format!("{zero} {zero} {zero}"));

    // >3 digits with a leading zero: every digit individually.
    assert_eq!(ipa("0800"), format!("{zero} {} {zero} {zero}", ipa("8")));

    // Unaffected: a lone 0, ordinary numbers, and decimals.
    assert_eq!(ipa("0"), zero);
    assert_eq!(ipa("7"), seven);
    assert_eq!(ipa("42"), "fˈɔːti tˈuː");
    assert!(ipa("0.5").starts_with(&zero), "0.5 keeps 'zero point five': {}", ipa("0.5"));
}
