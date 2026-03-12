// src/oracle/mod.rs
//
// Thin FFI wrapper around libespeak-ng (the C library).
//
// This module is **only compiled when the `c-oracle` feature is active**.
// Its purpose is:
//   1. Provide the C implementation's output for comparison tests.
//   2. Serve as the baseline for benchmarks (`benches/vs_c.rs`).
//
// Thread-safety note:
//   libespeak-ng uses extensive global state and is NOT thread-safe.
//   All calls go through a single `Mutex<OracleInner>`.  Tests and
//   benchmarks that use the oracle must therefore run sequentially
//   (use `cargo test -- --test-threads=1` or the mutex will simply
//   serialise them for you).

#![cfg(feature = "c-oracle")]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::{Mutex, OnceLock};

fn espeak_bin_path() -> String {
    std::env::var("ESPEAK_NG_BIN")
        .ok()
        .or_else(|| option_env!("ESPEAK_NG_BIN").map(str::to_owned))
        .unwrap_or_else(|| "espeak-ng".to_string())
}

fn maybe_data_path() -> Option<String> {
    std::env::var("ESPEAK_NG_DATA")
        .ok()
        .or_else(|| option_env!("ESPEAK_NG_DATA").map(str::to_owned))
}

fn espeak_cmd() -> std::process::Command {
    let mut cmd = std::process::Command::new(espeak_bin_path());
    if let Some(data) = maybe_data_path() {
        cmd.env("ESPEAK_DATA_PATH", data);
    }
    cmd
}

