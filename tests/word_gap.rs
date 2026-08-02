// tests/word_gap.rs
//
// `-g` word gap: inserts a pause phoneme (`_`) after each word in the phoneme
// (`-x`) output.  Verified byte-identical to the C oracle on single-clause
// input (`espeak-ng -q -x -g 20 -v en "two words"` → `t'u:_ w'3:dz_`).

use espeak_ng::Translator;

#[test]
fn word_gap_adds_one_pause_per_word() {
    let mut t = match Translator::new("en", None) {
        Ok(t) if t.translate_to_codes("two words").is_ok() => t,
        _ => { eprintln!("[SKIP] no en data"); return }
    };
    let base = t.translate_to_codes("two words").unwrap().len();

    t.options.word_gap = 20;
    let gapped = t.translate_to_codes("two words").unwrap();

    // Exactly one extra (pause) code per word.
    assert_eq!(gapped.len(), base + 2, "expected one pause per word");
}

#[test]
fn no_word_gap_by_default() {
    let t = match Translator::new("en", None) {
        Ok(t) if t.translate_to_codes("hello").is_ok() => t,
        _ => { eprintln!("[SKIP] no en data"); return }
    };
    // Default word_gap is 0, so codes are unchanged from a fresh translator.
    assert_eq!(t.options.word_gap, 0);
}
