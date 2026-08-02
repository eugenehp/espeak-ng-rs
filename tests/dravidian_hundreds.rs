// tests/dravidian_hundreds.rs
//
// Several languages use a distinct standalone form for a round hundred vs. the
// combining form spoken before more digits — Tamil "நூறு" (nūru) alone but
// "நூற்று" (nūṭṭri) in 101; likewise Kannada.  The dictionary encodes this as
// `_{n}C0` (exact) vs `_{n}C` (combining); the port used to always take the
// combining form, so a bare "100"/"200" was wrong.  Values confirmed against the
// espeak-ng 1.52.0 C binary.

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    match Translator::new_default(lang) {
        Ok(t) => Some(t),
        Err(_) => { eprintln!("[SKIP] no {lang} dict"); None }
    }
}

#[test]
fn tamil_standalone_vs_combining_hundred() {
    let Some(t) = tr("ta") else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // Standalone hundreds use the "நூறு" (…nūru) form, not "நூற்று" (…nūṭṭri).
    assert_eq!(ipa("100"), "nˈuːrʉ");
    assert!(ipa("100").ends_with("nˈuːrʉ") && !ipa("100").contains("ʈʈ"),
            "ta 100 is standalone 'nūru': {}", ipa("100"));
    assert!(ipa("200").ends_with("nˈuːrʉ"), "ta 200 = 'irunūru': {}", ipa("200"));
    // A remainder switches to the combining "நூற்று" form.
    assert!(ipa("101").contains("ʈʈri"), "ta 101 uses combining 'nūṭṭri': {}", ipa("101"));
}

#[test]
fn kannada_standalone_vs_combining_hundred() {
    let Some(t) = tr("kn") else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    assert_eq!(ipa("100"), "nˈuːɹu");             // standalone "…nūru"
    assert!(ipa("101").contains("ɹaː"), "kn 101 uses combining 'nūɹaː': {}", ipa("101"));
}

#[test]
fn malayalam_bare_one_hundred() {
    let Some(t) = tr("ml") else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // "100" uses the bare hundred word "നൂറ്" (…nūṟṟu), not "ഒരു നൂറ്" (oru …) —
    // no leading "one" and no combining form.
    assert!(ipa("100").starts_with("nˈuːɾ") && !ipa("100").contains("oɾ"),
            "ml 100 is bare 'nūṟṟu': {}", ipa("100"));
    assert!(!ipa("101").contains("oɾ"), "ml 101 drops 'oru' too: {}", ipa("101"));
    // 200 keeps its dedicated word ("irunnūṟu"), unaffected.
    assert!(ipa("200").contains("nn"), "ml 200 keeps dedicated form: {}", ipa("200"));
}

// The exact-thousands variant `_0M{n}x`: a round N000 uses a distinct scale word
// vs. one spoken before more digits — Tamil "ஆயிரம்" (āyiram) vs "ஆயிரத்தி"
// (āyiratti), Malayalam likewise; and several of these languages also drop the
// "one" before "thousand".  Confirmed against the espeak-ng C binary.
#[test]
fn exact_thousand_variant() {
    if let Some(t) = tr("ta") {
        let ipa = |s: &str| t.text_to_ipa(s).unwrap();
        // Round thousand → bare "āyiram", no leading "onru".
        assert!(ipa("1000").contains("aːj") && ipa("1000").ends_with("ʌm") && !ipa("1000").contains("onrʉ"),
                "ta 1000 = 'āyiram': {}", ipa("1000"));
        // With a remainder → combining "āyiratti".
        assert!(ipa("1001").contains("ʌttʉ"), "ta 1001 = 'āyiratti onru': {}", ipa("1001"));
        assert!(!ipa("1001").starts_with("onrʉ"), "ta 1001 drops the leading 'one': {}", ipa("1001"));
    }
    if let Some(t) = tr("ml") {
        let ipa = |s: &str| t.text_to_ipa(s).unwrap();
        assert!(ipa("1000").ends_with("ɐm") && !ipa("1000").starts_with("onn"),
                "ml 1000 = 'āyiram': {}", ipa("1000"));
        assert!(ipa("1001").contains("ɐtti"), "ml 1001 = 'āyiratti onnu': {}", ipa("1001"));
    }
}
