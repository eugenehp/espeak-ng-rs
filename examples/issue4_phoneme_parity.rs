//! Phonemization samples from [GitHub issue #4](https://github.com/eugenehp/espeak-ng-rs/issues/4):
//! US vs UK regional English, prefix words (`unseen`), and “or” + comma phrasing.
//!
//! The library applies the same clause-level promotion for **IPA** (`text_to_ipa`) and
//! **phoneme codes** (`translate_to_codes`), so synthesis and `-x` output track IPA for these
//! cases. Full `-x` parity with C can still differ on minor mnemonic spellings for some regional
//! voices (e.g. `en-rp`).
//!
//! **Voices**
//!
//! - **US:** `en-us` (phoneme table + shared `en_dict`).
//! - **UK:** this crate uses the `en-rp` (Received Pronunciation) table — there is
//!   no `en-gb` entry in `phontab` for the Rust loader, so `en-gb` is not used here.
//!   The optional **espeak-ng** comparison block still tries `-v en-gb` because the
//!   official CLI exposes that voice name.
//!
//! Compare with the C CLI (same data directory via `ESPEAK_DATA_PATH`):
//!
//! ```text
//! export ESPEAK_DATA_PATH=/path/to/espeak-ng-data
//! espeak-ng -v en-us -q --ipa -- "seen or unseen,"
//! espeak-ng -v en-gb -q --ipa -- "seen or unseen,"
//! espeak-ng -v en-rp -q --ipa -- "seen or unseen,"
//! ```
//!
//! **`espeak-ng` binary:** set `ESPEAK_NG_BIN` or install `espeak-ng` on `PATH`.
//! (A path is also emitted when you build with `--features bundled-espeak` for benches.)
//!
//! Data directory (Rust side, first match):
//!
//! - `ESPEAK_NG_DATA` or `ESPEAK_DATA_PATH` if they contain `en_dict`
//! - `./espeak-ng-data`
//! - `/usr/share/espeak-ng-data`
//! - or `--features bundled-data` → temp extract
//!
//! ```text
//! cargo run --example issue4_phoneme_parity --features bundled-data
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

use espeak_ng::Translator;

/// Removes a temp data dir on drop when we extracted bundled files there.
struct TempDataGuard {
    path: PathBuf,
    cleanup: bool,
}

