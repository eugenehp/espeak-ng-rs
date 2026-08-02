// tests/inline_phonemes.rs
//
// `[[ … ]]` inline phoneme input.
//
// espeak reads phonemes given directly in double square brackets (using the
// phoneme *mnemonics* from the current phoneme table) and emits them verbatim,
// bypassing the dictionary.  This is an always-on feature (no `-m` needed).
//
// Clean single/simple-mnemonic cases are asserted against exact values that
// were confirmed identical to the C oracle; the more complex cases only check
// robust invariants (no bracket leakage, stress rendered) because per-phoneme
// IPA rendering inherits pre-existing renderer differences.

use espeak_ng::Translator;

fn en() -> Option<Translator> {
    Translator::new("en", None).ok()
}

#[test]
fn single_mnemonics_match_oracle() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    assert_eq!(t.text_to_ipa("[[k]]").unwrap(), "k");
    assert_eq!(t.text_to_ipa("[[t]]").unwrap(), "t");
    assert_eq!(t.text_to_ipa("[[s]]").unwrap(), "s");
}

#[test]
fn inline_phonemes_embed_in_a_sentence() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    // Confirmed byte-identical to `espeak-ng -q --ipa -v en "say [[k]] now"`.
    assert_eq!(t.text_to_ipa("say [[k]] now").unwrap(), "sˈeɪ k nˈaʊ");
}

#[test]
fn brackets_and_letters_never_leak() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    let out = t.text_to_ipa("[[h@l3U]]").unwrap();
    assert!(!out.is_empty(), "inline phonemes produced no output");
    assert!(!out.contains('['), "bracket leaked: {out:?}");
    assert!(!out.contains(']'), "bracket leaked: {out:?}");
    // Should be phonemes, not the letters h/l/U spelled out.
    assert!(out.starts_with('h'), "expected leading /h/, got {out:?}");
}

#[test]
fn inline_stress_mark_is_rendered() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    // The `'` inside the brackets is a stress phoneme → primary stress mark.
    let out = t.text_to_ipa("[[h'@l@U]]").unwrap();
    assert!(out.contains('ˈ'), "expected primary stress mark in {out:?}");
}

#[test]
fn unknown_mnemonics_are_skipped_not_spelled() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    // Digits/garbage that don't match a mnemonic must be dropped, not spoken
    // as numbers or letters.  A lone valid phoneme still comes through.
    let out = t.text_to_ipa("[[$$$k$$$]]").unwrap();
    assert_eq!(out, "k", "expected only /k/, got {out:?}");
}

#[test]
fn inline_phonemes_reach_the_code_stream() {
    let Some(t) = en() else { eprintln!("[SKIP] no en data"); return };
    // Regression: the `-x` / synthesis path (`translate_to_codes`) built the
    // inline-phoneme entry but the code-emission match dropped it, so `[[k]]`
    // produced no codes (empty `-x`).  It must now render like `text_to_ipa`.
    let codes = t.translate_to_codes("[[k]]").expect("codes");
    assert!(
        codes.iter().any(|c| !c.is_boundary && c.code >= 35),
        "inline [[k]] produced no phoneme codes (silently dropped)"
    );
    // Embedded in a sentence, [[k]] must add codes over the same text without it.
    let with = t
        .translate_to_codes("say [[k]] now")
        .expect("codes")
        .iter()
        .filter(|c| !c.is_boundary)
        .count();
    let without = t
        .translate_to_codes("say now")
        .expect("codes")
        .iter()
        .filter(|c| !c.is_boundary)
        .count();
    assert!(with > without, "inline [[k]] added no codes: {with} vs {without}");
}
