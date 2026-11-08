// A marker closing a language switch names the base voice's phoneme table, not the voice tag. Expected values come from espeak-ng -q --ipa -v <voice>. The tests are ignored by default and fail when run without ESPEAK_NG_DATA; run them with `cargo test -- --ignored`.

use espeak_ng::Translator;

fn ipa(voice: &str, word: &str) -> String {
    let dir = std::env::var_os("ESPEAK_NG_DATA")
        .expect("ESPEAK_NG_DATA must point at an espeak-ng-data directory");
    Translator::new(voice, Some(std::path::Path::new(&dir)))
        .unwrap_or_else(|e| panic!("cannot load voice {voice}: {e:?}"))
        .text_to_ipa(word)
        .unwrap_or_else(|e| panic!("cannot translate {word:?} with {voice}: {e:?}"))
}

#[test]
#[ignore = "needs ESPEAK_NG_DATA pointing at an espeak-ng-data dir"]
fn pt_closes_with_its_phoneme_table_name() {
    assert_eq!(ipa("pt", "michael"), "(en)mˈaɪkəl(pt-pt)");
    assert_eq!(ipa("pt", "trump"), "(en)tɹˈʌmp(pt-pt)");
    assert_eq!(ipa("pt", "feuerbach"), "(de)fˈɔøɜbˌax(pt-pt)");
}

#[test]
#[ignore = "needs ESPEAK_NG_DATA pointing at an espeak-ng-data dir"]
fn pt_br_closes_with_pt() {
    assert_eq!(ipa("pt-br", "michael"), "(en)mˈaɪkəl(pt)");
}
