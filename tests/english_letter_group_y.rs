// tests/english_letter_group_y.rs
//
// English redefines letter group `Y` as "vowels, including y" (`tr_languages.c`,
// `SetLetterBits(tr, 6, "aeiouy")`); other Latin languages keep the front
// vowels `eiy`.  Rules such as `_) x (Y  z` and `_) kn (Y  n` need every vowel,
// so word-initial `x` read `ks` and `kn` read `kn` before `a`, `o` and `u`.
//
// Values from a locally built upstream `espeak-ng` 1.52.0.

use espeak_ng::Translator;

fn en() -> Option<Translator> {
    let t = Translator::new("en", Some(std::path::Path::new("espeak-ng-data"))).ok()?;
    t.text_to_ipa("a").ok()?;
    Some(t)
}

fn check(cases: &[(&str, &str)]) {
    let Some(t) = en() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    for (word, want) in cases {
        let got = t.text_to_ipa(word).unwrap_or_default();
        assert_eq!(&got, want, "word {word:?}");
    }
}

#[test]
fn word_initial_x_before_a_vowel_is_z() {
    check(&[
        ("xa", "zˈɑː"),
        ("xoab", "zˈəʊab"),
        ("xaiph", "zˈeɪf"),
        ("xu", "zˈuː"),
        ("xai", "zˈaɪ"),
    ]);
}

#[test]
fn initial_x_before_a_consonant_stays_ks() {
    check(&[("xb", "ˌɛksbˈiː")]);
}

#[test]
fn other_rules_using_group_y_match_back_vowels() {
    check(&[("kna", "nˈɑː"), ("knoe", "nˈəʊ"), ("phoa", "fˈəʊə")]);
}
