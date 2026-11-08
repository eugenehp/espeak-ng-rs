// The marker that closes a language switch names the base voice's phoneme
// table, not the voice tag. Expected values are from `espeak-ng -q --ipa -v <voice>`
// (identical on 1.52.0 and master). The tests need the full data set: set
// ESPEAK_NG_DATA to an espeak-ng-data directory, otherwise they are skipped.

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
