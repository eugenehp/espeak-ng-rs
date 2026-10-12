// tests/clause_punctuation.rs
//
// Clause punctuation beyond the ASCII six, and the pause a bracket makes.
//
// C classifies clause punctuation by Unicode property (`clause_type_from_codepoint`),
// which is how scripts written without spaces get sentence breaks at all, and
// pauses at a bracket through `langopts.param[LOPT_BRACKET_PAUSE]`.
//
// Expectations are byte-identical to a locally built upstream `espeak-ng`
// (`ba90c8e`); one clause per line, as `--ipa` prints them.

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    Translator::new(lang, None).ok()
}

macro_rules! skip_unless {
    ($lang:expr) => {
        match tr($lang) {
            Some(t) => t,
            None => {
                eprintln!("[SKIP] no {} data", $lang);
                return;
            }
        }
    };
}

#[test]
fn non_ascii_terminators_break_the_clause() {
    let en = skip_unless!("en");
    // An em dash and an ellipsis are clause punctuation: "a" ends its clause,
    // so it takes the clause-final form.
    assert_eq!(en.text_to_ipa("a — b").unwrap(), "ˈeɪ\nbˈiː");
    assert_eq!(en.text_to_ipa("a … b").unwrap(), "ˈeɪ\nbˈiː");
    // …but only when a space, a bracket or the end of the text follows: an
    // unspaced en dash is dropped instead, as in C.
    assert_eq!(en.text_to_ipa("a–b").unwrap(), "ɐ bˈiː");
}

#[test]
fn space_less_scripts_get_their_sentence_breaks() {
    // These terminators carry `OPTIONAL_SPACE_AFTER`, so they end the clause
    // with nothing after them — the only way CJK and Indic text breaks at all.
    if let Some(hi) = tr("hi") {
        assert_eq!(hi.text_to_ipa("क । ख").unwrap(), "kˈə\nkʰˈə");
    }
    if let Some(cmn) = tr("cmn") {
        assert_eq!(cmn.text_to_ipa("你好。再见").unwrap(), "niɜχˈɑu2\ntsˈai5 tɕˈiɛ5n");
    }
    if let Some(ja) = tr("ja") {
        assert_eq!(ja.text_to_ipa("これ。あれ").unwrap(), "kˈo̞ɽe̞\nˈäɽe̞");
    }
    // Armenian full stop (U+0589).
    if let Some(hy) = tr("hy") {
        assert_eq!(hy.text_to_ipa("բարև։ ինչպես").unwrap(), "baɹˈev\nintʃʰpˈes");
    }
}

#[test]
fn brackets_pause_without_speaking() {
    let en = skip_unless!("en");
    let pauses = |s: &str| {
        en.translate_to_codes(s)
            .expect("codes")
            .iter()
            .filter(|c| c.code == espeak_ng::phoneme::PHON_PAUSE)
            .count()
    };
    // `LOPT_BRACKET_PAUSE` is 4, which C's pre-pause loop spends as two pauses.
    assert_eq!(pauses("a ( b"), pauses("a b") + 2);
    assert_eq!(pauses("a {b} c"), pauses("a b c") + 4);
    // C builds the pause as the *next* word's pre-pause, so a closing bracket
    // with nothing after it in the clause adds none.
    assert_eq!(pauses("(hello)"), pauses("hello") + 2);
    // Nothing is spoken for the bracket itself.
    assert_eq!(en.text_to_ipa("a (b) c").unwrap(), en.text_to_ipa("a b c").unwrap());
}

#[test]
fn a_voice_can_turn_the_bracket_pause_off() {
    // Hungarian's voice file sets `brackets 0` / `bracketsAnnounced 0`.
    let hu = skip_unless!("hu");
    let pauses = |s: &str| {
        hu.translate_to_codes(s)
            .expect("codes")
            .iter()
            .filter(|c| c.code == espeak_ng::phoneme::PHON_PAUSE)
            .count()
    };
    assert_eq!(pauses("a ( b"), pauses("a b"), "hu asks for no bracket pause");
}

#[test]
fn a_bracket_closes_an_open_language_switch() {
    // The pause belongs to the base language, so C closes the switch before it
    // and opens a new one after: `(en)ˈeɪ(ru) (en)bˈiː(ru)`.
    let Some(ru) = tr("ru") else { eprintln!("[SKIP] no ru data"); return };
    assert_eq!(ru.text_to_ipa("a ( b").unwrap(), "(en)ˈeɪ(ru) (en)bˈiː(ru)");
}
