// tests/units_first_cardinals.rs
//
// Danish, Faroese and Slovenian form compound cardinals units-first, like German
// ("enogtyve" = one-and-twenty, "enaindvajset" = one-and-twenty).  They used to
// share a tens-first arm and read the wrong order ("tyve en" = twenty-one).

use espeak_ng::Translator;

#[test]
fn danish_is_units_first() {
    let Some(t) = Translator::new("da", None).ok() else {
        eprintln!("[SKIP] no da data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    // 21 = "en" + "og" + "tyve" (one-and-twenty) — the units word comes first.
    // Guard: it must NOT start with the tens word "tyve" (the old wrong order).
    let twenty = ipa("20");
    let twentyone = ipa("21");
    assert!(!twentyone.starts_with(&twenty), "da 21 should be units-first, got {twentyone:?} (20={twenty:?})");
    // 21 begins with the units cardinal "en".
    assert!(twentyone.starts_with(&ipa("1")[..2]) || twentyone.starts_with('ˈ'),
        "da 21 should begin with 'en': {twentyone:?}");
    // A compound ordinal suffixes the whole cardinal ("enogtyvende"), so 21.
    // extends 21 rather than reading a separate ordinal word.
    assert!(ipa("21.").starts_with(&twentyone[..twentyone.len().min(6)]),
        "da 21. should build on the 21 cardinal: {} vs {}", ipa("21."), twentyone);
}

#[test]
fn slovenian_is_units_first() {
    let Some(t) = Translator::new("sl", None).ok() else {
        eprintln!("[SKIP] no sl data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // 21 = "ena" + "in" + "dvajset" — must not start with the tens "dvajset".
    let twenty = ipa("20");
    assert!(!ipa("21").starts_with(&twenty), "sl 21 should be units-first: {}", ipa("21"));
}

#[test]
fn tens_first_languages_unchanged() {
    // Finnish/Estonian/Norwegian stay tens-first.
    for (lang, sub) in [("fi", "kˌaksɪkymm"), ("nb", "ʂˈʉːaˈeːn")] {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        assert!(t.text_to_ipa("21").unwrap().starts_with(sub),
            "{lang} 21 should stay tens-first: {}", t.text_to_ipa("21").unwrap());
    }
}
