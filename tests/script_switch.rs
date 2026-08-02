// tests/script_switch.rs
//
// Automatic script-based voice switching (espeak #2436/#2479): a Cyrillic word
// inside a Latin-script clause is re-translated with the Russian voice instead
// of being dropped.  Uses the same `phonSWITCH` mechanism as `_^_LL` dict
// entries, so the switched text is wrapped `(ru)…(orig)`.  Skips without data.

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    Translator::new(lang, None).ok()
}

#[test]
fn cyrillic_word_switches_to_russian() {
    let (Some(en), Some(_ru)) = (tr("en"), tr("ru")) else {
        eprintln!("[SKIP] no en/ru data");
        return;
    };
    // The Cyrillic word is spoken (via ru), not dropped: output carries "(ru)".
    let out = en.text_to_ipa("hello мир").unwrap();
    assert!(out.contains("(ru)"), "Cyrillic word should switch to ru: {out}");
    assert!(out.contains("həlˈəʊ"), "the English word still reads: {out}");
    // A leading Cyrillic word works too.
    assert!(en.text_to_ipa("Привет world").unwrap().contains("(ru)"));
    // A Greek word switches to el.
    if tr("el").is_some() {
        let g = en.text_to_ipa("the word Αθήνα").unwrap();
        assert!(g.contains("(el)"), "Greek word should switch to el: {g}");
    }
}

#[test]
fn native_and_latin_scripts_do_not_switch() {
    // A Cyrillic voice reads Cyrillic itself — no switch marker.
    if let Some(ru) = tr("ru") {
        assert!(!ru.text_to_ipa("мир").unwrap().contains("(ru)"), "ru reads Cyrillic natively");
    }
    // Latin text in a Latin voice is untouched (no switch).
    if let Some(en) = tr("en") {
        assert!(!en.text_to_ipa("café").unwrap().contains("(ru)"), "Latin word not switched");
    }
}
