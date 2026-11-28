// tests/german_ordinals.rs
//
// Ordinals as C's `TranslateNumber_1` reads them (`LookupNum2`/`LookupNum3`
// with the ordinal control bit): each case is C espeak-ng's own IPA.

use espeak_ng::Translator;

fn check(lang: &str, cases: &[(&str, &str)]) {
    let Some(t) = Translator::new(lang, None).ok() else {
        eprintln!("[SKIP] no {lang} data");
        return;
    };
    for &(text, want) in cases {
        assert_eq!(t.text_to_ipa(text).unwrap(), want, "{lang} {text:?}");
    }
}

/// German ordinals: `_ord`/`_ord20` after the whole number, and a dot that
/// marks one only before a lower-case word.
#[test]
fn german_ordinals() {
    check(
        "de",
        &[
            ("21. mai", "ˈaɪn ʊnttsvˈantsɪçstə mˈaɪ"),
            ("22. mai", "tsvˈaɪ ʊnttsvˈantsɪçstə mˈaɪ"),
            ("20. mai", "tsvˈantsɪçstə mˈaɪ"),
            ("1. mai", "ˈeːɾstə mˈaɪ"),
            ("3. mai", "dɾˈɪtə mˈaɪ"),
            ("100. mai", "ˈaɪnhˈʊndɜtstə mˈaɪ"),
            ("der 21. Mai", "dɛɾ ˈaɪn ʊnttsvˈantsɪç\nmˈaɪ"),
            ("der 21. mai", "dɛɾ ˈaɪn ʊnttsvˈantsɪçstə mˈaɪ"),
            ("1.", "ˈaɪns"),
            ("3.", "dɾˈaɪ"),
            ("21.", "ˈaɪn ʊnttsvˈantsɪç"),
            ("1", "ˈaɪns"),
            ("21", "ˈaɪn ʊnttsvˈantsɪç"),
            ("31", "ˈaɪn ʊntdɾˈaɪsɪç"),
            ("101", "ˈaɪnhˈʊndɜt ˈaɪns"),
        ],
    );
}

/// Dutch `e` ordinals (`ordinal_indicator`).
#[test]
fn dutch_ordinals() {
    check(
        "nl",
        &[
            ("1e", " ˈɪːrstə"),
            ("21e", " ˈeːnɛntʋˌɪntəxstə"),
            ("20e", "tʋˈɪntəxstə"),
            ("100e", "hˈɔndɛrtstə"),
            ("2e", "tʋˈeːdə"),
            ("3e", "dˈɛrdə"),
            ("8e", " ˈɑxtstə"),
        ],
    );
}

/// English suffix ordinals: `_1o` and the suffix's own `_#st`.
#[test]
fn english_ordinals() {
    check(
        "en",
        &[
            ("21st", "twˈɛnti fˈɜːst"),
            ("33rd", "θˈɜːti θˈɜːd"),
            ("101st", "wˈɒnhˈʌndɹɪdən fˈɜːst"),
            ("1st", "fˈɜːst"),
            ("0th", "zˈiəɹəʊθ"),
            ("1000000th", "wˈɒn mˈɪliən θ"),
            ("100th", "wˈɒnhˈʌndɹɪd θ"),
            ("2nd", "sˈɛkənd"),
            ("4th", "fˈɔːθ"),
        ],
    );
}
