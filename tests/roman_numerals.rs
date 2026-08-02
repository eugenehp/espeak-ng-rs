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
fn keyword_roman_numerals_read_as_cardinals() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // "Chapter IV" reads the numeral as the spelled cardinal.
    assert_eq!(ipa("Chapter IV"), ipa("Chapter 4"), "Chapter IV → Chapter 4");
    assert_eq!(ipa("World War II"), ipa("World War 2"), "World War II → 2");
    assert_eq!(ipa("Part III"), ipa("Part 3"));
    assert_eq!(ipa("Type II"), ipa("Type 2"));
    assert_eq!(ipa("Chapter I"), ipa("Chapter 1"));
    // The numeral genuinely became a number (contains the word, not letters).
    assert!(ipa("Section VII").contains("sˈɛvən"), "Section VII → seven: {}", ipa("Section VII"));
}

#[test]
fn non_keyword_roman_letters_are_left_alone() {
    let Some(t) = en() else { return };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // A valid Roman numeral NOT after a keyword must not be converted.
    assert_eq!(ipa("MIX tape"), ipa("MIX tape"), "sanity");
    assert_ne!(ipa("MIX tape"), ipa("1009 tape"), "MIX must stay the word 'mix'");
    // Regnal ordinals are deliberately not touched (ambiguous cardinal/ordinal).
    assert_ne!(ipa("Louis XIV"), ipa("Louis 14"), "regnal XIV left alone");
    // A non-Roman word after a keyword is unchanged.
    assert!(!ipa("Section A").contains("nˈʌmbə"), "Section A stays a letter");
}
