// tests/bishnupriya.rs
//
// Bishnupriya Manipuri (bpy) reading.  bpy is an Indo-Aryan language written in
// the Bengali/Eastern-Nagari script (U+0980 block) and ships 60 group rules,
// but — unlike bn/as — was missing `letter_bits_offset = 0x980`, so groups3
// never found its rules and every word produced empty output.  Adding the
// Bengali offset + ISCII letter-groups restores it.

use espeak_ng::Translator;

#[test]
fn bishnupriya_words_read() {
    let Some(t) = Translator::new("bpy", None).ok() else {
        eprintln!("[SKIP] no bpy data");
        return;
    };
    let ipa = |s: &str| t.text_to_ipa(s).unwrap();
    // A conjunct (স্ক) collapses, and words read (not empty, no diacritic names).
    for (word, frag) in [("নমস্কার", "ʃkar"), ("ভাষা", "bhˈaʃa"), ("বাংলা", "aŋla")] {
        let got = ipa(word);
        assert!(!got.trim().is_empty(), "bpy {word:?} produced no phonemes");
        assert!(got.contains(frag), "bpy {word:?}: want {frag:?}, got {got:?}");
    }
}
