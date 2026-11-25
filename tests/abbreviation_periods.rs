// tests/abbreviation_periods.rs
//
// A period before a capitalised word ends the clause, after a title ("Dr.
// Smith") as after any other word — C has no list of titles.  Before a
// lower-case word it doesn't.  A clause boundary shows up as a newline in the
// IPA.

use espeak_ng::Translator;

#[test]
fn title_period_before_a_capital_is_a_clause_break() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    assert_eq!(ipa("Dr. Smith"), "dˈɒktə\nsmˈɪθ");
    assert_eq!(ipa("Mr. Jones"), "mˈɪstə\ndʒˈəʊnz");
    assert_eq!(ipa("Dr. smith"), "dˈɒktə smˈɪθ");
    // Only the period before a capital breaks.
    assert_eq!(ipa("Mr. and Mrs. Smith"), "mˈɪstəɹ and mˈɪsɪz\nsmˈɪθ");
}

#[test]
fn real_sentence_period_still_breaks() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    // A period after an ordinary word is a genuine clause boundary.
    assert!(
        ipa("the end. Next one.").contains('\n'),
        "real sentence break lost: {:?}",
        ipa("the end. Next one.")
    );
    // A period before a *lowercase* word is not a sentence end at all, whatever
    // precedes it — upstream reads "etc. and" and "5. item" as one clause.
    assert!(
        !ipa("etc. and").contains('\n'),
        "etc. + lowercase is one clause: {:?}",
        ipa("etc. and")
    );
    assert!(
        !ipa("5. item").contains('\n'),
        "number-dot + lowercase: {:?}",
        ipa("5. item")
    );
    // …but a capitalised word after it still starts a new clause.
    assert!(
        ipa("etc. And").contains('\n'),
        "etc. + capital breaks: {:?}",
        ipa("etc. And")
    );
}
