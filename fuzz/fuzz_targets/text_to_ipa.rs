//! Text (plain and as SSML) to IPA.  The first byte picks the voice.
#![no_main]

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use espeak_ng::Translator;
use libfuzzer_sys::fuzz_target;

const LANGS: &[&str] = &["en", "de", "fr", "pt", "ru", "ar", "ku", "cmn", "hi", "tr"];

thread_local! {
    static TRANSLATORS: RefCell<HashMap<&'static str, Translator>> = RefCell::default();
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, text)) = data.split_first() else {
        return;
    };
    let text = String::from_utf8_lossy(text);
    let lang = LANGS[pick as usize % LANGS.len()];
    let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../espeak-ng-data"));
    TRANSLATORS.with_borrow_mut(|translators| {
        // Loading the data is most of the cost of a call.
        let translator = translators
            .entry(lang)
            .or_insert_with(|| Translator::new(lang, Some(dir)).expect("bundled voice"));
        let _ = translator.text_to_ipa(&text);
        let _ = translator.text_to_ipa_ssml(&text);
    });
});
