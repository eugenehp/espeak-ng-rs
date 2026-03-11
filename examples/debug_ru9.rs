/// Debug: show what IPA codes Russian words produce and how they map to IPA symbols.
fn main() {
    let words = ["да", "мама", "привет", "нет", "Россия", "а"];
    for word in &words {
        let rust_ipa = espeak_ng::text_to_ipa("ru", word).unwrap_or_default();
        let c_ipa = std::process::Command::new("espeak-ng")
            .args(["-v", "ru", "-q", "--ipa", word])
            .output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let ok = if rust_ipa == c_ipa { "✓" } else { "✗" };
        println!("{ok} {word:10}: Rust={rust_ipa:15} C={c_ipa}");
    }
}
