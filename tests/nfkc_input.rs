// tests/nfkc_input.rs
//
// Unicode compatibility (NFKC) folding of input — upstream #2523.  Applied only
// to the characters the port would otherwise *drop*, so the ones it already
// reads well keep their better reading.

use espeak_ng::Translator;

fn x(text: &str) -> Option<String> {
    Translator::new("en", None)
        .ok()
        .map(|t| t.text_to_ipa(text).unwrap())
}

/// Folding must not change ordinary text.
#[test]
fn plain_text_is_untouched() {
    let Some(_) = x("a") else { return };
    for s in ["hello world", "1,234.5", "don't", "naïve café"] {
        assert_eq!(x(s).unwrap(), x(s).unwrap(), "{s} is not stable");
    }
    assert_eq!(x("hello world").unwrap(), "həlˈəʊ wˈɜːld");
}

/// Each case is C espeak-ng's own IPA for the text.
fn oracle_check(lang: &str, cases: &[(&str, &str)]) {
    let Some(t) = Translator::new(lang, None).ok() else {
        eprintln!("[SKIP] no {lang} data");
        return;
    };
    for &(text, want) in cases {
        assert_eq!(t.text_to_ipa(text).unwrap(), want, "{lang} {text:?}");
    }
}

/// Compatibility characters as C reads them.
#[test]
fn compatibility_characters_as_c() {
    oracle_check(
        "en",
        &[
            ("™", "tɹˈeɪd mˈɑːk"),
            ("№5", "njˈuːməɹˌəʊ fˈaɪv"),
            ("x²", "ˈɛks tˈuː"),
        ],
    );
}
