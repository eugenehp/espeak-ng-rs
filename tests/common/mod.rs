// tests/common/mod.rs
//
// Shared helpers for integration tests.

use std::process::Command;

// ---------------------------------------------------------------------------
// Data directory helpers
// ---------------------------------------------------------------------------

/// Default path to the espeak-ng data directory.
#[allow(dead_code)]
pub const ESPEAK_DATA_DIR: &str = "/usr/share/espeak-ng-data";

/// Returns `true` if the espeak-ng data directory exists and contains
/// the core files needed for synthesis (phontab, en_dict, phondata).
#[allow(dead_code)]
pub fn data_available() -> bool {
    use std::path::Path;
    let base = Path::new(ESPEAK_DATA_DIR);
    base.join("phontab").exists()
        && base.join("en_dict").exists()
        && base.join("phondata").exists()
}

// ---------------------------------------------------------------------------
// Binary availability check
// ---------------------------------------------------------------------------

/// Returns `true` if the `espeak-ng` binary is reachable on PATH.
#[allow(dead_code)]
pub fn espeak_available() -> bool {
    Command::new("espeak-ng")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Oracle helpers  (return Option – None means binary not found)
// ---------------------------------------------------------------------------

/// Run `espeak-ng` and return its IPA output, or `None` if the binary is
/// not available.
///
/// This is the most conservative oracle: no FFI, just the installed binary.
#[allow(dead_code)]
pub fn try_espeak_ipa(lang: &str, text: &str) -> Option<String> {
    let output = Command::new("espeak-ng")
        .args(["-v", lang, "-q", "--ipa", "--", text])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Run `espeak-ng` and return its IPA output.
///
/// # Panics
/// Panics if `espeak-ng` is not on PATH or exits with an error.
/// Use `try_espeak_ipa` when the binary may be absent.
#[allow(dead_code)]
pub fn espeak_ipa(lang: &str, text: &str) -> String {
    try_espeak_ipa(lang, text)
        .unwrap_or_else(|| panic!(
            "espeak-ng binary not found or failed \
             (lang={lang:?}, text={text:?}). \
             Install espeak-ng or skip oracle tests with \
             `cargo test --lib` / `cargo test --test encoding_integration`."
        ))
}

/// Run `espeak-ng` and return its Kirshenbaum (ASCII) phoneme output,
/// or `None` if the binary is not available.
#[allow(dead_code)]
pub fn try_espeak_phonemes(lang: &str, text: &str) -> Option<String> {
    let output = Command::new("espeak-ng")
        .args(["-v", lang, "-q", "-x", "--", text])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Run `espeak-ng` and return its Kirshenbaum (ASCII) phoneme output.
///
/// # Panics
/// Panics if `espeak-ng` is not on PATH.
#[allow(dead_code)]
pub fn espeak_phonemes(lang: &str, text: &str) -> String {
    try_espeak_phonemes(lang, text)
        .unwrap_or_else(|| panic!("espeak-ng not found"))
}

/// Synthesize `text` to a WAV file and return the raw bytes,
/// or `None` if the binary is not available.
#[allow(dead_code)]
pub fn try_espeak_wav(lang: &str, text: &str) -> Option<Vec<u8>> {
    static COUNTER: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmpfile = std::env::temp_dir().join(format!(
        "espeak_oracle_{}_{}.wav",
        std::process::id(),
        n,
    ));

    let status = Command::new("espeak-ng")
        .args(["-v", lang, "-w", tmpfile.to_str().unwrap(), "--", text])
        .status()
        .ok()?;

    if !status.success() {
        return None;
    }

    std::fs::read(&tmpfile).ok()
}

/// Synthesize `text` to a WAV file and return the raw bytes.
///
/// # Panics
/// Panics if `espeak-ng` is not on PATH.
#[allow(dead_code)]
pub fn espeak_wav(lang: &str, text: &str) -> Vec<u8> {
    try_espeak_wav(lang, text)
        .unwrap_or_else(|| panic!("espeak-ng not found"))
}

// ---------------------------------------------------------------------------
// PCM / audio helpers
// ---------------------------------------------------------------------------

/// Parse a WAV file's PCM data (16-bit, little-endian, mono).
///
/// Returns the raw samples after the 44-byte WAV header.
#[allow(dead_code)]
pub fn wav_to_pcm(wav: &[u8]) -> Vec<i16> {
    if wav.len() < 44 {
        return Vec::new();
    }
    let data = &wav[44..];
    data.chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect()
}

/// Root-mean-square of a sample buffer.
#[allow(dead_code)]
pub fn rms(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();
    (sum_sq / samples.len() as f64).sqrt()
}

/// Signal-to-noise ratio in dB between two equal-length PCM buffers.
///
/// Returns `f64::INFINITY` if `reference` is all zeros.
/// Returns `None` if the buffers differ in length.
#[allow(dead_code)]
pub fn snr_db(reference: &[i16], test: &[i16]) -> Option<f64> {
    if reference.len() != test.len() {
        return None;
    }
    let signal_power: f64 = reference.iter().map(|&s| (s as f64).powi(2)).sum();
    let noise_power: f64 = reference
        .iter()
        .zip(test.iter())
        .map(|(&r, &t)| (r as f64 - t as f64).powi(2))
        .sum();

    if signal_power == 0.0 {
        return Some(f64::INFINITY);
    }
    if noise_power == 0.0 {
        return Some(f64::INFINITY);
    }
    Some(10.0 * (signal_power / noise_power).log10())
}
