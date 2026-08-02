// tests/portuguese_numbers.rs
//
// Portuguese cardinals are generated as text and pronounced by the letter-to-
// sound rules, because the shipped pt dictionary is missing most number keys
// (`1`/`5`/`100`/`1000` used to come out empty or wrong — e.g. `2000` → "dois").
// A generated cardinal must read identically to typing that number word.
// Skips when the pt voice data is absent.

use espeak_ng::Translator;

fn pt() -> Option<Translator> {
    Translator::new("pt", None).ok()
}

#[test]
fn portuguese_cardinals_read_as_words() {
    let Some(t) = pt() else {
        eprintln!("[SKIP] no pt data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // Each written number reads identically to the spelled-out word.
    let cases: &[(&str, &str)] = &[
        ("1", "um"),
        ("5", "cinco"),
        ("15", "quinze"),
        ("100", "cem"),
        ("101", "cento e um"),
        ("200", "duzentos"),
        ("234", "duzentos e trinta e quatro"),
        ("1000", "mil"),
        ("1500", "mil e quinhentos"),
        ("2000", "dois mil"),
        ("2024", "dois mil e vinte e quatro"),
        ("100000", "cem mil"),
        ("1000000", "um milhão"),
        ("5000000", "cinco milhões"),
    ];
    for (digit, word) in cases {
        assert_eq!(ipa(digit), ipa(word), "pt {digit:?} should read as {word:?}");
    }
    // Previously-empty values now produce sound.
    for n in ["1", "5", "1000"] {
        assert!(!ipa(n).trim().is_empty(), "pt {n} should not be empty");
    }
    // In-sentence.
    assert_eq!(ipa("tenho 5 anos"), ipa("tenho cinco anos"));
}

#[test]
fn portuguese_decimals_read_with_virgula() {
    let Some(t) = pt() else {
        eprintln!("[SKIP] no pt data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // Comma is the decimal separator; the fraction is read digit by digit.
    let cases: &[(&str, &str)] = &[
        ("1,5", "um vírgula cinco"),
        ("3,14", "três vírgula um quatro"),
        ("0,5", "zero vírgula cinco"),
        ("2,50", "dois vírgula cinco zero"),
        ("100,99", "cem vírgula nove nove"),
    ];
    for (digit, word) in cases {
        assert_eq!(ipa(digit), ipa(word), "pt {digit:?} should read as {word:?}");
    }
    // Period now groups thousands (Romance convention): 1.000 == mil.
    assert_eq!(ipa("1.000"), ipa("mil"), "pt 1.000 → mil");
    assert_eq!(ipa("custa 1,5 euros"), ipa("custa um vírgula cinco euros"));
}
