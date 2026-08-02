// tests/dotted_ordinals.rs
//
// Romance abbreviated ordinals are commonly written with a dot before the
// masculine/feminine indicator ("1.º", "2.ª" — the RAE-recommended Spanish
// form; also pt/it).  The dot used to break ordinal detection, so "3.º" read
// the cardinal "tres" instead of "tercero".  The dotted form now reads the same
// ordinal as the un-dotted "3º", and a bare "3." (no indicator) is untouched.

use espeak_ng::Translator;

#[test]
fn spanish_dotted_ordinal_matches_undotted() {
    let Some(t) = Translator::new("es", None).ok() else {
        eprintln!("[SKIP] no es data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    for (dotted, plain) in [("1.º", "1º"), ("2.ª", "2ª"), ("3.º", "3º"), ("10.º", "10º")] {
        assert_eq!(ipa(dotted), ipa(plain), "{dotted} should read like {plain}");
    }
    assert_eq!(ipa("3.º"), "teɾθˈɛɾˈo", "es 3.º = tercero");
    // In a sentence the ordinal is read (not the bare cardinal + a pause).
    assert!(ipa("el 3.º gato").contains("teɾθˈɛɾˈo"), "{}", ipa("el 3.º gato"));
}

#[test]
fn portuguese_and_italian_dotted_ordinals() {
    for (lang, s, sub) in [("pt", "1.º", "primˈeɪɾˈʊ"), ("it", "2.º", "sekˈɔndo")] {
        let Some(t) = Translator::new(lang, None).ok() else {
            eprintln!("[SKIP] no {lang} data");
            continue;
        };
        let got = t.text_to_ipa(s).unwrap();
        assert!(got.contains(sub), "{lang} {s} should be an ordinal ({sub}): {got}");
    }
}

#[test]
fn bare_dot_and_german_ordinal_are_unaffected() {
    // A bare "3." (no º/ª) stays a cardinal in Spanish, and a decimal is untouched.
    if let Some(es) = Translator::new("es", None).ok() {
        assert_eq!(es.text_to_ipa("3.").unwrap(), "tɾˈes", "bare es 3. stays cardinal");
        assert!(es.text_to_ipa("3.5").unwrap().contains("θˈinkˈo"), "es decimal unaffected");
    }
    // German uses the dot itself as the ordinal marker — "3." and "3.º" must
    // both still read "dritte" (the dot-before-indicator skip is gated off for
    // dot_marks_ordinal languages).
    if let Some(de) = Translator::new("de", None).ok() {
        assert_eq!(de.text_to_ipa("3.").unwrap(), "drˈɪtə", "de 3. = dritte");
        assert_eq!(de.text_to_ipa("3.º").unwrap(), "drˈɪtə", "de 3.º stays dritte (not empty)");
    }
}
