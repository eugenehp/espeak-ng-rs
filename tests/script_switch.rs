// tests/script_switch.rs
//
// Foreign-script handling in a voice that doesn't read that script.
//
// Upstream splits this two ways (`alphabets[]` in `tr_languages.c`): a script
// flagged `AL_WORDS` switches translator for the whole word, everything else is
// **spelled** with the current language's letter names.  Both behaviours are
// checked here against values taken from a locally built upstream binary.

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    Translator::new(lang, None).ok()
}

/// Cyrillic is not an `AL_WORDS` script, so an English voice spells it out —
/// `мир` reads as the letter names "em i er", exactly as upstream does.
#[test]
fn cyrillic_word_is_spelled_not_switched() {
    let (Some(en), Some(_ru)) = (tr("en"), tr("ru")) else {
        eprintln!("[SKIP] no en/ru data");
        return;
    };
    let out = en.text_to_ipa("hello мир").unwrap();
    assert_eq!(out, "həlˈəʊ ˈɛm ˈɪː ˈɛr", "Cyrillic should be spelled: {out}");
    assert!(!out.contains("(ru)"), "no voice switch for a spelled script: {out}");

    // A leading Cyrillic word spells too, and the Latin word still reads.
    let out = en.text_to_ipa("Привет world").unwrap();
    assert_eq!(out, "pˈɛː ˈɛɹ ˈɪː vˈɛː jˈɛː tˈɛː wˈɜːld", "{out}");
}

/// An `AL_WORDS` script (Arabic, Devanagari, Korean, Georgian, Tamil, …) still
/// switches translator, and the switch is wrapped `(target)…(orig)`.
#[test]
fn al_words_scripts_switch_translator() {
    let Some(en) = tr("en") else {
        eprintln!("[SKIP] no en data");
        return;
    };
    for (text, target) in [("مرحبا", "(ar)"), ("नमस्ते", "(hi)"), ("안녕하세요", "(ko)")] {
        if tr(target.trim_matches(|c| c == '(' || c == ')')).is_none() {
            continue;
        }
        let out = en.text_to_ipa(text).unwrap();
        assert!(out.contains(target), "{text} should switch to {target}: {out}");
    }
}

/// A script with no letter names in this language (Han, Hebrew) can't be
/// spelled, so it falls back to the script's own voice rather than being
/// dropped — a deliberate divergence: upstream says "Chinese letter".
#[test]
fn unspellable_scripts_fall_back_to_a_voice() {
    let Some(en) = tr("en") else { return };
    if tr("cmn").is_some() {
        let out = en.text_to_ipa("你好").unwrap();
        assert!(out.contains("(cmn)"), "Han should fall back to a voice: {out}");
    }
    if tr("he").is_some() {
        let out = en.text_to_ipa("שלום").unwrap();
        assert!(out.contains("(he)"), "Hebrew should fall back to a voice: {out}");
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
