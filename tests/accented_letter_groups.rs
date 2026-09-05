// tests/accented_letter_groups.rs
//
// Letter-group membership is tested on a letter's *unaccented base* (C's
// `IsLetter` via `remove_accent`), so a rule like `d (A` — "d before a vowel" —
// matches an accented vowel too.
//
// Without it, a Spanish word whose second letter is an accented vowel lost
// everything after the first letter (`día` → `d`), and French `s` between
// vowels didn't voice when the following vowel was accented.  Expected values
// come from a locally built upstream binary.

use espeak_ng::Translator;

fn x(lang: &str, text: &str) -> Option<String> {
    Translator::new(lang, None).ok().map(|t| t.text_to_ipa(text).unwrap())
}

#[test]
fn spanish_accented_vowel_does_not_truncate_the_word() {
    let Some(got) = x("es", "día") else {
        eprintln!("[SKIP] no es data");
        return;
    };
    assert_eq!(got, "dˈia", "an accented vowel must not end the word: {got}");
    assert_eq!(x("es", "días").unwrap(), "dˈias");
    assert_eq!(x("es", "búho").unwrap(), "bˈuo");
    // The whole phrase reads, and the `d (A → D` fricative-allophone rule fires
    // before an accented vowel — the `-x` mnemonics are `bw'enos D'i;as` and
    // `--ipa` now shows the same phonemes, because it renders from the clause's
    // code list rather than word by word.
    assert_eq!(x("es", "buenos días").unwrap(), "bwˈenos ðˈias");
}

#[test]
fn french_s_voices_between_vowels_with_an_accent() {
    let Some(got) = x("fr", "divisé") else {
        eprintln!("[SKIP] no fr data");
        return;
    };
    assert!(got.contains("divizˈe"), "intervocalic s should voice: {got}");
}

/// The base-letter lookup must not turn a *consonant* with a diacritic into a
/// vowel, or leak outside the table's range.
#[test]
fn accent_folding_is_bounded() {
    use espeak_ng::dictionary::rules::is_letter_wc;
    // A plain vowel bank: bit 7 = "vowels including y".
    let mut bits = [0u8; 256];
    bits[b'i' as usize] = 1 << 7;
    bits[b'n' as usize] = 1 << 2;

    assert!(is_letter_wc(&bits, 'í' as u32, 0, 7), "í folds to i");
    assert!(!is_letter_wc(&bits, 'ñ' as u32, 0, 7), "ñ folds to n, not a vowel");
    assert!(is_letter_wc(&bits, 'ñ' as u32, 0, 2), "ñ folds to n, a consonant");
    // Outside the table: CJK, and a character with no ASCII equivalent (×).
    assert!(!is_letter_wc(&bits, '你' as u32, 0, 7));
    assert!(!is_letter_wc(&bits, '×' as u32, 0, 7), "× has no base letter");
    // A language with a letter_bits_offset (Arabic) is unaffected.
    assert!(!is_letter_wc(&bits, 'í' as u32, 0x600, 7));
}
