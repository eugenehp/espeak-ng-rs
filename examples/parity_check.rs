//! Prints IPA via [`espeak_ng::text_to_ipa`] for several language tags.
//!
//! Regional tags (`en-us`, `en-rp`, `pt-pt`, …) and underscores (`en_US`)
//! are normalized the same way as in the library API.

fn main() {
    let cases = [
        ("en", "hello"),
        ("en-us", "hello"),
        ("en-rp", "hello"),
        ("en_US", "hello"),
        ("pt", "olá"),
        ("pt-pt", "olá"),
        ("de", "Guten"),
        ("de", "Straße"),
        ("de", "regen"),
        ("fr", "bonjour"),
        ("fr", "rouge"),
        ("tr", "merhaba"),
        ("tr", "hello"),
        ("fi", "hyvää"),
        ("fi", "Helsinki"),
        ("ru", "привет"),
        ("es", "hola"),
        ("it", "ciao"),
        ("nl", "goedendag"),
        ("pl", "dzień"),
        ("sv", "hej"),
        ("da", "hej"),
        ("hu", "szia"),
        ("tr", "su"),
        ("tr", "ev"),
        ("tr", "yol"),
    ];

    const W_TAG: usize = 10;

    for (lang, word) in &cases {
        match espeak_ng::text_to_ipa(lang, word) {
            Ok(s) => println!("{:<w$} {:<20} Rust: {}", lang, word, s, w = W_TAG),
            Err(e) => println!("{:<w$} {:<20} ERR:  {}", lang, word, e, w = W_TAG),
        }
    }
}
