// tests/acronym_periods.rs
//
// A period sitting tightly between single letters ("U.S.A.", "e.g.", "i.e.") is
// an acronym separator, not a sentence end — espeak-ng does not break the clause
// there.  The port used to treat every '.' as a clause boundary, so "U.S.A."
// came out as three separate clauses ("juː⏎ɛs⏎eɪ").  Clause breaks surface as
// '\n' in `text_to_ipa`, so the tests assert on their presence/absence.

use espeak_ng::Translator;

fn en() -> Option<Translator> {
    match Translator::new_default("en") {
        Ok(t) => Some(t),
        Err(_) => { eprintln!("[SKIP] no en dict"); None }
    }
}

#[test]
fn single_letter_acronyms_do_not_break_the_clause() {
    let Some(t) = en() else { return };
    for acronym in ["U.S.A.", "e.g.", "i.e.", "a.b", "a.m.", "p.m.", "I.B.M."] {
        let ipa = t.text_to_ipa(acronym).unwrap();
        assert!(!ipa.contains('\n'), "{acronym:?} broke into clauses: {ipa:?}");
    }
}

#[test]
fn real_sentence_periods_still_break() {
    let Some(t) = en() else { return };
    // A period followed by whitespace is a real sentence end.
    assert!(t.text_to_ipa("cat. Dog").unwrap().contains('\n'), "cat. Dog must break");
    // A single letter followed by ". " is a list marker / sentence — it breaks.
    assert!(t.text_to_ipa("A. First").unwrap().contains('\n'), "A. First must break");
}

#[test]
fn trailing_and_decimal_periods_unchanged() {
    let Some(t) = en() else { return };
    // Trailing period after a real word: one clause, no dangling break mid-text.
    assert!(!t.text_to_ipa("end.").unwrap().contains('\n'), "end. is one clause");
    // "3.14" stays a decimal ("three point one four"), not a broken pair.
    let dec = t.text_to_ipa("3.14").unwrap();
    assert!(!dec.contains('\n'), "3.14 must not break: {dec:?}");
    assert!(dec.contains("pɔɪnt"), "3.14 reads as a decimal: {dec:?}");
}

// A period tightly between multi-character words / numbers is not a sentence
// end either — espeak reads it as the "dot"/"point"/"punkt" symbol.
#[test]
fn period_between_words_reads_as_dot() {
    let Some(t) = en() else { return };
    // "cat.Dog" → "cat dot dog" (matches espeak-ng exactly), one clause.
    let ipa = t.text_to_ipa("cat.Dog").unwrap();
    assert!(!ipa.contains('\n'), "cat.Dog must not break: {ipa:?}");
    assert_eq!(ipa, "kˈat dˈɒt dˈɒɡ");
    // digit.word too ("5.and" → "five dot and").
    assert_eq!(t.text_to_ipa("5.and").unwrap(), "fˈaɪv dˈɒt and");
}

#[test]
fn period_between_digits_in_comma_decimal_locale() {
    // German writes decimals with a comma, so a period between digits is not a
    // decimal separator — espeak reads it as the "." symbol ("punkt"): "3.14" →
    // "drei punkt vierzehn" (14 as a whole number, not digit-by-digit).
    let Ok(t) = Translator::new_default("de") else { eprintln!("[SKIP] no de dict"); return };
    let ipa = t.text_to_ipa("3.14").unwrap();
    assert!(!ipa.contains('\n'), "de 3.14 must not break: {ipa:?}");
    assert!(ipa.contains("pˈʊŋkt"), "de 3.14 reads '.' as punkt: {ipa:?}");
    assert!(ipa.contains("fˈɪɾtseːn"), "de 3.14 reads 14 as vierzehn: {ipa:?}");
}
