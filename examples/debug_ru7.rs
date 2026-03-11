fn main() {
    let cases = [
        ("ru", "привет"), ("ru", "да"), ("ru", "нет"), ("ru", "я"),
        ("ru", "Россия"), ("ru", "мама"), ("ru", "папа"),
    ];
    for (lang, word) in &cases {
        let c = std::process::Command::new("espeak-ng")
            .args(["-v", lang, "-q", "--ipa", word])
            .output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let r = espeak_ng::text_to_ipa(lang, word).unwrap_or_default();
        let ok = if c == r { "✓" } else { "✗" };
        println!("{ok} [{lang}] {word:15}  C:{c:25} Rust:{r}");
    }
}
