// English redefines letter group Y as "aeiouy" (tr_languages.c, SetLetterBits(tr, 6, ...)); other Latin languages keep "eiy". Rules like "_) x (Y" and "_) kn (Y" depend on it.
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
