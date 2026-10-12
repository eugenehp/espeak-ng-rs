// tests/portuguese_numbers.rs
//
// Portuguese cardinals come from the dictionary's `_N` entries, which are
// behind `?1` (European) / `?2` (Brazilian) conditions — they looked missing
// until the port honoured those, and were generated as text instead.  Reading
// them from the list is not the same as reading the spelled-out word, and
// upstream has the same split: `pt "1"` is `ˈum` (list phonemes) while `pt
// "um"` is `ˈũm` (the letter-to-sound rules).
//
// Values from a locally built upstream `espeak-ng` (`ba90c8e`).
// Skips when the pt voice data is absent.

use espeak_ng::Translator;

fn pt() -> Option<Translator> {
    Translator::new("pt", None).ok()
}

#[test]
fn portuguese_cardinals_come_from_the_list() {
    let Some(t) = pt() else {
        eprintln!("[SKIP] no pt data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    let cases: &[(&str, &str)] = &[
        ("1", "ˈum"),
        ("5", "sˈĩŋku"),
        ("15", "kˈĩnzɨ"),
        ("100", "sˈeɪŋ"),
        ("200", "dˌuzˈẽntuʃ"),
        // The conjunction joins hundreds→remainder and tens→units
        // (`NUM_HUNDRED_AND | NUM_AND_UNITS`).
        ("234", "dˌuzˈẽntuz i tɾˈĩntɐ ikwˈatɾu"),
        ("1000", "mˈil"),
        ("1500", "mˈil kiɲˈẽntuʃ"),
        ("2000", "dˈoɪʒ mˈil"),
        ("2024", "dˈoɪʒ mˈil i vˈĩntɨ ikwˈatɾu"),
        // …and after a scale word when the remainder is below a hundred:
        // "mil E um".
        ("1001", "mˈil i ˈum"),
        ("100000", "sˈeɪn mˈil"),
        ("1000000", "ˈũn miʎˈɐ̃ʊ̃"),
        ("5000000", "sˈĩŋku miʎˈõɪ̃ʃ"),
    ];
    for (digits, want) in cases {
        assert_eq!(&ipa(digits), want, "pt {digits:?}");
    }
    assert_eq!(ipa("tenho 5 anos"), "tˌeɲʊ sˈĩŋku ˈɐ̃nʊʃ");
    // The list reading and the typed word differ, in C as here.
    assert_ne!(ipa("1"), ipa("um"));
}

#[test]
fn portuguese_decimals_read_with_virgula() {
    let Some(t) = pt() else {
        eprintln!("[SKIP] no pt data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // Comma is the decimal separator, and a fraction of at most two digits is
    // read as one number (upstream `NUM_DFRACTION_2`): "3,14" is "três vírgula
    // catorze", not "… um quatro".  A longer fraction falls back to digit by
    // digit.
    let cases: &[(&str, &str)] = &[
        ("1,5", "ˈun vˈiɾɡulɐ sˈĩŋku"),
        ("3,14", "tɾˈeʒ vˈiɾɡulɐ kɐtˈoɾzɨ"),
        ("0,5", "zˈɛɾu vˈiɾɡulɐ sˈĩŋku"),
        ("2,50", "dˈoɪʒ vˈiɾɡulɐ sĩŋkwˈẽntɐ"),
        ("100,99", "sˈeɪn vˈiɾɡulɐ nuvˈẽntɐ inˈɔvɨ"),
        ("1,234", "ˈun vˈiɾɡulɐ dˈoɪʃ tɾˈeʃ kwˈatɾu"),
    ];
    for (digits, want) in cases {
        assert_eq!(&ipa(digits), want, "pt {digits:?}");
    }
    // A leading zero in the fraction is spoken on its own.
    assert!(ipa("0,05").starts_with("zˈɛɾu vˈiɾɡulɐ"));
    assert!(ipa("0,05").ends_with("zˈɛɾu sˈĩŋku"));
    // Period groups thousands (Romance convention): 1.000 == mil.
    assert_eq!(ipa("1.000"), ipa("1000"));
    assert_eq!(ipa("custa 1,5 euros"), "kˈuʃtɐ ˈun vˈiɾɡulɐ sˈĩŋku ˈeʊɾʊʃ");
}
