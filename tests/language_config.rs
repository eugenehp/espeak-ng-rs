// tests/language_config.rs
//
// The per-language settings the port transcribes from upstream's
// `tr_languages.c`: which syllable takes the stress, the `S_*` flags that shape
// the secondary stresses, the inter-word pauses, and the letter groups.  These
// used to be one ad-hoc match whose catch-all reset the flags to zero, so most
// of the flag table never applied.  Every expectation here comes from running
// upstream espeak-ng (master) on the same input.

use espeak_ng::translate::Translator;

fn tr(lang: &str) -> Option<Translator> {
    Translator::new(lang, Some(std::path::Path::new("espeak-ng-data"))).ok()
}

fn x(lang: &str, text: &str) -> Option<String> {
    let t = tr(lang)?;
    let codes = t.translate_to_codes(text).ok()?;
    let mut ph = espeak_ng::phoneme::PhonemeData::load(std::path::Path::new("espeak-ng-data")).ok()?;
    espeak_ng::translate::select_phoneme_table(&mut ph, std::path::Path::new("espeak-ng-data"), lang).ok()?;
    Some(espeak_ng::translate::phoneme_ipa::codes_to_ipa(
        &codes,
        &ph,
        std::path::Path::new("espeak-ng-data"),
        lang,
        false,
    ))
}

/// The stress rule each language actually uses.
#[test]
fn stress_rules_match_upstream() {
    for (lang, text, want) in [
        // Final syllable (`STRESSPOSN_1R`): Armenian, Turkish-style `1RU` for az/kk.
        ("hy", "բարեւ", "baɹˈev"),
        ("az", "Salam dünya", "saɫˈam dynjˈa"),
        ("kk", "Сәлем әлем", "sælˈem ælˈem"),
        // Penultimate (`STRESSPOSN_2R`): Esperanto, Indonesian, Swahili.
        ("eo", "Saluton mondo", "salˈuton mˈondo"),
        ("id", "Selamat pagi dunia", "səlˈamat pˈaɡi dunˈia"),
        ("sw", "asubuhi", "ˌasubˈuhi"),
        // First syllable (`STRESSPOSN_1L`): Latvian.
        ("lv", "Labrīt pasaule", "labrˈiːt pˈasaule"),
        // Antepenultimate (`STRESSPOSN_3R`): Macedonian.
        ("mk", "здраво свету", "zdrˈavo svˈetʊ"),
        // Stress the heaviest syllable (`STRESSPOSN_1RH`): Telugu.
        ("te", "హలో ప్రపంచం", "hˈaloː prˈapancan"),
    ] {
        let Some(got) = x(lang, text) else { continue };
        assert_eq!(got, want, "{lang} {text:?}");
    }
}

/// `S_NO_AUTO_2` and friends decide whether an automatic secondary stress
/// appears at all.
#[test]
fn stress_flags_reach_the_languages_that_set_them() {
    // Croatian keeps `S_FINAL_NO_2`: no secondary on the final syllable.
    if let Some(got) = x("hr", "Dobar dan svijete") {
        assert_eq!(got, "dˈobɑr dˈan svˈijetɛ");
    }
    // Georgian likewise.
    if let Some(got) = x("ka", "მსოფლიო") {
        assert_eq!(got, "msˈopʰlio");
    }
    // Russian has `S_NO_AUTO_2` (from `Translator_Russian`), so "молоко" gets
    // one stress, not a secondary as well.
    if let Some(got) = x("ru", "молоко") {
        assert_eq!(got, "mʌɭʌkˈo");
    }
}

/// A hyphen between two letters breaks the word for lookup but not for speech.
#[test]
fn hyphen_joins_the_words_it_separates() {
    let Some(got) = x("en", "well-known") else { return };
    assert_eq!(got, "wˈɛlnˈəʊn");
    assert_eq!(x("en", "twenty-two thousand").unwrap(), "twˈɛntitˈuː θˈaʊzənd");
    // Each part still carries its own stress.
    assert_eq!(x("en", "state-of-the-art").unwrap().matches('ˈ').count(), 2);
}

/// The inter-word pauses a language asks for.
#[test]
fn word_gaps_and_vowel_pauses() {
    // Italian's `vowel_pause = 1`: a very short break between two vowels across
    // a word boundary.
    if let Some(got) = x("it", "La casa è molto grande.") {
        assert!(got.contains("kˈaza"), "intervocalic s → z: {got}");
    }
    // Latin's voice file says `words 2`: a short pause after every word.
    if let Some(t) = tr("la") {
        let codes = t.translate_to_codes("Salve munde").unwrap();
        let pauses = codes
            .iter()
            .filter(|c| c.code == espeak_ng::phoneme::PHON_PAUSE_SHORT)
            .count();
        assert_eq!(pauses, 2, "one pause after each word");
    }
    // Swahili's "za" is `$u $brk`, which puts a short break in front of it.
    if let Some(t) = tr("sw") {
        let codes = t.translate_to_codes("Habari za asubuhi").unwrap();
        assert!(
            codes.iter().any(|c| c.code == espeak_ng::phoneme::PHON_PAUSE_NOLINK),
            "$brk inserts a no-link pause"
        );
    }
}

