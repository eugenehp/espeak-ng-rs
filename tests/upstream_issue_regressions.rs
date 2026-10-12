// tests/upstream_issue_regressions.rs
//
// Behaviour this port must keep, pinned against open upstream issues.  Three of
// these are upstream bugs the port does not have — they are easy to *acquire*
// while refactoring the same code paths, so they are nailed down here.
//
// Values checked against a locally built upstream `espeak-ng` (`ba90c8e`).

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

/// Upstream #2551: a one-letter word repeated more than three times loses every
/// repeat after the third, because `FLAG_MAX3` is applied to one-character
/// entries that are letters, not just to symbols.  C reads three; we read six.
#[test]
fn repeated_one_letter_words_all_speak() {
    let en = skip_unless!("en-us");
    assert_eq!(en.text_to_ipa("b b b b b b").unwrap().split_whitespace().count(), 6);
    assert_eq!(en.text_to_ipa("B b B b B b").unwrap().split_whitespace().count(), 6);
    assert_eq!(en.text_to_ipa("I I I I I I").unwrap().split_whitespace().count(), 6);
}

/// Upstream #2552 draws the line the other way for *symbols*: `$max3` keeps a
/// repeated symbol to three, and a word in between starts the run again.
#[test]
fn repeated_symbols_stop_at_three() {
    let en = skip_unless!("en");
    let count = |s: &str, needle: &str| en.text_to_ipa(s).unwrap().matches(needle).count();
    assert_eq!(count("~ ~ ~ ~ ~ ~", "tˈɪld"), 3);
    assert_eq!(count("% % % % % %", "pəsˈɛnt"), 3);
    assert_eq!(count("~ ~", "tˈɪld"), 2);
    // "a" ends the run, so the tildes after it start counting again.
    assert_eq!(count("~ a ~ ~ ~ ~", "tˈɪld"), 4);
}

/// Upstream #2577: Russian decimal fractions with six or more places read only
/// the last four digits.  The whole fraction must be spoken.
#[test]
fn long_russian_fractions_read_in_full() {
    let ru = skip_unless!("ru");
    let ipa = ru.text_to_ipa("0,123456").unwrap();
    // "ноль и сто двадцать три тысячи четыреста пятьдесят шесть миллионных":
    // C drops everything before the last four digits ("…и три тысячи…").
    assert!(ipa.contains("stˈo dvˈɑtsatʲ"), "the leading digits are spoken: {ipa}");
}

/// Upstream #2437 / #2438: Pashto "5" read as "carrot" and Nepali digits read in
/// English.  Both languages have their own number words and must use them.
#[test]
fn digits_use_the_languages_own_words() {
    if let Some(ps) = tr("ps") {
        assert_eq!(ps.text_to_ipa("5").unwrap(), "pˈəndzə"); // "پنځه", not "ګازره"
    }
    if let Some(ne) = tr("ne") {
        let five = ne.text_to_ipa("5").unwrap();
        assert!(!five.contains("(en)"), "ne 5 should not switch to English: {five}");
    }
}

/// Upstream #2555: an unrecognised letter must not take the rest of the word
/// with it.  Russian reads the Serbian `његов` as "egof", skipping the `њ` its
/// rules don't cover — and a switching voice must not leak the `phonSWITCH`
/// marker when the switched language comes back empty.
#[test]
fn an_unrecognised_letter_does_not_drop_the_word() {
    let ru = skip_unless!("ru");
    assert_eq!(ru.text_to_ipa("његов").unwrap(), "ˈeɡʌf");
    assert_eq!(ru.text_to_ipa("ћирилица").unwrap(), "tʃʲirʲˈiɭʲitsa");
    if let Some(en) = tr("en") {
        let got = en.text_to_ipa("његов").unwrap();
        assert!(!got.contains('\u{15}'), "raw phonSWITCH leaked: {got:?}");
        assert!(!got.trim().is_empty(), "the word was dropped: {got:?}");
    }
}

/// Upstream #2469: an unknown voice crashes C after an unhelpful message.  Here
/// it is a clean `voice not found`, not "not yet implemented".
#[test]
fn an_unknown_voice_reports_a_voice_error() {
    let Some(t) = tr("alex") else { return };
    match t.text_to_ipa("example") {
        Ok(out) => panic!("expected an error for an unknown voice, got {out:?}"),
        Err(e) => {
            let msg = e.to_string();
            assert!(msg.contains("voice not found"), "unhelpful error: {msg}");
        }
    }
}
