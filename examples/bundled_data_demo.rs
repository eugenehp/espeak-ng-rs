//! Demonstrates bundled data features: either the full `bundled-data` feature
//! or one or more selective `bundled-data-<lang>` features.
//!
//! Run with:
//!   cargo run --example bundled_data_demo --features bundled-data
//!   cargo run --example bundled_data_demo --features bundled-data-en,bundled-data-de

fn main() -> std::io::Result<()> {
    if !cfg!(feature = "bundled-data") && espeak_ng::bundled_languages().is_empty() {
        eprintln!("This example requires bundled data features");
        eprintln!("Run one of:");
        eprintln!("  cargo run --example bundled_data_demo --features bundled-data");
        eprintln!("  cargo run --example bundled_data_demo --features bundled-data-en,bundled-data-de");
        return Ok(());
    }

    #[cfg(feature = "bundled-data")]
    {
        // Create a temporary directory for the data
        let data_dir = std::env::temp_dir().join("espeak-ng-bundled-demo");
        std::fs::create_dir_all(&data_dir)?;

        println!("Installing bundled data → {}", data_dir.display());
        espeak_ng::install_bundled_data(&data_dir)?;
        println!("  {} files installed", count_files(&data_dir));

        // Use the extracted data
        let engine = espeak_ng::EspeakNg::with_data_dir("en", &data_dir)
            .expect("failed to init engine");

        let test_cases = [
            ("en", "hello world"),
            ("de", "guten Tag"),
            ("fr", "bonjour"),
            ("es", "hola mundo"),
        ];

        println!("\nText → IPA (using bundled data):");
        for (lang, text) in &test_cases {
            let engine = espeak_ng::EspeakNg::with_data_dir(lang, &data_dir)
                .expect("failed to init engine");
            let ipa = engine.text_to_phonemes(text).unwrap_or_default();
            println!("  [{lang}] {text:20} → {ipa}");
        }

        // Synthesize audio
        let (samples, rate) = engine.synth("hello world").expect("synth failed");
        println!("\nSynthesis: {} samples at {} Hz", samples.len(), rate);

        // Clean up
        std::fs::remove_dir_all(&data_dir)?;
        println!("\nTemp dir cleaned up. Done.");
        Ok(())
    }

    #[cfg(not(feature = "bundled-data"))]
    {
        let data_dir = std::env::temp_dir().join("espeak-ng-bundled-demo");
        std::fs::create_dir_all(&data_dir)?;

        let selected = espeak_ng::bundled_languages();
        println!("Installing selective bundled data {:?} -> {}", selected, data_dir.display());
        espeak_ng::install_bundled_languages(&data_dir, selected)?;
        println!("  {} files installed", count_files(&data_dir));

        let test_cases = [
            ("en", "hello world"),
            ("de", "guten Tag"),
            ("fr", "bonjour"),
            ("es", "hola mundo"),
            ("uk", "privit"),
            ("ru", "privet"),
        ];

        println!("\nText -> IPA (using selective bundled data):");
        for (lang, text) in test_cases {
            if !espeak_ng::has_bundled_language(lang) {
                continue;
            }
            let engine = espeak_ng::EspeakNg::with_data_dir(lang, &data_dir)
                .expect("failed to init engine");
            let ipa = engine.text_to_phonemes(text).unwrap_or_default();
            println!("  [{lang}] {text:20} -> {ipa}");
        }

        std::fs::remove_dir_all(&data_dir)?;
        println!("\nTemp dir cleaned up. Done.");
        Ok(())
    }
}

fn count_files(dir: &std::path::Path) -> usize {
    walkdir(dir, 0)
}

fn walkdir(dir: &std::path::Path, acc: usize) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return acc };
    rd.flatten().fold(acc, |a, e| {
        let p = e.path();
        if p.is_dir() { walkdir(&p, a) } else { a + 1 }
    })
}
