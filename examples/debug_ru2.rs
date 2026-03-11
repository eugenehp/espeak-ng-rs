use espeak_ng::translate::{Translator, word_to_phonemes, LangOptions};

fn main() {
    use espeak_ng::EspeakNg;
    match EspeakNg::new("ru") {
        Ok(_)  => println!("Engine loaded OK"),
        Err(e) => println!("Engine load FAILED: {e}"),
    }

    match Translator::new("ru") {
        Ok(t) => {
            println!("Translator OK, lang={}", t.lang_name());
            // Try to translate the word
            let phonemes = t.text_to_phonemes("привет");
            println!("text_to_phonemes: {:?}", phonemes);
        }
        Err(e) => println!("Translator FAILED: {e}"),
    }
}
