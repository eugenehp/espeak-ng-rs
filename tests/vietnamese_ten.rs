// tests/vietnamese_ten.rs
//
// Vietnamese "ten" (#2442).  Vietnamese has no dedicated `_10` number word — it
// uses the `_1X` ten-prefix "mười".  The teens path only composed ten+unit for
// tensunits > 10, so *exactly* 10 fell through and produced no output at all.
// Now 10 uses the ten-prefix when `_10` is absent.

use espeak_ng::Translator;

#[test]
fn vietnamese_ten_reads() {
    let Some(t) = Translator::new("vi", None).ok() else {
        eprintln!("[SKIP] no vi data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // 10 = "mười" (was empty).
    let ten = ipa("10");
    assert!(!ten.trim().is_empty(), "vi 10 produced no output");
    // It's the same "mười" that begins the teens (15 = mười + …).
    assert!(ipa("15").starts_with(&ten), "vi 15 should start with the 10 word: {}", ipa("15"));
    // …and appears inside larger numbers (110 = một trăm mười).
    assert!(ipa("110").ends_with(&ten), "vi 110 should end with the 10 word: {}", ipa("110"));
}

#[test]
fn other_languages_ten_unaffected() {
    for (lang, want) in [("en", "tˈɛn"), ("de", "tsˈeːn"), ("tr", "ˈon")] {
        let Some(t) = Translator::new(lang, None).ok() else { continue };
        assert_eq!(t.text_to_ipa("10").unwrap(), want, "{lang} 10");
    }
}
