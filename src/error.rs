//! Error types for the espeak-ng-rs library.

use crate::phoneme::PhonemeFeature;

/// All errors that can be produced by the espeak-ng-rs library.
///
/// Variants that start with a capital letter directly correspond to
/// `espeak_ng_STATUS` codes in `espeak_ng.h`.  Variants in `snake_case`
/// are Rust-native additions.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    // ------------------------------------------------------------------
    // Variants that map 1-to-1 onto espeak_ng_STATUS codes
    // C: ENS_COMPILE_ERROR
    // ------------------------------------------------------------------

    /// `ENS_COMPILE_ERROR` — a compile error occurred in voice/dictionary data.
    #[error("compile error in voice/dictionary data")]
    CompileError,

    /// `ENS_VERSION_MISMATCH` — the data file has an unexpected version tag.
    #[error("data version mismatch: got 0x{got:06x}, expected 0x{expected:06x}")]
    VersionMismatch {
        /// The version tag found in the data file.
        got: u32,
        /// The version tag this build expects.
        expected: u32,
    },

    /// `ENS_FIFO_BUFFER_FULL` — the audio FIFO buffer is full.
    #[error("audio FIFO buffer is full")]
    FifoBufferFull,

    /// `ENS_NOT_INITIALIZED` — espeak-ng has not been initialised.
    #[error("espeak-ng has not been initialised")]
    NotInitialized,

    /// `ENS_AUDIO_ERROR` — audio system error.
    #[error("audio system error")]
    AudioError,

    /// `ENS_VOICE_NOT_FOUND` — the requested voice could not be found.
    #[error("voice not found: {0}")]
    VoiceNotFound(String),

    /// `ENS_MBROLA_NOT_FOUND` — the MBROLA binary is not installed.
    #[error("MBROLA binary not found")]
    MbrolaNotFound,

    /// `ENS_MBROLA_VOICE_NOT_FOUND` — the requested MBROLA voice is missing.
    #[error("MBROLA voice not found")]
    MbrolaVoiceNotFound,

    /// `ENS_EVENT_BUFFER_FULL` — the synthesis event queue overflowed.
    #[error("event buffer full")]
    EventBufferFull,

    /// `ENS_NOT_SUPPORTED` — the requested operation is not supported.
    #[error("operation not supported")]
    NotSupported,

    /// `ENS_UNSUPPORTED_PHON_FORMAT` — unrecognised phoneme encoding format.
    #[error("unsupported phoneme format")]
    UnsupportedPhonFormat,

    /// `ENS_NO_SPECT_FRAMES` — no spectral frames are available for synthesis.
    #[error("no spectral frames available")]
    NoSpectFrames,

    /// `ENS_EMPTY_PHONEME_MANIFEST` — the phoneme manifest file is empty.
    #[error("phoneme manifest is empty")]
    EmptyPhonemeManifest,

    /// `ENS_SPEECH_STOPPED` — synthesis was stopped mid-utterance.
    #[error("speech was stopped")]
    SpeechStopped,

    /// `ENS_UNKNOWN_PHONEME_FEATURE` — the feature tag is not recognised.
    #[error("unknown phoneme feature: {0}")]
    UnknownPhonemeFeature(PhonemeFeature),

    /// `ENS_UNKNOWN_TEXT_ENCODING` — the text encoding name is not recognised.
    #[error("unknown or unsupported text encoding: {0}")]
    UnknownTextEncoding(String),

    // ------------------------------------------------------------------
    // Rust-native errors
    // ------------------------------------------------------------------

    /// The data path does not exist or cannot be opened.
    #[error("data path error: {0}")]
    DataPath(String),

    /// Invalid or corrupt binary data in an espeak-ng-data file.
    #[error("invalid data: {0}")]
    InvalidData(String),

    /// A feature of the Rust port has not been implemented yet.
    ///
    /// The oracle comparison tests treat this as a "skip" and print the C
    /// oracle output so you can see what the implementation should produce.
    #[error("not yet implemented: {0}")]
    NotImplemented(&'static str),

    /// An I/O error occurred while reading data files.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The text could not be decoded with the requested encoding.
    #[error("decoding error at byte offset {offset}: {detail}")]
    DecodingError {
        /// Byte offset in the input where the error occurred.
        offset: usize,
        /// Human-readable description of the error.
        detail: String,
    },
}

/// Convenience `Result` alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;
