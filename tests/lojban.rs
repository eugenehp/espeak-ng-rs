// tests/lojban.rs
//
// Lojban (jbo) capital-stress notation — espeak `LOPT_CAPS_IN_WORD`.  Lojban
// writes the stressed syllable with capital letters; espeak turns the first
// mid-word capital into a primary-stress mark `ˈ` and keeps the word whole
// (it does NOT split, unlike the general CamelCase rule — see camelcase.rs).
// The explicit mark must also suppress the default penultimate-stress rule, so
// exactly one primary stress lands on the capitalised syllable.

use espeak_ng::Translator;

fn jbo() -> Option<Translator> {
    Translator::new("jbo", None).ok()
}

fn primary_marks(s: &str) -> usize {
    s.matches('\u{02c8}').count()
}

#[test]
fn capital_marks_stress_and_keeps_word_whole() {
    let Some(t) = jbo() else {
        eprintln!("[SKIP] no jbo data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    // A capital pins primary stress to that syllable, and the word is NOT split.
    assert_eq!(ipa("spAgeti"), "spˈaɡetˌi"); // stress the 1st syllable
    assert_eq!(ipa("spagetI"), "spˌaɡetˈi"); // stress the last syllable
    assert_eq!(ipa("bAstu"), "bˈastu");
    // Leading capital also marks stress (still one word).
    assert_eq!(ipa("Bastu"), "bˈastu");

    for w in ["spAgeti", "spagetI", "bAstu", "Bastu"] {
        let got = ipa(w);
        assert!(!got.contains(' '), "jbo {w:?} must stay one word: {got:?}");
        // Exactly one PRIMARY stress — the explicit mark suppresses the default
        // penultimate rule (previously this double-stressed, e.g. "spˈaɡˈeti").
        assert_eq!(primary_marks(&got), 1, "jbo {w:?} one primary stress: {got:?}");
    }
}

#[test]
fn capital_moves_stress_off_the_default_position() {
    let Some(t) = jbo() else {
        eprintln!("[SKIP] no jbo data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // Default (no caps) is penultimate; a non-penultimate capital must change it.
    let default = ipa("spageti");
    assert_ne!(ipa("spAgeti"), default, "1st-syllable capital should move stress");
    assert_ne!(ipa("spagetI"), default, "final-syllable capital should move stress");
}
