// tests/en_us_phoneme_table.rs
//
// English voices share `en_dict` but NOT a phoneme table: `en` is en-GB and
// `en-us` is its own table.  The curated `EN_IPA_OVERRIDES` were derived from
// `espeak-ng -v en` and used to outrank the active table's own `i_IPA_NAME`
// phondata, so every American voice spoke en-GB — "hello" came out "həlˈəʊ"
// and "over" was non-rhotic.  Now phondata from the active table wins, and the
// two entries that encode en-GB's own vowel quality (the GOAT vowel, codes
// 144/145) apply only when the `en` table is selected.

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    Translator::new(lang, None).ok()
}

#[test]
fn en_us_uses_american_vowels() {
    let Some(t) = tr("en-us") else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    // GOAT vowel: /oʊ/, not RP /əʊ/. This is the codes-144/145 scoping.
    assert_eq!(ipa("hello"), "həlˈoʊ");
    assert!(ipa("boat").contains("oʊ"), "boat: {}", ipa("boat"));
    assert!(ipa("go").contains("oʊ"), "go: {}", ipa("go"));

    // Rhotic vowels come from the en-us table's phondata (code 111 → ɚ,
    // code 154 → aɪɚ), which the en-GB overrides used to mask.
    assert!(ipa("over").ends_with('ɚ'), "over: {}", ipa("over"));
    assert!(ipa("water").ends_with('ɚ'), "water: {}", ipa("water"));
    assert!(ipa("fire").contains("aɪɚ"), "fire: {}", ipa("fire"));

    // LOT vowel: American ɑː (code 129 phondata), not RP ɒ.
    assert!(ipa("not").contains('ɑ'), "not: {}", ipa("not"));
}

#[test]
fn en_gb_is_unchanged() {
    let Some(t) = tr("en") else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();

    // Every override code's phondata in the `en` table is either identical to
    // the override or absent, so making phondata authoritative is a no-op here.
    assert_eq!(ipa("hello"), "həlˈəʊ");
    assert!(ipa("over").ends_with('ə'), "over: {}", ipa("over"));
    assert!(ipa("not").contains('ɒ'), "not: {}", ipa("not"));
    assert!(ipa("fire").contains("aɪə"), "fire: {}", ipa("fire"));
    // Syllabic consonants and the other accent-neutral overrides still apply.
    assert!(ipa("bottle").contains('l'), "bottle: {}", ipa("bottle"));
}

#[test]
fn en_gb_and_en_us_actually_differ() {
    let (Some(gb), Some(us)) = (tr("en"), tr("en-us")) else {
        eprintln!("[SKIP] no en data");
        return;
    };
    let text = "hello over, the fire is not far from the water";
    let (a, b) = (gb.text_to_ipa(text).unwrap(), us.text_to_ipa(text).unwrap());
    assert_ne!(a, b, "en and en-us must not render identically");
}
