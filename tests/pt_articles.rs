// The plural articles `os` and `as` are flags-only list entries (`$u+ $nounf`),
// while the singular `o`/`a` also have `$atend` pronunciations.  Removing the
// plural `s` leaves the stem `o`/`a`, whose `$atend` entry must not apply
// inside a clause: the article stays reduced and takes no primary stress.
//
// Values from a locally built upstream `espeak-ng` (`f0fb11a8`).
// Skips when the pt voice data is absent.

use espeak_ng::Translator;

fn check(lang: &str, cases: &[(&str, &str)]) {
    let Ok(t) = Translator::new(lang, None) else {
        eprintln!("[SKIP] no {lang} data");
        return;
    };
    for (text, want) in cases {
        assert_eq!(&t.text_to_ipa(text).unwrap(), want, "{lang} {text:?}");
    }
}

#[test]
fn european_plural_articles_stay_reduced() {
    check(
        "pt",
        &[
            ("os amigos", "uz ɐmˈiɡʊʃ"),
            ("os gatos", "uʒ ɡˈatʊʃ"),
            ("as casas", "ɐʃ kˈazɐʃ"),
            ("vi os livros", "vˈi uʒ lˈivɹʊʃ"),
            ("as pessoas e os amigos", "ɐʃ pˌɨsˈoɐz i uz ɐmˈiɡʊʃ"),
            ("com as casas grandes", "kˈom ɐʃ kˈazɐʒ ɡɹˈɐ̃ŋdɨʃ"),
        ],
    );
}

#[test]
fn brazilian_plural_articles_stay_reduced() {
    check(
        "pt-br",
        &[
            ("as casas", "as kˈazæs"),
            ("os amigos", "ʊz ˌæmˈiɡʊs"),
            ("as pessoas e os amigos", "as pˌesˈoæz i ʊz ˌæmˈiɡʊs"),
            ("com as casas grandes", "koŋ as kˈazæz ɡrˈɐ̃ŋdʒys"),
        ],
    );
}

#[test]
fn a_plural_article_ending_the_clause_keeps_its_stress() {
    check("pt", &[("os", "ˈʊʃ")]);
    check("pt-br", &[("os", "ˈʊs")]);
}
