use espeak_ng::translate::Translator;

fn main() {
    use espeak_ng::EspeakNg;
    match EspeakNg::new("ru") {
        Ok(_)  => println!("Engine loaded OK"),
        Err(e) => println!("Engine load FAILED: {e}"),
    }

    match Translator::new_default("ru") {
        Ok(t) => {
            println!("Translator OK");
            // Try to translate the word
            let ipa = t.text_to_ipa("привет");
            println!("text_to_ipa: {:?}", ipa);
        }
        Err(e) => println!("Translator FAILED: {e}"),
    }
}
