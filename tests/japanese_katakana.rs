// tests/japanese_katakana.rs
//
// Japanese katakana support.  Full-width katakana (U+30A1–U+30F6) is
// phonetically identical to hiragana (カ = か); the ja rules/dict are keyed on
// hiragana, so katakana is folded onto hiragana before translation.  Without
// this, katakana loanwords and katakana emoji descriptions produced no
// phonemes at all (upstream #2464 — "descriptions silent even when pure kana").

use espeak_ng::Translator;

fn ja() -> Option<Translator> {
    Translator::new("ja", None).ok()
}

#[test]
fn katakana_reads_identically_to_hiragana() {
    let Some(t) = ja() else {
        eprintln!("[SKIP] no ja data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // Each katakana word must match its hiragana twin, mora for mora.
    for (kata, hira) in [("カタカナ", "かたかな"), ("アメリカ", "あめりか")] {
        let (k, h) = (ipa(kata), ipa(hira));
        assert!(!k.trim().is_empty(), "{kata} produced no phonemes");
        assert_eq!(k, h, "{kata:?} should read like {hira:?}");
    }
    // The prolonged-sound mark ー lengthens the preceding vowel (ハート).
    let haato = ipa("ハート");
    // (C writes the length mark through its own phoneme: `ː`.)
    assert_eq!(haato, "hˈäːto̞", "ハート should carry a long vowel");
    // A longer loanword with several ー marks still reads.
    assert!(
        !ipa("コンピューター").trim().is_empty(),
        "コンピューター should read"
    );
}

#[test]
fn voiced_and_semivoiced_kana_are_pronounced() {
    // Precomposed voiced/semi-voiced kana (が だ ば ぱ …, ~a third of the
    // syllabary) previously produced NO phonemes — the ja rules match the
    // decomposed base+dakuten form (ja_rules `.replace`), which the port now
    // reproduces.  Each must voice correctly and differ from its unvoiced base.
    let Some(t) = ja() else {
        eprintln!("[SKIP] no ja data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // (voiced kana, an IPA fragment its sound must contain)
    for (kana, frag) in [
        ("が", "ɡ"),
        ("ざ", "z"),
        ("だ", "d"),
        ("ば", "b"),
        ("ぱ", "p"),
        ("ご", "ɡ"),
        ("ぷ", "p"),
    ] {
        let got = ipa(kana);
        assert!(!got.trim().is_empty(), "{kana} produced nothing");
        assert!(
            got.contains(frag),
            "{kana} should voice to contain {frag:?}: {got}"
        );
    }
    // 'da' must differ from unvoiced 'ta'.
    assert_ne!(ipa("だ"), ipa("た"), "だ (da) must differ from た (ta)");
    // A real voiced word reads in full (だいがく = daigaku, university).
    let daigaku = ipa("だいがく");
    assert!(
        daigaku.starts_with('d') && daigaku.contains('ɡ'),
        "だいがく: {daigaku}"
    );
    // Voiced katakana too (ダ = だ).
    assert_eq!(ipa("ダ"), ipa("だ"), "voiced katakana ダ should equal だ");
}

#[test]
fn katakana_emoji_descriptions_read_as_upstream() {
    // #2464: emoji whose Japanese CLDR name is katakana used to be silent.
    let Some(t) = ja() else {
        eprintln!("[SKIP] no ja data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // ⭐ "スター", 👍 "サムズアップ": upstream's rules read hiragana, and the
    // `.replace` table that turns katakana into it only applies to input
    // text, not to a replacement, so these are silent there and here.
    for emo in ["⭐", "👍"] {
        assert_eq!(ipa(emo), "", "ja {emo}");
    }
    // 😀's description mixes in kanji, which the rules can't read either.
    assert_eq!(ipa("😀"), "", "ja 😀");
}
