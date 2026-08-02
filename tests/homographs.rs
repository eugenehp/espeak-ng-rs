// tests/homographs.rs
//
// Part-of-speech homograph selection.  The dictionary's default is the
// noun/adjective form; a verb-triggering word ("to", a modal) selects the
// `$verb` pronunciation and a perfect/passive auxiliary ("have", "was") the
// `$past` one.  Previously the port never set expect_verb/past, so "to live"
// read "lˈaɪv" instead of "lˈɪv".

use espeak_ng::Translator;

#[test]
fn verb_form_after_to_and_modals() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // After "to"/modal the verb pronunciation differs from the standalone default.
    for w in ["live", "use", "record", "close", "present"] {
        let default = ipa(w);
        let after_to = ipa(&format!("to {w}"));
        assert_ne!(
            after_to, format!("tuː {default}"),
            "'to {w}' should switch to the verb form (default was {default})"
        );
    }
    // Specific expected verb forms.
    assert_eq!(ipa("to record"), "tuː ɹɪkˈɔːd");
    assert_eq!(ipa("to use"), "tuː jˈuːz");
    assert_eq!(ipa("will record"), "wɪl ɹɪkˈɔːd");
    // Subject pronouns are verb triggers too.
    assert!(ipa("I live here").contains("lˈɪv"), "I live → lɪv: {}", ipa("I live here"));
    assert!(ipa("we use it").contains("jˈuːz"), "we use → juːz: {}", ipa("we use it"));
    // …but a determiner is not, so an adjective/noun context is unchanged.
    assert!(ipa("the live wire").contains("lˈaɪv"), "the live → laɪv: {}", ipa("the live wire"));
}

#[test]
fn past_form_after_auxiliary_and_defaults() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // "have read" → past "rˈɛd"; present "read" stays "rˈiːd".
    assert!(ipa("I have read it").contains("ɹˈɛd"), "have read → red: {}", ipa("I have read it"));
    assert!(ipa("I read books").contains("ɹˈiːd"), "read (pres) → reed: {}", ipa("I read books"));
    // No trigger → the default noun/adjective form is kept.
    assert_eq!(ipa("a record"), ipa("a") + " " + &ipa("record"));
    assert_eq!(ipa("record"), "ɹˈɛkɔːd");
    assert!(ipa("a live show").contains("lˈaɪv"), "live (adj) stays: {}", ipa("a live show"));
}

#[test]
fn noun_form_after_determiner() {
    let Some(t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // `$noun` homographs default to the verb; a determiner shifts them to the
    // noun/adjective form (stress on the first syllable).
    assert!(ipa("a convert").contains("kˈɒnvɜːt"), "a convert → noun: {}", ipa("a convert"));
    assert!(ipa("a refund").contains("ɹˈiːfʌnd"), "a refund → noun: {}", ipa("a refund"));
    assert!(ipa("an upset").contains("ˈʌpsɛt"), "an upset → noun: {}", ipa("an upset"));
    // …and they stay verbs after a verb trigger.
    assert!(ipa("to convert").contains("kɒnvˈɜːt"), "to convert → verb: {}", ipa("to convert"));
    // A non-$noun word after a determiner keeps its default (noun) pronunciation.
    assert!(ipa("the record").contains("ɹˈɛkɔːd"), "the record stays default: {}", ipa("the record"));
    assert!(ipa("a live show").contains("lˈaɪv"), "a live stays adj: {}", ipa("a live show"));
}
