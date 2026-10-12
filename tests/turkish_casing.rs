// tests/turkish_casing.rs
//
// Turkish and Azerbaijani casing and vowel changes.
//
// Three mechanisms meet in these words:
//   * `langopts.dotless_i` — C's `towlower2` maps `I` to the *dotless* `ı` for
//     tr/az only, and `İ` to a bare `i` (the simple Unicode mapping; Rust's
//     full mapping gives `i` plus a combining dot, which broke the next rule
//     match and lost a letter);
//   * the length mark is a *flag* on the phoneme before it, not a list entry,
//     so a phoneme program's `nextPh`/`next2Ph` looks straight past it;
//   * a pronunciation given in the list is not re-reduced (`isTranslationGiven`).
//
// Values from a locally built upstream `espeak-ng` (`ba90c8e`).

use espeak_ng::Translator;

fn tr(lang: &str) -> Option<Translator> {
    let t = Translator::new(lang, Some(std::path::Path::new("espeak-ng-data"))).ok()?;
    t.text_to_ipa("a").ok()?;
    Some(t)
}

/// `I` is the dotless `ı`, `İ` is `i` — in Turkish and Azerbaijani only.
#[test]
fn dotless_i_casing() {
    if let Some(t) = tr("tr") {
        assert_eq!(t.text_to_ipa("I").unwrap(), "ˈɯ");
        assert_eq!(t.text_to_ipa("İ").unwrap(), "ˈi");
        assert_eq!(t.text_to_ipa("IŞIK").unwrap(), "ɯʃˈɯk");
        assert_eq!(t.text_to_ipa("Işık").unwrap(), "ɯʃˈɯk");
        // `İ` must not leave a combining dot behind: that lost the `s`.
        assert_eq!(t.text_to_ipa("İstanbul").unwrap(), "ɪstˈanbʊɫ");
        assert_eq!(t.text_to_ipa("TÜRKİYE").unwrap(), t.text_to_ipa("türkiye").unwrap());
    }
    if let Some(t) = tr("az") {
        assert_eq!(t.text_to_ipa("I").unwrap(), "ˈɯ");
        assert_eq!(t.text_to_ipa("İ").unwrap(), "ˈi");
        assert_eq!(t.text_to_ipa("IŞIQ").unwrap(), "ɯʃˈɯx");
    }
    // Every other language keeps Unicode's own mapping.
    if let Some(t) = tr("en") {
        assert_eq!(t.text_to_ipa("I").unwrap(), "ˈaɪ");
    }
}

/// The length mark is invisible to `nextPh`/`next2Ph`: the `e` before `kː` sees
/// the vowel after it, so it does not become `ɛ`.
#[test]
fn the_length_mark_is_not_a_neighbour() {
    let t = match tr("tr") {
        Some(t) => t,
        None => {
            eprintln!("[SKIP] no tr data");
            return;
        }
    };
    assert_eq!(t.text_to_ipa("teşekkür").unwrap(), "teʃekːˈør");
    // `o` before `k` + a vowel stays `o`; the blanket "upgrade under stress"
    // the port used to apply made it `ɔ`.
    assert_eq!(t.text_to_ipa("dokuz").unwrap(), "dˈokʊz");
}

/// A letter name comes from `tr_list` as phonemes, so it is not re-reduced.
#[test]
fn a_letter_name_keeps_its_vowel() {
    let t = match tr("tr") {
        Some(t) => t,
        None => return,
    };
    assert_eq!(t.text_to_ipa("b").unwrap(), "bˈe");
    assert_eq!(t.text_to_ipa("a b c").unwrap(), "ˈa bˈe dʒˈɛ");
}
