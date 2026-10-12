// Word-level IPA pairs: each source word with the phonemes it was rendered as.
//
//   cargo run --example word_phonemes -- "Version 2.5 uses .NET."
use espeak_ng::Translator;

fn main() {
    let mut args = std::env::args().skip(1);
    let text = args.next().unwrap_or_else(|| "Version 2.5 uses .NET.".to_string());
    let lang = args.next().unwrap_or_else(|| "en-us".to_string());
    let tr = Translator::new(&lang, Some(std::path::Path::new("espeak-ng-data"))).unwrap();
    for w in tr.text_to_ipa_words(&text).unwrap() {
        println!("{:>12} -> {}   [{}..{}]", w.word, w.phonemes, w.offset, w.offset + w.length);
    }
    println!("flat: {}", tr.text_to_ipa(&text).unwrap());
}
