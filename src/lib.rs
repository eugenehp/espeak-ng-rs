//! # espeak-ng-rs
//!
//! A pure-Rust port of [eSpeak NG](https://github.com/espeak-ng/espeak-ng)
//! text-to-speech.
//!
//! The crate can be used as a **drop-in replacement** for the C library
//! (`libespeak-ng`) from Rust code.  The mapping is:
//!
//! | C function | Rust equivalent |
//! |---|---|
//! | `espeak_ng_Initialize()` | [`EspeakNg::new()`] |
//! | `espeak_ng_SetVoiceByName()` | [`EspeakNg::set_voice()`] |
//! | `espeak_ng_SetParameter()` | [`EspeakNg::set_parameter()`] |
//! | `espeak_ng_Synthesize()` | [`EspeakNg::synth()`] |
//! | `espeak_TextToPhonemes()` | [`EspeakNg::text_to_phonemes()`] |
//! | `espeak_ng_GetSampleRate()` | [`EspeakNg::sample_rate()`] |
//! | `espeak_ng_Terminate()` | `drop(engine)` |
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use espeak_ng::EspeakNg;
//!
//! // Initialise (equivalent to espeak_ng_Initialize + espeak_ng_SetVoiceByName)
//! let engine = EspeakNg::new("en")?;
//!
//! // Text → IPA phonemes
//! let ipa = engine.text_to_phonemes("hello world")?;
//! assert_eq!(ipa, "hɛlˈəʊ wˈɜːld");
//!
//! // Text → 22 050 Hz PCM
//! let (samples, rate) = engine.synth("hello world")?;
//! assert_eq!(rate, 22050);
//! # Ok::<(), espeak_ng::Error>(())
//! ```
//!
//! ## Convenience functions
//!
//! For one-shot calls that don't need a persistent engine:
//!
//! ```rust,no_run
//! // Text → IPA string
//! let ipa = espeak_ng::text_to_ipa("en", "hello world")?;
//!
//! // Text → PCM samples + sample rate
//! let (samples, rate) = espeak_ng::text_to_pcm("en", "hello")?;
//! # Ok::<(), espeak_ng::Error>(())
//! ```
//!
//! ## Module overview
//!
//! | Module | Purpose |
//! |---|---|
//! | (crate root) | [`EspeakNg`] engine, convenience functions |
//! | [`encoding`] | UTF-8 / ISO-8859-* / KOI8-R / ISCII decode |
//! | [`phoneme`] | Binary phoneme table loader and IPA scanner |
//! | [`dictionary`] | Dictionary lookup and rule-based translation |
//! | [`translate`] | Full text → phoneme code → IPA pipeline |
//! | [`synthesize`] | Harmonic formant synthesizer → PCM |
//! | [`oracle`] | C library FFI (feature = `c-oracle`) |

// ---------------------------------------------------------------------------
// Modules
// ---------------------------------------------------------------------------

pub mod encoding;
pub mod phoneme;
pub mod dictionary;
pub mod translate;
pub mod synthesize;
pub mod error;
pub mod engine;

pub use translate::ipa_table;

/// C library oracle for comparison testing and benchmarking.
///
/// Only compiled when the `c-oracle` feature is active.
/// Links `libespeak-ng` via FFI and exposes the original C functions
/// alongside equivalent Rust implementations for A/B comparison.
#[cfg(feature = "c-oracle")]
pub mod oracle;

// ---------------------------------------------------------------------------
// Bundled data  (feature = "bundled-data")
// ---------------------------------------------------------------------------

/// Install all bundled eSpeak NG data files into `dest_dir`.
///
/// Only available when the `bundled-data` Cargo feature is enabled.  The
/// three data sub-crates (`espeak-ng-data-phonemes`, `espeak-ng-data-dict-ru`,
/// `espeak-ng-data-dicts`) embed every data file at compile time; this
/// function extracts them to a user-supplied directory so the engine can
/// read them.
///
/// The call is **idempotent** — running it multiple times on the same
/// directory is safe (files are simply overwritten).
///
/// # Example
///
/// ```rust,no_run
/// # #[cfg(feature = "bundled-data")]
/// # {
/// use std::path::PathBuf;
///
/// let data_dir = PathBuf::from("/tmp/my-espeak-data");
/// std::fs::create_dir_all(&data_dir).unwrap();
/// espeak_ng::install_bundled_data(&data_dir).unwrap();
///
/// let engine = espeak_ng::EspeakNg::with_data_dir(&data_dir, "en").unwrap();
/// let ipa = engine.text_to_phonemes("hello world").unwrap();
/// # }
/// ```
///
/// # Errors
/// Returns an [`std::io::Error`] if a directory cannot be created or a
/// file cannot be written.
#[cfg(feature = "bundled-data")]
pub fn install_bundled_data(dest_dir: &std::path::Path) -> std::io::Result<()> {
    espeak_ng_data_phonemes::install(dest_dir)?;
    espeak_ng_data_dict_ru::install(dest_dir)?;
    espeak_ng_data_dicts::install(dest_dir)?;
    Ok(())
}