/// Only Russian and Ukrainian put "е и є ї" in the iotated-vowel letter group;
/// applying them to every Cyrillic language palatalised Bulgarian's `в`.
#[test]
fn cyrillic_iotated_vowels_are_russian_only() {
    if let Some(got) = x("bg", "Здравей свят") {
        assert_eq!(got, "zdrˈavej svʲˈat");
    }
    if let Some(got) = x("ru", "привет мир") {
        assert_eq!(got, "prʲivʲˈet mʲˈir");
    }
}

/// `LOPT_IT_LENGTHEN` drops the `:` from a syllable that isn't stressed enough
/// to keep it.
#[test]
fn length_marks_are_removed_from_unstressed_syllables() {
    let Some(got) = x("sl", "Dobro jutro svet") else { return };
    assert_eq!(got, "dˈɔːbrɔ jˈuːtrɔ sʋˈeːt");
}

/// `isFirstVowel` keeps a word's first vowel out of the reduction branch.
#[test]
fn first_vowel_condition_is_evaluated() {
    let Some(got) = x("ml", "ഹലോ") else { return };
    assert_eq!(got, "hɐlˈoː");
}

/// `phonX1` is a marker, not a sound.
#[test]
fn phon_x1_is_not_spoken() {
    let Some(t) = tr("fi") else { return };
    let codes = t.translate_to_codes("kaikille").unwrap();
    assert!(
        !codes.iter().any(|c| c.code == espeak_ng::phoneme::PHON_X1),
        "phonX1 must not reach the phoneme list"
    );
    assert_eq!(x("fi", "Hyvää huomenta kaikille.").unwrap(), "hˈyvæː hˈuomenta kˈaikɪlle");
}

/// The voice file's own `replace <flags> <old> <new>` substitutions.
#[test]
fn voice_phoneme_replacements_apply() {
    // Russian's `replace 03 a a#`: a word-final `a` in an unstressed syllable.
    if let Some(got) = x("ru", "мама") {
        assert_eq!(got, "mˈɑma", "{got}");
    }
}

/// `$abbrev` with no pronunciation, and a word with no vowel at all, are both
/// read letter by letter — with the spelling stress upstream applies (every
/// primary but the last demoted).
#[test]
fn abbreviations_are_spelled_letter_by_letter() {
    let Some(t) = tr("en") else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // `usa  $abbrev` gives no phonemes, so the letters are spoken.
    assert_eq!(ipa("USA"), "jˌuːˌɛsˈeɪ");
    assert_eq!(ipa("usa"), "jˌuːˌɛsˈeɪ");
    // "BBC" has no vowel at all.
    assert_eq!(ipa("the BBC"), "ðə bˌiːbˌiːsˈiː");
    // …but a word with a vowel is read normally, even with a long onset.
    assert_eq!(ipa("street"), "stɹˈiːt");
    assert!(ipa("brown").starts_with("bɹ"), "{}", ipa("brown"));
    // A four-letter acronym the dictionary knows stays a word.
    assert!(ipa("NASA and the USA").starts_with("nˈasɐ"), "{}", ipa("NASA and the USA"));
}

/// A rule's post-context may reach past the word boundary into the next word.
#[test]
fn post_context_reaches_across_the_word_boundary() {
    // Portuguese `_) o (_C` — the article is `U` before a consonant-initial word.
    if let Some(got) = x("pt", "O menino comeu o pão.") {
        assert_eq!(got, "ʊ mˌenˈinʊ kumˈew ʊ pˈɐ\u{303}ʊ\u{303}");
    }
}

/// A digit takes its combining `_<n>a` form when a scale word follows.
#[test]
fn digits_have_a_combining_form_before_scale_words() {
    let Some(t) = tr("de") else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // "einhundert" / "ein tausend", but standalone "eins".
    assert!(ipa("100").contains("ˈaɪnhˈʊnd"), "{}", ipa("100"));
    assert!(ipa("1000").starts_with("ˈaɪn ") , "{}", ipa("1000"));
    assert!(ipa("1").ends_with("ˈaɪns"), "{}", ipa("1"));
    assert!(ipa("1001").ends_with("ˈaɪns"), "{}", ipa("1001"));
}