impl Drop for TempDataGuard {
    fn drop(&mut self) {
        if self.cleanup {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// US table + UK RP table (closest to “GB” in this port’s phoneme data).
const RUST_VOICES: &[(&str, &str)] = &[
    ("en-us", "US (en-us)"),
    ("en-rp", "UK (en-rp, RP)"),
];

/// Voices to try with the **espeak-ng** CLI for a GB vs US sanity check.
const CLI_VOICES: &[(&str, &str)] = &[
    ("en-us", "CLI US (en-us)"),
    ("en-gb", "CLI UK (en-gb)"),
    ("en-rp", "CLI UK (en-rp)"),
];

fn main() -> std::io::Result<()> {
    let (dir, cleanup) = prepare_data_dir()?;
    let _guard = TempDataGuard {
        path: dir.clone(),
        cleanup,
    };
    let data_dir = dir.as_path();

    println!("Using data dir: {}", data_dir.display());
    let _ = try_set_espeak_data_path_for_cli(data_dir);

    let samples = [
        "When choices cease",
        "unseen",
        "seen or unseen,",
    ];

    println!("\n--- Rust Translator (shared data dir) ---\n");
    let mut rust_rows: Vec<Vec<String>> = Vec::new();
    for text in &samples {
        rust_rows.push(vec![text.to_string()]);
    }

    for (voice, label) in RUST_VOICES {
        let t = Translator::new(voice, Some(data_dir)).unwrap_or_else(|e| {
            eprintln!("failed to create translator for {voice}: {e}");
            std::process::exit(1);
        });
        println!("Voice {voice} — {label} (normalised tag: {})", t.options.lang);
        for (i, text) in samples.iter().enumerate() {
            match t.text_to_ipa(text) {
                Ok(ipa) => {
                    let bad_ctrl: String = ipa.chars().filter(|c| c.is_control()).collect();
                    println!("  {text:?} → {ipa}");
                    if !bad_ctrl.is_empty() {
                        println!("    ! control chars in IPA: {:?}", bad_ctrl);
                    }
                    rust_rows[i].push(ipa.clone());
                }
                Err(e) => {
                    println!("  {text:?} → error: {e}");
                    rust_rows[i].push(format!("(error: {e})"));
                }
            }
        }
        println!();
    }

    println!("--- US (en-us) vs UK (en-rp) — same string? ---\n");
    for (i, text) in samples.iter().enumerate() {
        let us = rust_rows[i].get(1).map(String::as_str).unwrap_or("");
        let uk = rust_rows[i].get(2).map(String::as_str).unwrap_or("");
        let same = us == uk;
        println!("{text:?}: {}", if same { "identical" } else { "differs" });
        if !same {
            println!("  en-us: {us}");
            println!("  en-rp: {uk}");
        }
        println!();
    }

    if let Some(bin) = resolve_espeak_bin() {
        println!("--- espeak-ng CLI ({bin}) + ESPEAK_DATA_PATH ---\n");
        for (voice, label) in CLI_VOICES {
            println!("{label} (-v {voice})");
            for text in &samples {
                match espeak_cli_ipa(&bin, data_dir, voice, text) {
                    Ok(ipa) => println!("  {text:?} → {ipa}"),
                    Err(e) => println!("  {text:?} → (cli error: {e})"),
                }
            }
            println!();
        }

        println!("--- Rust en-us vs CLI en-us (same data) ---\n");
        for text in &samples {
            let ru = Translator::new("en-us", Some(data_dir))
                .and_then(|t| t.text_to_ipa(text))
                .unwrap_or_else(|e| format!("(rust err: {e})"));
            let cli = espeak_cli_ipa(&bin, data_dir, "en-us", text)
                .unwrap_or_else(|e| format!("(cli err: {e})"));
            let ok = ru == cli;
            println!("{text:?}: {}", if ok { "match" } else { "differs" });
            if !ok {
                println!("  rust: {ru}");
                println!("  cli:  {cli}");
            }
            println!();
        }
    } else {
        println!(
            "--- espeak-ng CLI: skipped (set ESPEAK_NG_BIN or install espeak-ng on PATH) ---\n"
        );
    }

    Ok(())
}

fn try_set_espeak_data_path_for_cli(data_dir: &Path) {
    if std::env::var("ESPEAK_DATA_PATH").is_err() {
        std::env::set_var("ESPEAK_DATA_PATH", data_dir);
        println!(
            "(set ESPEAK_DATA_PATH={} for subprocess comparison)\n",
            data_dir.display()
        );
    }
}

fn resolve_espeak_bin() -> Option<String> {
    if let Ok(b) = std::env::var("ESPEAK_NG_BIN") {
        let p = PathBuf::from(&b);
        if p.is_file() {
            return Some(b);
        }
    }
    let out = Command::new("espeak-ng").arg("--version").output().ok()?;
    if out.status.success() {
        return Some("espeak-ng".into());
    }
    None
}

fn espeak_cli_ipa(bin: &str, data_dir: &Path, voice: &str, text: &str) -> std::io::Result<String> {
    let output = Command::new(bin)
        .args(["-v", voice, "-q", "--ipa", "--", text])
        .env("ESPEAK_DATA_PATH", data_dir)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("status {:?}: {stderr}", output.status),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Returns `(path, remove_when_done)` — `remove_when_done` is true for a temp extract dir.
fn prepare_data_dir() -> std::io::Result<(PathBuf, bool)> {
    #[cfg(feature = "bundled-data")]
    {
        let dir = std::env::temp_dir().join("espeak-ng-issue4-demo");
        std::fs::create_dir_all(&dir)?;
        espeak_ng::install_bundled_data(&dir)?;
        return Ok((dir, true));
    }

    #[cfg(not(feature = "bundled-data"))]
    {
        for key in ["ESPEAK_NG_DATA", "ESPEAK_DATA_PATH"] {
            if let Ok(p) = std::env::var(key) {
                let pb = PathBuf::from(p);
                if pb.join("en_dict").exists() {
                    return Ok((pb, false));
                }
            }
        }
        let local = PathBuf::from("espeak-ng-data");
        if local.join("en_dict").exists() {
            return Ok((local, false));
        }
        let sys = PathBuf::from("/usr/share/espeak-ng-data");
        if sys.join("en_dict").exists() {
            return Ok((sys, false));
        }

        eprintln!("No eSpeak NG data found (need en_dict).");
        eprintln!("Set ESPEAK_NG_DATA / ESPEAK_DATA_PATH, add ./espeak-ng-data, install system data, or run:");
        eprintln!("  cargo run --example issue4_phoneme_parity --features bundled-data");
        std::process::exit(1);
    }
}
