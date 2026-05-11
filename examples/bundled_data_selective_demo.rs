//! Demonstrates selective bundled data features (`bundled-data-<lang>`).
//!
//! Regional tags like `en-us` or `en-rp` work as long as the base language data
//! is installed (e.g. `bundled-data-en` covers `en`, `en-us`, and `en-rp`).
//!
//! Run with:
//!   cargo run --example bundled_data_selective_demo --features bundled-data-en,bundled-data-uk

fn main() -> std::io::Result<()> {
    let selected = espeak_ng::bundled_languages();
    if selected.is_empty() {
        eprintln!("This example requires at least one selective bundled-data feature.");
        eprintln!("Run: cargo run --example bundled_data_selective_demo --features bundled-data-en,bundled-data-uk");
        return Ok(());
    }

    let data_dir = std::env::temp_dir().join("espeak-ng-bundled-selective-demo");
    std::fs::create_dir_all(&data_dir)?;

    println!("Selected bundled languages: {:?}", selected);
    println!("Installing selective bundled data -> {}", data_dir.display());
    espeak_ng::install_bundled_languages(&data_dir, selected)?;

    let samples = [
        ("en", "hello world"),
        ("en-us", "hello world"),
        ("en-rp", "hello world"),
        ("de", "guten Tag"),
        ("fr", "bonjour"),
        ("uk", "privit"),
    ];

    println!("\nText -> IPA for available selected languages:");
    for (lang, text) in samples {
        let base = lang.split('-').next().unwrap_or(lang);
        if !espeak_ng::has_bundled_language(base) {
            continue;
        }
        let engine = espeak_ng::EspeakNg::with_data_dir(lang, &data_dir)
            .expect("failed to init engine with selective data");
        let ipa = engine.text_to_phonemes(text).unwrap_or_default();
        println!("  [{lang}] {text:20} -> {ipa}");
    }

    std::fs::remove_dir_all(&data_dir)?;
    println!("\nTemp dir cleaned up. Done.");
    Ok(())
}
