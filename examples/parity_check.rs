fn main() {
    let cases = [
        ("en","hello"),("en","world"),("de","Guten"),("de","Straße"),("de","regen"),
        ("fr","bonjour"),("fr","rouge"),
        ("tr","merhaba"),("tr","hello"),("fi","hyvää"),("fi","Helsinki"),
        ("ru","привет"),("es","hola"),("it","ciao"),("nl","goedendag"),
        ("pl","dzień"),("sv","hej"),("da","hej"),("hu","szia"),
        ("tr","su"),("tr","ev"),("tr","yol"),
    ];
    for (lang, word) in &cases {
        match espeak_ng::text_to_ipa(lang, word) {
            Ok(s) => println!("{:<6} {:<20} Rust: {}", lang, word, s),
            Err(e) => println!("{:<6} {:<20} ERR:  {}", lang, word, e),
        }
    }
}
