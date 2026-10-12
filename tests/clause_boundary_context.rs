// tests/clause_boundary_context.rs
//
// In C a clause boundary *is* a pause phoneme in the phoneme list, so nothing
// looks across it: a phoneme program sees a pause as its neighbour, and
// regressive voicing resets there.  In the port a boundary is a marker, and the
// passes used to walk straight past it.
//
// Values from a locally built upstream `espeak-ng` (`ba90c8e`).

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    let t = Translator::new(lang, Some(std::path::Path::new("espeak-ng-data"))).ok()?;
    t.text_to_ipa("a").ok()?;
    Some(t)
}

/// Spanish `b` is a stop after a pause and a fricative after a vowel, so the
/// comma decides which one it is.
#[test]
fn a_phoneme_program_does_not_see_past_the_comma() {
    let es = match tr("es") {
        Some(t) => t,
        None => {
            eprintln!("[SKIP] no es data");
            return;
        }
    };
    // Across the boundary: the stop.
    assert_eq!(es.text_to_ipa("a , b").unwrap(), "ˈa\nbˈe");
    assert_eq!(es.text_to_ipa("Vamos. Bien").unwrap(), "bˈamos\nbjˈen");
    // Within one clause, after a vowel: the fricative.
    assert_eq!(es.text_to_ipa("a b").unwrap(), "a βˈe");
    assert_eq!(es.text_to_ipa("una vaca").unwrap(), "ˈuna βˈaka");
    // …and at the start of an utterance, the stop again.
    assert_eq!(es.text_to_ipa("vaca").unwrap(), "bˈaka");
}

/// Polish voicing assimilation runs backwards through a consonant cluster and
/// across word boundaries, but a *clause* boundary stops it — so a word-final
/// `g` devoices even when the next clause begins with a voiced consonant.
#[test]
fn voicing_does_not_propagate_past_a_clause_boundary() {
    let pl = match tr("pl") {
        Some(t) => t,
        None => {
            eprintln!("[SKIP] no pl data");
            return;
        }
    };
    let x = |s: &str| pl.text_to_ipa(s).unwrap();
    // Across the boundary: devoiced.
    assert!(x("flag. Zażółć").starts_with("flˈak"), "{}", x("flag. Zażółć"));
    // Within the clause it still assimilates, in both directions.
    assert!(x("flag zawżdy").starts_with("flˈaɡ"), "{}", x("flag zawżdy"));
    assert!(x("minus b").starts_with("mˈinuz"), "{}", x("minus b"));
    assert!(x("w prawo").starts_with("f"), "{}", x("w prawo"));
    // Word-final devoicing before a pause is unchanged.
    assert_eq!(x("chleb"), "xlˈɛp");
}
