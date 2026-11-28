// tests/leading_zero_numbers.rs
//
// A number with a leading zero is a code, not a quantity (C `TranslateNumber`):
// a short one (≤3 digits) speaks its leading zeros then the number, a longer one
// is spoken digit-by-digit.  Previously the leading zeros were dropped ("007" →
// "seven").

use espeak_ng::Translator;

/// Each case is C espeak-ng's own IPA for the text.
fn oracle_check(lang: &str, cases: &[(&str, &str)]) {
    let Some(t) = Translator::new(lang, None).ok() else {
        eprintln!("[SKIP] no {lang} data");
        return;
    };
    for &(text, want) in cases {
        assert_eq!(t.text_to_ipa(text).unwrap(), want, "{lang} {text:?}");
    }
}

/// The zeros of a short number are one word before it (C's `ph_zeros`); a
/// longer one is read digit by digit, a pause after each pair.
#[test]
fn leading_zeros_as_c() {
    oracle_check(
        "en",
        &[
            ("007", "zˈiəɹəʊzˈiəɹəʊ sˈɛvən"),
            ("05", "zˈiəɹəʊ fˈaɪv"),
            ("000", "zˈiəɹəʊzˈiəɹəʊ zˈiəɹəʊ"),
            ("0800", "zˈiəɹəʊˈeɪtzˈiəɹəʊzˈiəɹəʊ"),
            ("0", "zˈiəɹəʊ"),
            ("7", "sˈɛvən"),
            ("42", "fˈɔːti tˈuː"),
            ("0.5", "zˈiəɹəʊ pɔɪnt fˈaɪv"),
        ],
    );
}