fn wav_to_pcm(wav: &[u8]) -> Option<Vec<i16>> {
    if wav.len() < 44 {
        return None;
    }
    Some(
        wav[44..]
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Raw C declarations
// From: /usr/include/espeak-ng/speak_lib.h
//       /usr/include/espeak-ng/espeak_ng.h
// ---------------------------------------------------------------------------

/// `AUDIO_OUTPUT_SYNCHRONOUS` – synthesize synchronously.
const AUDIO_OUTPUT_SYNCHRONOUS: c_int = 3;

/// `espeakCHARS_UTF8`
const ESPEAK_CHARS_UTF8: c_int = 1;
/// `espeakPHONEMES_IPA` – return IPA strings in the phoneme callback.
const ESPEAK_PHONEMES_IPA: c_int = 0x02;

extern "C" {
    // espeak (legacy API) -----------------------------------------------

    /// int espeak_Initialize(espeak_AUDIO_OUTPUT output, int buflength,
    ///                       const char *path, int options) -> sample_rate
    fn espeak_Initialize(
        output: c_int,
        buflength: c_int,
        path: *const c_char,
        options: c_int,
    ) -> c_int;

    /// espeak_ERROR espeak_SetVoiceByName(const char *name)
    fn espeak_SetVoiceByName(name: *const c_char) -> c_int;

    /// const char *espeak_TextToPhonemes(const void **textptr,
    ///                                   int textmode, int phonememode)
    fn espeak_TextToPhonemes(
        textptr: *mut *const c_void,
        textmode: c_int,
        phonememode: c_int,
    ) -> *const c_char;

    /// espeak_ERROR espeak_Terminate()
    fn espeak_Terminate() -> c_int;

    // espeak-ng (new API) ------------------------------------------------

    /// void espeak_ng_InitializePath(const char *path)
    fn espeak_ng_InitializePath(path: *const c_char);

    /// espeak_ng_STATUS espeak_ng_Initialize(espeak_ng_ERROR_CONTEXT *ctx)
    fn espeak_ng_Initialize(ctx: *mut *mut c_void) -> u32;

    /// espeak_ng_STATUS espeak_ng_InitializeOutput(
    ///   espeak_ng_OUTPUT_MODE output_mode, int buffer_length, const char *device)
    fn espeak_ng_InitializeOutput(
        output_mode: u32,
        buffer_length: c_int,
        device: *const c_char,
    ) -> u32;

    /// espeak_ng_STATUS espeak_ng_SetVoiceByName(const char *name)
    fn espeak_ng_SetVoiceByName(name: *const c_char) -> u32;

    /// int espeak_ng_GetSampleRate()
    fn espeak_ng_GetSampleRate() -> c_int;
}

// ---------------------------------------------------------------------------
// Initialization guard
// ---------------------------------------------------------------------------

/// Internal state – lives inside the global Mutex.
struct OracleInner {
    initialized: bool,
    current_voice: String,
    sample_rate: u32,
}

/// The global, lazily-initialized oracle.
static ORACLE: OnceLock<Mutex<OracleInner>> = OnceLock::new();

fn get_oracle() -> &'static Mutex<OracleInner> {
    ORACLE.get_or_init(|| {
        Mutex::new(OracleInner {
            initialized: false,
            current_voice: String::new(),
            sample_rate: 0,
        })
    })
}

/// Initialize the C library (idempotent; safe to call many times).
///
/// Uses `AUDIO_OUTPUT_RETRIEVAL` so no sound card is needed.
fn ensure_initialized(inner: &mut OracleInner) {
    if inner.initialized {
        return;
    }
    unsafe {
        // espeak_ng path: uses the installed espeak-ng-data
        espeak_ng_InitializePath(std::ptr::null());
        let status = espeak_ng_Initialize(std::ptr::null_mut());
        assert_eq!(status, 0, "espeak_ng_Initialize failed with status {status}");

        // ENOUTPUT_MODE_SYNCHRONOUS = 0x0001
        let status = espeak_ng_InitializeOutput(0x0001, 0, std::ptr::null());
        assert_eq!(
            status, 0,
            "espeak_ng_InitializeOutput failed with status {status}"
        );

        inner.sample_rate = espeak_ng_GetSampleRate() as u32;
    }
    inner.initialized = true;
}

fn ensure_voice(inner: &mut OracleInner, lang: &str) {
    ensure_initialized(inner);
    if inner.current_voice == lang {
        return;
    }
    let cname = CString::new(lang).expect("lang must not contain interior nuls");
    let status = unsafe { espeak_ng_SetVoiceByName(cname.as_ptr()) };
    assert_eq!(
        status, 0,
        "espeak_ng_SetVoiceByName({lang:?}) failed with status {status}"
    );
    inner.current_voice = lang.to_string();
}

// ---------------------------------------------------------------------------
// Public interface
// ---------------------------------------------------------------------------

/// The sample rate used by the C library (22050 Hz for espeak-ng).
pub fn sample_rate() -> u32 {
    let mut guard = get_oracle().lock().unwrap();
    ensure_initialized(&mut guard);
    guard.sample_rate
}

/// Translate `text` to IPA using the C library.
///
/// Equivalent to: `espeak-ng -v <lang> -q --ipa <text>`
///
/// Returns the full IPA string with stress marks and syllable separators,
/// exactly as the C library produces it.  Trailing whitespace/newlines
/// are stripped.
///
/// # Panics
/// Panics if the C library fails to initialize or the voice is not found.
pub fn text_to_ipa(lang: &str, text: &str) -> String {
    let mut guard = get_oracle().lock().unwrap();
    ensure_voice(&mut guard, lang);
    let mut ipa_parts: Vec<String> = Vec::new();

    // espeak_TextToPhonemes walks a pointer through the input text,
    // clause by clause.  We loop until it returns NULL.
    //
    // The legacy `espeak_Initialize` API is needed because
    // `espeak_TextToPhonemes` is only in the legacy API surface.
    // We re-initialize with AUDIO_OUTPUT_SYNCHRONOUS for text-only use.
    unsafe {
        // Re-init with synchronous mode (no sound, phoneme-only output).
        // The sample rate is guaranteed to be 22050 by the espeak-ng design.
        let sr = espeak_Initialize(
            AUDIO_OUTPUT_SYNCHRONOUS,
            0,
            std::ptr::null(),
            0,
        );
        assert!(sr > 0, "espeak_Initialize (legacy) failed: sr={sr}");

        let cname = CString::new(lang).unwrap();
        let rc = espeak_SetVoiceByName(cname.as_ptr());
        assert_eq!(rc, 0, "espeak_SetVoiceByName({lang:?}) = {rc}");

        // Null-terminate the input for C.
        let c_text = CString::new(text).expect("text must not contain interior nuls");
        let mut ptr: *const c_void = c_text.as_ptr() as *const c_void;

        loop {
            let result = espeak_TextToPhonemes(
                &mut ptr,
                ESPEAK_CHARS_UTF8,
                ESPEAK_PHONEMES_IPA,
            );
            if result.is_null() {
                break;
            }
            let part = CStr::from_ptr(result)
                .to_string_lossy()
                .into_owned();
            if !part.trim().is_empty() {
                ipa_parts.push(part);
            }
            if ptr.is_null() {
                break;
            }
        }

        espeak_Terminate();

        // Re-initialize the ng API so subsequent calls work.
        guard.initialized = false;
        guard.current_voice = String::new();
        ensure_initialized(&mut guard);
    }

    ipa_parts.join(" ").trim().to_string()
}

/// Translate `text` using the CLI binary (`espeak-ng`).
///
/// This is a fallback for environments where the shared library is not
/// available, and also useful for verifying that the FFI gives the same
/// output as the binary.
///
/// Returns `Err` if `espeak-ng` is not on PATH.
pub fn text_to_ipa_cli(lang: &str, text: &str) -> Result<String, String> {
    let output = espeak_cmd()
        .args(["-v", lang, "-q", "--ipa", "--", text])
        .output()
        .map_err(|e| format!("failed to run espeak-ng: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "espeak-ng exited with {:?}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string())
}

/// Synthesize `text` to raw 16-bit PCM samples using the C library.
///
/// Returns `(samples, sample_rate)`.
///
/// # Status
/// Not yet implemented – synthesis via the callback API is more complex
/// than phoneme conversion.  This stub returns an empty buffer.
pub fn text_to_pcm(_lang: &str, _text: &str) -> (Vec<i16>, u32) {
    let lang = _lang;
    let text = _text;
    let sr = sample_rate();

    let output = match espeak_cmd()
        .args(["--stdout", "-v", lang, "--", text])
        .output()
    {
        Ok(o) => o,
        Err(_) => return (Vec::new(), sr),
    };

    if !output.status.success() {
        return (Vec::new(), sr);
    }

    match wav_to_pcm(&output.stdout) {
        Some(pcm) => (pcm, sr),
        None => (Vec::new(), sr),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // These tests require the `c-oracle` feature and libespeak-ng installed.
    // Run with:  cargo test --features c-oracle -- --test-threads=1

    #[test]
    fn oracle_sample_rate_is_22050() {
        assert_eq!(sample_rate(), 22050);
    }

    #[test]
    fn oracle_cli_english_hello() {
        let ipa = text_to_ipa_cli("en", "hello").unwrap();
        // eSpeak NG produces "həlˈəʊ" for "hello" in English.
        // We check it's non-empty and contains 'h' as a sanity check.
        assert!(!ipa.is_empty(), "IPA should not be empty");
        assert!(
            ipa.contains('h'),
            "English 'hello' IPA should start with /h/, got: {ipa:?}"
        );
    }

    #[test]
    fn oracle_cli_french_bonjour() {
        let ipa = text_to_ipa_cli("fr", "bonjour").unwrap();
        assert!(!ipa.is_empty());
        // French /bɔ̃ʒuʁ/ should contain 'b'
        assert!(ipa.contains('b'), "got: {ipa:?}");
    }

    #[test]
    fn oracle_cli_matches_multiple_words() {
        let ipa = text_to_ipa_cli("en", "hello world").unwrap();
        assert!(!ipa.is_empty());
        // Both words should produce something
        assert!(ipa.len() > 4, "got: {ipa:?}");
    }

    #[test]
    fn oracle_cli_and_lib_agree_on_english() {
        // Compare CLI output with the library output for a simple word.
        // They should produce identical strings.
        let cli_ipa = text_to_ipa_cli("en", "hello").unwrap();
        let lib_ipa = text_to_ipa("en", "hello");

        assert_eq!(
            cli_ipa, lib_ipa,
            "CLI and library disagree on /hello/:\n  CLI: {cli_ipa:?}\n  LIB: {lib_ipa:?}"
        );
    }
}
