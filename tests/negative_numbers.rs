// tests/negative_numbers.rs
//
// A unary minus written with an ASCII hyphen directly before a number ("-5")
// reads as the language's "minus" word.  Previously the leading hyphen fell
// through to the dropped-punctuation path, so "-5" read "five" (the sign was
// silently lost).  The fix only fires at a clause start or after a space — a
// hyphen following a word or number ("COVID-19", "5-10", a date) stays a
// separator, not a sign.

use espeak_ng::Translator;

#[test]
fn leading_minus_before_number_reads_minus() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    // "-5" must read exactly like the spelled-out "minus 5".
    assert_eq!(ipa("-5"), ipa("minus 5"));
    assert_eq!(ipa("-5"), "mˈaɪnəs fˈaɪv");
    // Works mid-clause (after a space) and after a clause boundary.
    assert!(ipa("was -5 degrees").contains("mˈaɪnəs fˈaɪv"), "{}", ipa("was -5 degrees"));
    assert!(ipa("temperature: -40").contains("mˈaɪnəs fˈɔːti"), "{}", ipa("temperature: -40"));
    // A decimal keeps the sign too.
    assert!(ipa("-5.5").starts_with("mˈaɪnəs"), "{}", ipa("-5.5"));
}

#[test]
fn leading_minus_before_currency_reads_minus() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    // A minus before a currency amount is a negative amount ("-$5" = minus five
    // dollars).  Previously the sign was dropped ("-$5" → "five dollars").
    assert_eq!(ipa("-$5"), "mˈaɪnəs fˈaɪv dˈɒləz");
    assert!(ipa("-€10").starts_with("mˈaɪnəs"), "{}", ipa("-€10"));
    assert!(ipa("-£5").starts_with("mˈaɪnəs"), "{}", ipa("-£5"));
    assert!(ipa("was -$5").contains("mˈaɪnəs fˈaɪv dˈɒləz"), "{}", ipa("was -$5"));
    // The `+`/`$` positive form is unchanged.
    assert_eq!(ipa("+$5"), "plˈʌs fˈaɪv dˈɒləz");
    assert_eq!(ipa("$5"), "fˈaɪv dˈɒləz");
    // A hyphen after a word/number before a currency symbol is NOT a minus.
    for s in ["US-$5", "5-$3", "price-$5"] {
        assert!(!ipa(s).contains("mˈaɪnəs"), "{s:?} should not read a minus: {}", ipa(s));
    }
}

#[test]
fn hyphen_after_word_or_number_is_not_a_minus() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    // A hyphen after a word/number is a separator/range, not a sign — no "minus".
    for s in ["5-10", "pages 5-10", "COVID-19", "2024-01-15", "1-800-flowers"] {
        assert!(!ipa(s).contains("mˈaɪnəs"), "{s:?} should not read a minus: {}", ipa(s));
    }
    // A spaced dash "3 - 5" (hyphen not immediately before the digit) is unaffected.
    assert!(!ipa("3 - 5").contains("mˈaɪnəs"), "{}", ipa("3 - 5"));
    // The letter-hyphen word-break behaviour is preserved.
    assert_eq!(ipa("well-known"), "wˈɛlnˈəʊn");
}

#[test]
fn leading_minus_is_language_neutral() {
    // The reading comes from each language's own symbol table (not a literal
    // English "minus"), so German "-5" reads its own "minus" word.
    let Some(t) = Translator::new("de", None).ok() else {
        eprintln!("[SKIP] no de data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    assert_eq!(ipa("-5"), ipa("minus 5"));
    assert!(!ipa("5-10").contains("mˈiːnʊs"), "de 5-10 should not read a minus: {}", ipa("5-10"));
}

#[test]
fn leading_minus_reads_the_sign_across_languages() {
    // Each language reads its own "minus"/"plus" word (not a literal English
    // one), so "-5" / "+5" must differ from a bare "5" — the sign is spoken,
    // not dropped.  Covers the languages whose symbol table gained readings.
    for lang in ["pl", "pt", "ru", "uk", "bg", "cs", "ro", "sv", "da", "nb", "tr"] {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        let ipa = |s: &str| t.text_to_ipa(s).unwrap();
        let (five, minus5, plus5) = (ipa("5"), ipa("-5"), ipa("+5"));
        assert!(
            !minus5.is_empty() && minus5 != five,
            "{lang} '-5' dropped the sign: {minus5:?} (5={five:?})"
        );
        assert!(
            !plus5.is_empty() && plus5 != five,
            "{lang} '+5' dropped the sign: {plus5:?} (5={five:?})"
        );
    }
}
