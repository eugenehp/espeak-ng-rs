// tests/ipa_from_codes.rs
//
// `--ipa` renders the clause's finished phoneme-code list, exactly as C's
// `GetTranslatedPhonemeString(espeakPHONEMES_IPA)` does.  Two things follow, and
// this file pins both:
//
//   * `-x` and `--ipa` describe the *same* phonemes, so a cross-word allophone
//     visible in one is visible in the other;
//   * a phoneme's IPA name comes from running its bytecode program with real
//     neighbours (`ipa <string>`), not from its mnemonic — so it can differ by
//     context, and it can differ from what the mnemonic's characters suggest.
//
// Every expectation here was taken from upstream espeak-ng (master) on the same
// input.

use espeak_ng::translate::Translator;

fn tr(lang: &str) -> Option<Translator> {
    Translator::new(lang, Some(std::path::Path::new("espeak-ng-data"))).ok()
}

/// The allophones that only a cross-word view can produce.
#[test]
fn cross_word_allophones_reach_the_ipa() {
    // Spanish `d` is the fricative `ð` after the vowel that ends the word before.
    if let Some(t) = tr("es") {
        assert_eq!(t.text_to_ipa("buenos días").unwrap(), "bwˈenos ðˈias");
        // On its own the word keeps the stop.
        assert_eq!(t.text_to_ipa("días").unwrap(), "dˈias");
    }
    // Ukrainian voicing assimilation runs backwards across the word boundary.
    if let Some(t) = tr("uk") {
        assert_eq!(t.text_to_ipa("привіт світ").unwrap(), "prɪβˈiːd̪ zβˈiːt̪");
    }
    // Portuguese `m` takes the place of the following stop.
    if let Some(t) = tr("pt") {
        assert_eq!(t.text_to_ipa("bom dia").unwrap(), "bˈo\u{303}n dˈiɐ");
    }
}

/// The IPA name is whatever the phoneme's program reaches, which the mnemonic
/// alone cannot tell you.
#[test]
fn ipa_names_come_from_the_phoneme_program() {
    // Italian `*` is a trill `r` between vowels; the mnemonic would give `ɾ`.
    if let Some(t) = tr("it") {
        assert_eq!(t.text_to_ipa("buongiorno").unwrap(), "bʊondʒˈɔrno");
    }
    // Turkish `R` is `r`, not the uvular `ʀ` its mnemonic maps to.
    if let Some(t) = tr("tr") {
        assert_eq!(t.text_to_ipa("merhaba dünya").unwrap(), "mˈɛrhaba dønjˈa");
    }
    // Spanish `i;` prints plain `i` — the mnemonic's `;` would add a `ʲ`.
    if let Some(t) = tr("es") {
        let ipa = t.text_to_ipa("días").unwrap();
        assert!(!ipa.contains('ʲ'), "no palatalisation mark: {ipa}");
    }
    // A mnemonic with no program `ipa` falls back character by character, `-`
    // included: French's reducible schwa `@-` is `ə-`.
    if let Some(t) = tr("fr") {
        assert_eq!(t.text_to_ipa("bonjour le monde").unwrap(), "bɔ̃ʒˈuʁ lə- mˈɔ̃d");
    }
}

/// A stressed vowel's allophone depends on its neighbours, not on the stress
/// alone: Russian `е` is `e` after a soft consonant.
#[test]
fn russian_stressed_vowel_is_contextual() {
    let Some(t) = tr("ru") else { return };
    assert_eq!(t.text_to_ipa("привет мир").unwrap(), "prʲivʲˈet mʲˈir");
    // …and the unstressed vowels still reduce.
    assert_eq!(t.text_to_ipa("молоко").unwrap(), "mʌɭʌkˈo");
}

/// `-x` and `--ipa` are two spellings of one phoneme list, so they must agree on
/// how many phonemes there are and where the word boundaries fall.
#[test]
fn x_and_ipa_agree_on_word_count() {
    for (lang, text) in [
        ("en", "hello world"),
        ("es", "buenos días"),
        ("de", "guten Tag"),
        ("ru", "привет мир"),
    ] {
        let Some(t) = tr(lang) else { continue };
        let ipa = t.text_to_ipa(text).unwrap();
        let codes = t.translate_to_codes(text).unwrap();
        let boundaries = codes.iter().filter(|c| c.is_boundary).count();
        assert!(boundaries > 0, "{lang}: no word boundary in the code list");
        assert_eq!(
            ipa.split_whitespace().count(),
            text.split_whitespace().count(),
            "{lang}: {ipa}"
        );
    }
}

/// The stress-conditioned phoneme changes are refused for a word the dictionary
/// list pronounces — C's `StressCondition` — so English "unseen" keeps its `ʌ`
/// while a rules-pronounced Russian word still reduces.
#[test]
fn dictionary_words_keep_their_given_vowels() {
    if let Some(t) = tr("en") {
        assert_eq!(t.text_to_ipa("unseen").unwrap(), "ʌnsˈiːn");
        // "and" is a list entry, so its `a` is not diminished away.
        assert!(t.text_to_ipa("four and five").unwrap().contains(" and "));
    }
    if let Some(t) = tr("ru") {
        // Not in ru_list: the rules pronounce it and the reduction applies.
        assert_eq!(t.text_to_ipa("молоко").unwrap(), "mʌɭʌkˈo");
    }
}

/// A spoken symbol standing tight between two words is its own word.
#[test]
fn spoken_symbols_are_separate_words() {
    let Some(t) = tr("en") else { return };
    assert_eq!(t.text_to_ipa("cat.Dog").unwrap(), "kˈat dˈɒt dˈɒɡ");
}
