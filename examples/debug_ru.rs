fn main() {
    // Russian word
    let cases = [
        ("ru", "привет"),
        ("ru", "да"),
        ("ru", "нет"),
        ("ru", "hello"),
    ];
    for (lang, word) in &cases {
        match espeak_ng::text_to_ipa(lang, word) {
            Ok(s)  => println!("[{lang}] {word:20} → {:?}", s),
            Err(e) => println!("[{lang}] {word:20} → ERROR: {e}"),
        }
    }
}