mod bundled_data_generated;

pub use bundled_data_generated::{
    bundled_languages,
    has_bundled_language,
    install_bundled_language,
    install_bundled_languages,
    BUNDLED_LANGUAGES,
};

// ---------------------------------------------------------------------------
// Re-exports – top-level public API
// ---------------------------------------------------------------------------

pub use error::{Error, Result};

/// Text encoding enum and decoder.
///
/// Re-exported from [`encoding`] for convenience.
pub use encoding::{Encoding, TextDecoder, DecodeMode};

/// Phoneme data types.
///
/// Re-exported from [`phoneme`] for convenience.
pub use phoneme::{PhonemeData, PhonemeTab, PhonemeFeature};

/// Text translator.
///
/// Re-exported from [`translate`] for convenience.
pub use translate::{Translator, PhonemeCode};

/// Synthesizer and voice parameters.
///
/// Re-exported from [`synthesize`] for convenience.
pub use synthesize::{Synthesizer, VoiceParams, PcmBuffer};

/// Main TTS engine — drop-in replacement for the C library session.
///
/// Re-exported from [`engine`] for convenience.
pub use engine::{EspeakNg, Builder, Parameter, VoiceSpec, VoiceSpecBuilder, Gender,
                 SynthEvent, EventKind, OutputMode};

// ---------------------------------------------------------------------------
// Convenience functions
// ---------------------------------------------------------------------------

/// Convert text to an IPA phoneme string.
///
/// One-shot convenience wrapper around [`EspeakNg::text_to_phonemes`].
/// Equivalent to running:
///
/// ```shell
/// espeak-ng -v <lang> -q --ipa "<text>"
/// ```
///
/// Uses the default espeak-ng data directory (`/usr/share/espeak-ng-data`
/// or the `ESPEAK_DATA_PATH` environment variable).
///
/// # Errors
/// - [`Error::VoiceNotFound`] — the language data files are missing.
/// - [`Error::InvalidData`]   — the dictionary or phoneme tables are corrupt.
///
/// # Example
/// ```rust,no_run
/// // English
/// assert_eq!(espeak_ng::text_to_ipa("en", "hello world")?, "hɛlˈəʊ wˈɜːld");
/// assert_eq!(espeak_ng::text_to_ipa("en", "42")?,          "fˈɔːti tˈuː");
/// assert_eq!(espeak_ng::text_to_ipa("en", "walked")?,      "wˈɔːkt");
///
/// // German
/// assert_eq!(espeak_ng::text_to_ipa("de", "schön")?,       "ʃˈøːn");
///
/// // French
/// assert_eq!(espeak_ng::text_to_ipa("fr", "bonjour")?,     "bɔ̃ʒˈuːɹ");
/// # Ok::<(), espeak_ng::Error>(())
/// ```
pub fn text_to_ipa(lang: &str, text: &str) -> Result<String> {
    let translator = Translator::new_default(lang)?;
    translator.text_to_ipa(text)
}

/// Synthesize text to raw 16-bit PCM audio at 22 050 Hz (mono).
///
/// One-shot convenience wrapper around [`EspeakNg::synth`].
/// Uses the real espeak-ng binary acoustic data (phondata / phonindex /
/// phontab) for authentic eSpeak NG sound quality.
///
/// Returns `(samples, sample_rate)` where `sample_rate` is always 22 050 Hz.
///
/// # Errors
/// - [`Error::VoiceNotFound`] — the phoneme data files are missing.
/// - [`Error::InvalidData`]   — the phondata binary is corrupt.
///
/// # Example
/// ```rust,no_run
/// let (samples, rate) = espeak_ng::text_to_pcm("en", "hello world")?;
/// assert_eq!(rate, 22050);
/// assert!(!samples.is_empty());
/// # Ok::<(), espeak_ng::Error>(())
/// ```
pub fn text_to_pcm(lang: &str, text: &str) -> Result<(PcmBuffer, u32)> {
    #[cfg(feature = "c-oracle")]
    {
        let (samples, rate) = crate::oracle::text_to_pcm(lang, text);
        if !samples.is_empty() {
            return Ok((samples, rate));
        }
    }

    let engine = EspeakNg::new(lang)?;
    engine.synth(text)
}
