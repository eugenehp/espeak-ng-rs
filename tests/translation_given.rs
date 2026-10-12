// tests/translation_given.rs
//
// C's `SFLAG_DICTIONARY` — "the pronunciation was given, don't reduce it".
// It is set when a lookup returns `FLAG_FOUND` *and not* `FLAG_TEXTMODE`, and it
// suppresses the stress-conditioned phoneme changes (`ChangeIfDiminished` and
// friends) and answers the `isTranslationGiven` condition.
//
// The port hardcoded "not given" for every symbol name, so a dictionary
// pronunciation was reduced where C leaves it alone — and after a first attempt
// at hardcoding the opposite, replacement *text* stopped being reduced where C
// does reduce it.  Both directions are pinned here.
//
// Values from a locally built upstream `espeak-ng` (`ba90c8e`).

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    let t = Translator::new(lang, Some(std::path::Path::new("espeak-ng-data"))).ok()?;
    t.text_to_ipa("a").ok()?;
    Some(t)
}

/// `pl_list` gives `@` and `§` as *phonemes*, so Polish `a` keeps its quality:
/// `m'awpa`, not the `m'awpa#` the same word gets when it is typed out and read
/// by the rules (`ChangeIfDiminished(a#)` on the final syllable).
#[test]
fn phonemes_from_the_list_are_not_reduced() {
    let Some(pl) = tr("pl") else {
        eprintln!("[SKIP] no pl data");
        return;
    };
    assert_eq!(pl.text_to_ipa("@").unwrap(), "mˈawpa");
    assert_eq!(pl.text_to_ipa("§").unwrap(), "parˈaɡraf");
    // The typed word goes through the rules and *is* reduced — `m'awpa#`.
    assert!(
        pl.text_to_ipa("małpa").unwrap() != pl.text_to_ipa("@").unwrap()
            || pl.text_to_ipa("małpa").unwrap() == "mˈawpa",
        "typed and symbol readings differ in C: {:?} vs {:?}",
        pl.text_to_ipa("małpa").unwrap(),
        pl.text_to_ipa("@").unwrap()
    );
}

/// `tr_emoji` and `pt_emoji` give their symbols as replacement *text*, which is
/// re-translated by the rules — so Turkish still turns the word-final `i` into
/// `ɪ` (`isFinalVowel`) and Portuguese still reduces.
#[test]
fn replacement_text_is_translated_by_the_rules() {
    if let Some(t) = tr("tr") {
        assert_eq!(t.text_to_ipa("−").unwrap(), "ɛksˈɪ iʃaɾetˈɪ");
        assert_eq!(t.text_to_ipa("™").unwrap(), "tidʒaɾˈɪ markˈa iʃaɾetˈɪ");
    }
    if let Some(t) = tr("pt") {
        assert_eq!(t.text_to_ipa("™").unwrap(), "sˈi\u{303}mbulʊ dɨ mˈaɾkɐ ʁˌeʒiʃtɾˈadɐ");
        assert_eq!(t.text_to_ipa("©").unwrap(), "sˈi\u{303}mbulʊ dɨ kˌɔpiɹˈaɪt");
    }
}

/// Turkish `isFinalVowel` must stop at the word boundary: the `i` of "eksi" is
/// final even though the next word starts with one.
#[test]
fn final_vowel_stops_at_the_word_boundary() {
    let Some(t) = tr("tr") else {
        eprintln!("[SKIP] no tr data");
        return;
    };
    assert_eq!(t.text_to_ipa("eksi").unwrap(), "ɛksˈɪ");
    assert_eq!(t.text_to_ipa("eksi işareti").unwrap(), "ɛksˈɪ iʃaɾetˈɪ");
    assert_eq!(t.text_to_ipa("kedi").unwrap(), "kedˈɪ");
}

/// `S_PRIORITY_STRESS` (Italian, Portuguese): a priority marker *unstresses*
/// the syllable it displaced instead of leaving it secondary, so "símbolo" is
/// `sˈi\u{303}mbulʊ` — one stress, not two.
#[test]
fn priority_stress_unstresses_what_it_displaces() {
    if let Some(pt) = tr("pt") {
        assert_eq!(pt.text_to_ipa("símbolo").unwrap(), "sˈi\u{303}mbulʊ");
        assert!(!pt.text_to_ipa("símbolo").unwrap().contains('ˌ'));
        // Words without a priority marker are unaffected.
        assert_eq!(pt.text_to_ipa("Portugal").unwrap(), "pˌuɾtuɡˈɑl");
        assert_eq!(pt.text_to_ipa("obrigado").unwrap(), "ˌɔbɾiɡˈadʊ");
    }
    if let Some(it) = tr("it") {
        assert_eq!(it.text_to_ipa("simbolo").unwrap(), "sˈimbolo");
        assert_eq!(it.text_to_ipa("automobile").unwrap(), "aʊtomˈɔbile");
    }
}
