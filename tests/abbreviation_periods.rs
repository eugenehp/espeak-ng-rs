// tests/abbreviation_periods.rs
//
// A period after a title abbreviation ("Dr.", "Mr.", "St.") is part of the
// abbreviation, not a sentence end — it must not insert a clause pause
// (upstream #2340).  A period after any other word is a real sentence boundary
// and is left alone.  A clause boundary shows up as a newline in the IPA.

use espeak_ng::Translator;

#[test]
fn title_abbreviation_period_is_not_a_clause_break() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    // No clause break (no newline) after a title abbreviation.
    for s in ["Dr. Smith", "Mr. Jones", "Mrs. Brown", "Prof. Lee", "St. Louis", "Gen. Grant"] {
        assert!(!ipa(s).contains('\n'), "{s:?} should not pause after the title: {:?}", ipa(s));
    }
    // Two titles in a row, still no pauses.
    assert!(!ipa("Mr. and Mrs. Smith").contains('\n'), "{}", ipa("Mr. and Mrs. Smith"));
    // The word after the title is still spoken.
    assert!(ipa("Dr. Smith").contains("smˈɪθ"), "{}", ipa("Dr. Smith"));
}

#[test]
fn real_sentence_period_still_breaks() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    // A period after an ordinary word is a genuine clause boundary.
    assert!(ipa("the end. Next one.").contains('\n'), "real sentence break lost: {:?}", ipa("the end. Next one."));
    // A period before a *lowercase* word is not a sentence end at all, whatever
    // precedes it — upstream reads "etc. and" and "5. item" as one clause.
    assert!(!ipa("etc. and").contains('\n'), "etc. + lowercase is one clause: {:?}", ipa("etc. and"));
    assert!(!ipa("5. item").contains('\n'), "number-dot + lowercase: {:?}", ipa("5. item"));
    // …but a capitalised word after it still starts a new clause.
    assert!(ipa("etc. And").contains('\n'), "etc. + capital breaks: {:?}", ipa("etc. And"));
}
