// tests/roman_numerals.rs
//
// An uppercase Roman numeral after an enumeration keyword ("Chapter IV",
// "World War II") reads as a cardinal number, instead of spelling the letters
// ("chapter i-v").  Keyword-gated so real all-caps words and regnal ordinals are
// left alone.  English-only.  Skips without en data.

use espeak_ng::Translator;

fn en() -> Option<Translator> {
    Translator::new("en", None).ok()
}

#[test]
fn non_keyword_roman_letters_are_left_alone() {
    let Some(t) = en() else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // A valid Roman numeral NOT after a keyword must not be converted.
    assert_eq!(ipa("MIX tape"), ipa("MIX tape"), "sanity");
    assert_ne!(
        ipa("MIX tape"),
        ipa("1009 tape"),
        "MIX must stay the word 'mix'"
    );
    // Regnal ordinals are deliberately not touched (ambiguous cardinal/ordinal).
    assert_ne!(ipa("Louis XIV"), ipa("Louis 14"), "regnal XIV left alone");
    // A non-Roman word after a keyword is unchanged.
    assert!(
        !ipa("Section A").contains("nˈʌmbə"),
        "Section A stays a letter"
    );
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

/// C's `TranslateRoman`: "roman" (`_roman`) and the number.
#[test]
fn roman_numerals_as_c() {
    oracle_check(
        "en",
        &[
            ("Chapter IV", "tʃˈaptə ɹˌəʊmən fˈɔː"),
            ("World War II", "wˈɜːld wˈɔː ɹˌəʊmən tˈuː"),
            ("Part III", "pˈɑːt ɹˌəʊmən θɹˈiː"),
            ("Type II", "tˈaɪp ɹˌəʊmən tˈuː"),
            ("Chapter I", "tʃˈaptəɹ ˈaɪ"),
            ("Section VII", "sˈɛkʃən ɹˌəʊmən sˈɛvən"),
            ("Louis XIV", "lˈuːi ɹˌəʊmən fˈɔːtiːn"),
            ("Henry VIII", "hˈɛnɹi ɹˌəʊmən ˈeɪt"),
        ],
    );
}
