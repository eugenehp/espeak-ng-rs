// tests/phonswitch.rs
//
// phonSWITCH — mid-clause language switching (`_^_LL` dictionary entries).
//
// The Afrikaans dictionary switches many foreign place names to English,
// German or French.  eSpeak NG re-translates such a word with the target
// language's translator and marks it `(target)…(orig)` in `--ipa` output.
//
// All expected values below are byte-identical to the C oracle
// (`espeak-ng -q --ipa -v af <word>`).  Tests skip when af data is absent.

use espeak_ng::Translator;

fn af() -> Option<Translator> {
    let t = Translator::new("af", None).ok()?;
    // Only run if the af dictionary is actually present.
    t.text_to_ipa("hallo").ok().map(|_| t)
}

#[test]
fn switch_to_english_default() {
    let Some(t) = af() else { eprintln!("[SKIP] no af data"); return };
    // `_^_EN` compiles to a bare phonSWITCH (EN == default voice).
    assert_eq!(t.text_to_ipa("boston").unwrap(), "(en)bˈɒstən(af)");
    assert_eq!(t.text_to_ipa("washington").unwrap(), "(en)wˈɒʃɪŋtən(af)");
    assert_eq!(t.text_to_ipa("chicago").unwrap(), "(en)ʃɪkˈɑːɡəʊ(af)");
}

#[test]
fn switch_to_named_language() {
    let Some(t) = af() else { eprintln!("[SKIP] no af data"); return };
    assert_eq!(t.text_to_ipa("zürich").unwrap(), "(de)tsˈyːrɪç(af)");
    assert_eq!(t.text_to_ipa("rhône").unwrap(), "(fr)ʁˈoːn(af)");
}

#[test]
fn switch_inside_a_sentence() {
    let Some(t) = af() else { eprintln!("[SKIP] no af data"); return };
    assert_eq!(
        t.text_to_ipa("ek woon in boston naby melbourne").unwrap(),
        "æk vˈʊən ən (en)bˈɒstən(af) nˈɑːbɛɪ (en)mˈɛlbən(af)",
    );
}

#[test]
fn non_switch_words_unaffected() {
    let Some(t) = af() else { eprintln!("[SKIP] no af data"); return };
    // An ordinary af word must not gain any (lang) markers.
    let ipa = t.text_to_ipa("hallo").unwrap();
    assert!(!ipa.contains('('), "unexpected switch marker in {ipa:?}");
}

// ── A switch that comes from the *rules*, not from a list entry ─────────────
//
// `*_rules` files can send a word to another language too: German has
// `igh (K  _^_EN` for English spellings, and about thirty languages use the
// same device (fr has 180 such rules, af 66, de 60, vi 43, da 39).  C throws
// away everything the rules matched so far and returns just the marker, so the
// word is re-translated whole; the port used to append the marker to the
// phonemes it had already produced, stranding it inside the word
// (`de Copyright` → `kˈoːpyːr_^_t`).

#[test]
fn rule_level_switch_retranslates_the_whole_word() {
    let Some(de) = Translator::new("de", None).ok() else {
        eprintln!("[SKIP] no de data");
        return;
    };
    assert_eq!(de.text_to_ipa("Copyright").unwrap(), "(en)kˈɒpɪɹˌaɪt(de)");
    assert_eq!(de.text_to_ipa("high").unwrap(), "(en)hˈaɪ(de)");
    // An ordinary German word is untouched.
    assert!(!de.text_to_ipa("Gesicht").unwrap().contains('('));
}

#[test]
fn rule_level_switch_in_other_languages() {
    if let Some(af) = Translator::new("af", None).ok() {
        assert_eq!(af.text_to_ipa("light").unwrap(), "(en)lˈaɪt(af)");
    }
    // A symbol whose entry is nothing *but* a switch (`fr ©`) re-translates the
    // character itself in the target language.
    if let Some(fr) = Translator::new("fr", None).ok() {
        assert_eq!(fr.text_to_ipa("©").unwrap(), "(en)kˈɒpɪɹˌaɪt(fr)");
    }
}
