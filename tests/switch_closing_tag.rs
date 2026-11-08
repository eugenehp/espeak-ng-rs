// A marker closing a language switch names the base voice's phoneme table, not the voice tag. Expected values come from espeak-ng -q --ipa -v <voice>. Without ESPEAK_NG_DATA the tests return early and pass vacuously.

use espeak_ng::Translator;

fn ipa(voice: &str, word: &str) -> Option<String> {
    let dir = std::env::var_os("ESPEAK_NG_DATA")?;
    Translator::new(voice, Some(std::path::Path::new(&dir))).ok()?.text_to_ipa(word).ok()
}

#[test]
fn pt_closes_with_its_phoneme_table_name() {
    let Some(got) = ipa("pt", "michael") else { return };
    assert_eq!(got, "(en)mˈaɪkəl(pt-pt)");
    assert_eq!(ipa("pt", "trump").as_deref(), Some("(en)tɹˈʌmp(pt-pt)"));
    assert_eq!(ipa("pt", "feuerbach").as_deref(), Some("(de)fˈɔøɜbˌax(pt-pt)"));
}

#[test]
fn pt_br_closes_with_pt() {
    let Some(got) = ipa("pt-br", "michael") else { return };
    assert_eq!(got, "(en)mˈaɪkəl(pt)");
}
