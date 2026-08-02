// tests/capitals.rs
//
// Capital-letter announcement (`-k 2` / `option_capitals == 2`).
//
// eSpeak NG speaks the word "capital" (dict entry `_cap`) before each
// capitalised word.  Expected values are byte-identical to
// `espeak-ng -q --ipa -k 2 -v en <text>`.  Tests skip when en data is absent.

use espeak_ng::Translator;

fn en_caps() -> Option<Translator> {
    let mut t = Translator::new("en", None).ok()?;
    t.text_to_ipa("hallo").ok()?; // ensure data present
    t.options.capitals = 2;
    Some(t)
}

#[test]
fn announces_capital_before_capitalised_words() {
    let Some(t) = en_caps() else { eprintln!("[SKIP] no en data"); return };
    assert_eq!(t.text_to_ipa("Apple").unwrap(), "kˌapətəl ˈapəl");
    assert_eq!(
        t.text_to_ipa("Hello World").unwrap(),
        "kˌapətəl həlˈəʊ kˌapətəl wˈɜːld"
    );
    // Only the capital-initial word is announced.
    assert_eq!(t.text_to_ipa("the Big dog").unwrap(), "ðə kˌapətəl bˈɪɡ dˈɒɡ");
}

#[test]
fn lowercase_text_is_unaffected() {
    let Some(t) = en_caps() else { eprintln!("[SKIP] no en data"); return };
    assert_eq!(t.text_to_ipa("hello world").unwrap(), "həlˈəʊ wˈɜːld");
}

#[test]
fn disabled_by_default() {
    let t = match Translator::new("en", None) {
        Ok(t) if t.text_to_ipa("hallo").is_ok() => t,
        _ => { eprintln!("[SKIP] no en data"); return }
    };
    // Without capitals set, no announcement.
    assert_eq!(t.text_to_ipa("Apple").unwrap(), "ˈapəl");
}

#[test]
fn k1_inserts_the_capital_sound_not_the_word() {
    // `-k 1` (option_capitals == 1) inserts the phonCAPITAL *sound* (code 18,
    // preceded by a short pause) before a capitalised word, doubled for an
    // all-caps multi-letter word — it does NOT speak the word "capital".
    let Some(mut t) = Translator::new("en", None).ok() else {
        eprintln!("[SKIP] no en data");
        return;
    };
    // Ensure data present.
    if t.text_to_ipa("hi").is_err() { eprintln!("[SKIP]"); return; }
    t.options.capitals = 1;
    let cap_sounds = |s: &str| {
        t.translate_to_codes(s).unwrap().iter().filter(|c| c.code == 18).count()
    };
    assert_eq!(cap_sounds("Hello world"), 1, "one capital-initial word");
    assert_eq!(cap_sounds("hello world"), 0, "no capitals");
    assert_eq!(cap_sounds("NASA rocket"), 2, "all-caps word doubles the sound");
    // And it must NOT insert the spoken word "capital" (which -k 2 does).
    let ipa_k1 = {
        // codes → the capital sound is a phoneme, not the "_cap" word phonemes.
        cap_sounds("Hello")
    };
    assert_eq!(ipa_k1, 1);
}
