// tests/readme_examples.rs
//
// The `text_to_ipa` examples in README.md and the `text_to_ipa` doc comment
// are `no_run`, so nothing checked them: the French one said "bɔ̃ʒˈuːɹ" (an
// English-style long u and approximant r) while the port prints "bɔ̃ʒˈuʁ".
// This pins the documented outputs to the real ones.

use espeak_ng::translate::Translator;

fn tr(lang: &str) -> Option<Translator> {
    Translator::new(lang, Some(std::path::Path::new("espeak-ng-data"))).ok()
}

#[test]
fn documented_ipa_examples_are_accurate() {
    let cases = [
        ("en", "hello world", "həlˈəʊ wˈɜːld"),
        ("en", "42", "fˈɔːti tˈuː"),
        ("en", "walked", "wˈɔːkt"),
        ("en", "happily", "hˈapɪli"),
        ("de", "schön", "ʃˈøːn"),
        ("fr", "bonjour", "bɔ̃ʒˈuʁ"),
    ];
    for (lang, text, expected) in cases {
        let Some(t) = tr(lang) else { continue };
        assert_eq!(t.text_to_ipa(text).unwrap(), expected, "{lang} {text:?}");
    }
}
