//! Top-level drop-in replacement for the eSpeak NG C library.
//!
//! This module provides [`EspeakNg`], a stateful TTS engine that mirrors the
//! C library's session API:
//!
//! | C function | Rust equivalent |
//! |---|---|
//! | `espeak_ng_Initialize()` | [`EspeakNg::new()`] / [`Builder::build()`] |
//! | `espeak_ng_SetVoiceByName()` | [`EspeakNg::set_voice()`] |
//! | `espeak_ng_SetParameter()` | [`EspeakNg::set_parameter()`] |
//! | `espeak_ng_GetParameter()` | [`EspeakNg::get_parameter()`] |
//! | `espeak_ng_Synthesize()` | [`EspeakNg::synth()`] |
//! | `espeak_TextToPhonemes()` | [`EspeakNg::text_to_phonemes()`] |
//! | `espeak_ng_GetSampleRate()` | [`EspeakNg::sample_rate()`] |
//! | `espeak_ng_Terminate()` | drop |
//!
//! # Quick start
//!
//! ```rust,no_run
//! use espeak_ng::EspeakNg;
//!
//! // Equivalent to espeak_ng_Initialize() + espeak_ng_SetVoiceByName("en")
//! let mut engine = EspeakNg::builder().voice("en").build()?;
//!
//! // Text → IPA  (espeak_TextToPhonemes with IPA flag)
//! let ipa = engine.text_to_phonemes("hello world")?;
//! assert_eq!(ipa, "hɛlˈəʊ wˈɜːld");
//!
//! // Text → PCM  (espeak_ng_Synthesize in RETRIEVAL mode)
//! let (samples, rate) = engine.synth("hello world")?;
//! assert_eq!(rate, 22050);
//!
//! // Adjust voice  (espeak_ng_SetParameter)
//! engine.set_parameter(espeak_ng::Parameter::Rate, 150);
//! engine.set_parameter(espeak_ng::Parameter::Pitch, 60);
//! # Ok::<(), espeak_ng::Error>(())
//! ```

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::phoneme::PhonemeData;
use crate::synthesize::{PcmBuffer, Synthesizer, VoiceParams};
use crate::translate::{default_data_dir, Translator};

// ---------------------------------------------------------------------------
// Parameter – mirrors espeak_PARAMETER
// ---------------------------------------------------------------------------

/// Speech parameters, mirroring `espeak_PARAMETER` from `speak_lib.h`.
///
/// Pass to [`EspeakNg::set_parameter`] / [`EspeakNg::get_parameter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Parameter {
    /// Speaking rate in words-per-minute (80–450, default 175).
    ///
    /// C: `espeakRATE`
    Rate,
    /// Output volume (0–200, default 100).  0 = silence.
    ///
    /// C: `espeakVOLUME`
    Volume,
    /// Base pitch (0–100, default 50).
    ///
    /// C: `espeakPITCH`
    Pitch,
    /// Pitch range / intonation depth (0–100, default 50).  0 = monotone.
    ///
    /// C: `espeakRANGE`
    Range,
    /// Punctuation announcement mode.
    ///
    /// C: `espeakPUNCTUATION`
    Punctuation,
    /// Capital-letter announcement (0 = none, 1 = sound, 2 = spell, ≥3 = pitch raise in Hz).
    ///
    /// C: `espeakCAPITALS`
    Capitals,
    /// Pause between words, in units of 10 ms.
    ///
    /// C: `espeakWORDGAP`
    WordGap,
}

// ---------------------------------------------------------------------------
// VoiceSpec – mirrors espeak_VOICE
// ---------------------------------------------------------------------------

/// Voice selection criteria, mirroring `espeak_VOICE` from `speak_lib.h`.
///
/// Build one with [`VoiceSpec::builder()`] or use [`VoiceSpec::by_name`]
/// for the common case of selecting a voice by language code.
///
/// # Examples
/// ```rust
/// use espeak_ng::VoiceSpec;
///
/// let v = VoiceSpec::by_name("en");
/// let v = VoiceSpec::builder().language("fr").gender(espeak_ng::Gender::Female).build();
/// ```
#[derive(Debug, Clone, Default)]
pub struct VoiceSpec {
    /// BCP-47 language tag, e.g. `"en"`, `"en-gb"`, `"de"`.
    pub language: Option<String>,
    /// Voice name as it appears in the espeak-ng voices directory.
    pub name: Option<String>,
    /// Preferred gender.
    pub gender: Gender,
    /// Preferred speaker age (0 = unspecified).
    pub age: u8,
}

impl VoiceSpec {
    /// Create a voice spec that selects by language code only.
    ///
    /// Equivalent to calling `espeak_ng_SetVoiceByName("en")`.
    pub fn by_name(lang: &str) -> Self {
        VoiceSpec {
            language: Some(lang.to_string()),
            ..Default::default()
        }
    }

    /// Start building a voice specification.
    pub fn builder() -> VoiceSpecBuilder {
        VoiceSpecBuilder::default()
    }

    /// Return the effective language tag (language or name field).
    pub(crate) fn effective_lang(&self) -> &str {
        self.language
            .as_deref()
            .or(self.name.as_deref())
            .unwrap_or("en")
    }
}

/// Builder for [`VoiceSpec`].
#[derive(Debug, Default)]
pub struct VoiceSpecBuilder {
    spec: VoiceSpec,
}

impl VoiceSpecBuilder {
    /// Set the language tag (e.g. `"en"`, `"de"`, `"fr"`).
    pub fn language(mut self, lang: &str) -> Self {
        self.spec.language = Some(lang.to_string());
        self
    }

    /// Set the voice name.
    pub fn name(mut self, name: &str) -> Self {
        self.spec.name = Some(name.to_string());
        self
    }

    /// Set the preferred gender.
    pub fn gender(mut self, gender: Gender) -> Self {
        self.spec.gender = gender;
        self
    }

    /// Set the preferred speaker age (0 = unspecified).
    pub fn age(mut self, age: u8) -> Self {
        self.spec.age = age;
        self
    }

    /// Finalise the builder.
    pub fn build(self) -> VoiceSpec {
        self.spec
    }
}

// ---------------------------------------------------------------------------
// Gender
// ---------------------------------------------------------------------------

/// Speaker gender, mirroring `espeak_ng_VOICE_GENDER`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Gender {
    /// Gender not specified (default).
    #[default]
    Unknown = 0,
    /// Male voice.
    Male    = 1,
    /// Female voice.
    Female  = 2,
    /// Gender-neutral voice.
    Neutral = 3,
}

// ---------------------------------------------------------------------------
// SynthEvent – mirrors espeak_EVENT
// ---------------------------------------------------------------------------

/// An event fired during synthesis, mirroring `espeak_EVENT` from `speak_lib.h`.
///
/// In the C library these are delivered via a callback.  In Rust they are
/// returned as a `Vec<SynthEvent>` alongside the PCM samples from
/// [`EspeakNg::synth`].
#[derive(Debug, Clone)]
pub struct SynthEvent {
    /// The type of event.
    pub kind: EventKind,
    /// Character offset in the input text where this event originates.
    pub text_position: usize,
    /// Time offset within the generated audio in milliseconds.
    pub audio_position_ms: u32,
}

/// The kind of a [`SynthEvent`], mirroring `espeak_EVENT_TYPE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    /// Start of a word.  Payload is the word index within the sentence.
    Word(u32),
    /// Start of a sentence.
    Sentence,
    /// End of the current sentence or clause.
    End,
    /// End of the entire synthesis request.
    MsgTerminated,
    /// A phoneme boundary (only produced when phoneme events are enabled).
    Phoneme(String),
}

// ---------------------------------------------------------------------------
// OutputMode
// ---------------------------------------------------------------------------

/// Output mode for [`EspeakNg::synth`], mirroring `espeak_AUDIO_OUTPUT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputMode {
    /// Return PCM samples directly (synchronous retrieval).  Default.
    ///
    /// C: `AUDIO_OUTPUT_SYNCHRONOUS`
    #[default]
    Retrieval,
}

// ---------------------------------------------------------------------------
// EspeakNg — the main engine
// ---------------------------------------------------------------------------

/// Stateful eSpeak NG text-to-speech engine.
///
/// Drop-in replacement for the C library session.
/// All state that the C library keeps in process-global variables is stored
/// here instead, making it safe to use multiple engines concurrently.
///
/// # Lifecycle
///
/// ```rust,no_run
/// use espeak_ng::EspeakNg;
///
/// // Initialise (equivalent to espeak_ng_Initialize + espeak_ng_SetVoiceByName)
/// let mut engine = EspeakNg::new("en")?;
///
/// // Use
/// let ipa = engine.text_to_phonemes("hello")?;
///
/// // Drop releases all resources (equivalent to espeak_ng_Terminate)
/// drop(engine);
/// # Ok::<(), espeak_ng::Error>(())
/// ```
pub struct EspeakNg {
    /// Active voice specification.
    voice_spec: VoiceSpec,
    /// Speech rate in words-per-minute.
    rate:       u32,
    /// Output volume (0–200).
    volume:     u32,
    /// Base pitch (0–100).
    pitch:      u32,
    /// Pitch range (0–100).
    range:      u32,
    /// Word gap in 10ms units.
    word_gap:   i32,
    /// Path to the espeak-ng data directory.
    data_dir:   PathBuf,
}

impl EspeakNg {
    // ── Construction ────────────────────────────────────────────────────

    /// Initialise the engine for the given language code.
    ///
    /// Uses the default espeak-ng data directory (`/usr/share/espeak-ng-data`
    /// or the `ESPEAK_DATA_PATH` environment variable).
    ///
    /// Equivalent to:
    /// ```c
    /// espeak_ng_Initialize(NULL);
    /// espeak_ng_SetVoiceByName("en");
    /// ```
    pub fn new(lang: &str) -> Result<Self> {
        Self::with_data_dir(lang, Path::new(&default_data_dir()))
    }

    /// Initialise the engine pointing at an explicit data directory.
    pub fn with_data_dir(lang: &str, data_dir: &Path) -> Result<Self> {
        if !data_dir.exists() {
            return Err(Error::DataPath(format!(
                "espeak-ng data directory not found: {}",
                data_dir.display()
            )));
        }
        Ok(EspeakNg {
            voice_spec: VoiceSpec::by_name(lang),
            rate:       175,
            volume:     100,
            pitch:      50,
            range:      50,
            word_gap:   0,
            data_dir:   data_dir.to_path_buf(),
        })
    }

    /// Start building an engine with a fluent builder.
    ///
    /// # Example
    /// ```rust,no_run
    /// let engine = espeak_ng::EspeakNg::builder()
    ///     .voice("en")
    ///     .rate(200)
    ///     .pitch(55)
    ///     .build()?;
    /// # Ok::<(), espeak_ng::Error>(())
    /// ```
    pub fn builder() -> Builder {
        Builder::default()
    }

    // ── Configuration ────────────────────────────────────────────────────

    /// Select a voice by language code or voice name.
    ///
    /// Equivalent to `espeak_ng_SetVoiceByName(name)`.
    pub fn set_voice(&mut self, lang: &str) {
        self.voice_spec = VoiceSpec::by_name(lang);
    }

    /// Select a voice by detailed criteria.
    ///
    /// Equivalent to `espeak_ng_SetVoiceByProperties(voice_selector)`.
    pub fn set_voice_by_spec(&mut self, spec: VoiceSpec) {
        self.voice_spec = spec;
    }

    /// Set a synthesis parameter (absolute value).
    ///
    /// Equivalent to `espeak_ng_SetParameter(parameter, value, /*relative=*/0)`.
    ///
    /// # Panics
    /// Does not panic; silently clamps out-of-range values.
    pub fn set_parameter(&mut self, param: Parameter, value: i32) {
        match param {
            Parameter::Rate   => self.rate      = value.clamp(80, 450) as u32,
            Parameter::Volume => self.volume    = value.clamp(0, 200)  as u32,
            Parameter::Pitch  => self.pitch     = value.clamp(0, 100)  as u32,
            Parameter::Range  => self.range     = value.clamp(0, 100)  as u32,
            Parameter::WordGap => self.word_gap = value,
            Parameter::Punctuation | Parameter::Capitals => { /* TODO */ }
        }
    }

    /// Set a parameter relative to its current value.
    ///
    /// Equivalent to `espeak_ng_SetParameter(parameter, value, /*relative=*/1)`.
    pub fn set_parameter_relative(&mut self, param: Parameter, delta: i32) {
        let current = self.get_parameter(param);
        self.set_parameter(param, current + delta);
    }

    /// Get the current value of a parameter.
    ///
    /// Equivalent to `espeak_GetParameter(parameter, /*current=*/1)`.
    pub fn get_parameter(&self, param: Parameter) -> i32 {
        match param {
            Parameter::Rate      => self.rate     as i32,
            Parameter::Volume    => self.volume   as i32,
            Parameter::Pitch     => self.pitch    as i32,
            Parameter::Range     => self.range    as i32,
            Parameter::WordGap   => self.word_gap,
            Parameter::Punctuation | Parameter::Capitals => 0,
        }
    }

    /// Return the sample rate of the synthesizer in Hz.
    ///
    /// Always returns 22 050 for the current implementation.
    ///
    /// Equivalent to `espeak_ng_GetSampleRate()`.
    pub const fn sample_rate(&self) -> u32 {
        22050
    }

    // ── Text → phonemes ──────────────────────────────────────────────────

    /// Translate text to an IPA phoneme string.
    ///
    /// Equivalent to `espeak_TextToPhonemes()` with `espeakPHONEMES_IPA` flag,
    /// or running:
    /// ```shell
    /// espeak-ng -v en -q --ipa "hello"
    /// ```
    ///
    /// # Errors
    /// Returns [`Error::VoiceNotFound`] if the voice data files cannot be
    /// found in the configured data directory.
    ///
    /// # Example
    /// ```rust,no_run
    /// let mut engine = espeak_ng::EspeakNg::new("en")?;
    /// assert_eq!(engine.text_to_phonemes("hello world")?, "hɛlˈəʊ wˈɜːld");
    /// # Ok::<(), espeak_ng::Error>(())
    /// ```
    pub fn text_to_phonemes(&self, text: &str) -> Result<String> {
        let translator = self.make_translator()?;
        translator.text_to_ipa(text)
    }

    // ── Synthesis ────────────────────────────────────────────────────────

    /// Synthesize text to 16-bit PCM audio.
    ///
    /// Returns `(samples, sample_rate_hz)`.  The sample rate is always
    /// 22 050 Hz.  Samples are signed 16-bit mono.
    ///
    /// Equivalent to `espeak_ng_Synthesize()` in `AUDIO_OUTPUT_SYNCHRONOUS`
    /// mode (all audio returned at once, no callback).
    ///
    /// # Errors
    /// Returns [`Error::VoiceNotFound`] if the phoneme data files are absent.
    ///
    /// # Example
    /// ```rust,no_run
    /// let engine = espeak_ng::EspeakNg::new("en")?;
    /// let (samples, rate) = engine.synth("hello world")?;
    /// assert_eq!(rate, 22050);
    /// assert!(!samples.is_empty());
    /// # Ok::<(), espeak_ng::Error>(())
    /// ```
    pub fn synth(&self, text: &str) -> Result<(PcmBuffer, u32)> {
        let translator = self.make_translator()?;
        let mut phdata  = self.load_phdata()?;
        phdata.select_table_by_name(self.voice_spec.effective_lang())
            .map_err(|_| Error::VoiceNotFound(
                self.voice_spec.effective_lang().to_string()
            ))?;

        let codes   = translator.translate_to_codes(text)?;
        let voice   = self.make_voice_params();
        let synth   = Synthesizer::new(voice);
        let samples = synth.synthesize_codes(&codes, &phdata)?;

        Ok((samples, self.sample_rate()))
    }

    /// Synthesize text and also return the associated [`SynthEvent`] stream.
    ///
    /// This is the full-featured equivalent of `espeak_ng_Synthesize()` with
    /// an `espeak_SetSynthCallback` registered, providing word/sentence timing
    /// alongside the PCM.
    ///
    /// # Note
    /// In the current implementation the event stream contains only
    /// [`EventKind::MsgTerminated`].  Full word/sentence timing is on the
    /// roadmap.
    pub fn synth_with_events(&self, text: &str) -> Result<(PcmBuffer, u32, Vec<SynthEvent>)> {
        let (samples, rate) = self.synth(text)?;
        let events = vec![SynthEvent {
            kind:             EventKind::MsgTerminated,
            text_position:    text.len(),
            audio_position_ms: samples.len() as u32 * 1000 / rate,
        }];
        Ok((samples, rate, events))
    }

    // ── Info ─────────────────────────────────────────────────────────────

    /// Return the version string of this port.
    ///
    /// Equivalent to `espeak_Info(NULL)`.
    pub fn version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    /// Return the path to the espeak-ng data directory in use.
    pub fn data_path(&self) -> &Path {
        &self.data_dir
    }

    /// Return the currently active voice specification.
    pub fn current_voice(&self) -> &VoiceSpec {
        &self.voice_spec
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn make_translator(&self) -> Result<Translator> {
        Translator::new(
            self.voice_spec.effective_lang(),
            Some(&self.data_dir),
        )
    }

    fn load_phdata(&self) -> Result<PhonemeData> {
        PhonemeData::load(&self.data_dir)
            .map_err(|_| Error::VoiceNotFound(
                format!("phoneme data not found in {}", self.data_dir.display())
            ))
    }

    fn make_voice_params(&self) -> VoiceParams {
        // Map rate (wpm) to speed_percent:  175 wpm → 100 %
        let speed_percent = (self.rate * 100 / 175).clamp(50, 400);
        // Map pitch (0–100) to Hz:  50 → 118 Hz, 0 → 59 Hz, 100 → 177 Hz
        let pitch_hz = 59 + self.pitch * 118 / 100;
        // Map volume (0–200) to amplitude (0–100)
        let amplitude = (self.volume / 2).clamp(0, 100);

        VoiceParams {
            speed_percent,
            pitch_hz,
            amplitude,
            ..VoiceParams::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Fluent builder for [`EspeakNg`].
///
/// Obtain one from [`EspeakNg::builder()`].
#[derive(Debug)]
pub struct Builder {
    lang:     String,
    rate:     u32,
    volume:   u32,
    pitch:    u32,
    range:    u32,
    data_dir: Option<PathBuf>,
}

impl Default for Builder {
    fn default() -> Self {
        Builder {
            lang:     "en".to_string(),
            rate:     175,
            volume:   100,
            pitch:    50,
            range:    50,
            data_dir: None,
        }
    }
}

impl Builder {
    /// Select a language / voice by BCP-47 tag (e.g. `"en"`, `"de"`, `"fr"`).
    pub fn voice(mut self, lang: &str) -> Self {
        self.lang = lang.to_string();
        self
    }

    /// Speaking rate in words-per-minute (80–450, default 175).
    pub fn rate(mut self, wpm: u32) -> Self {
        self.rate = wpm.clamp(80, 450);
        self
    }

    /// Output volume (0–200, default 100).
    pub fn volume(mut self, vol: u32) -> Self {
        self.volume = vol.clamp(0, 200);
        self
    }

    /// Base pitch (0–100, default 50).
    pub fn pitch(mut self, pitch: u32) -> Self {
        self.pitch = pitch.clamp(0, 100);
        self
    }

    /// Pitch range / intonation depth (0–100, default 50).
    pub fn range(mut self, range: u32) -> Self {
        self.range = range.clamp(0, 100);
        self
    }

    /// Override the espeak-ng data directory.
    ///
    /// Defaults to `ESPEAK_DATA_PATH` environment variable, then
    /// `/usr/share/espeak-ng-data`.
    pub fn data_dir(mut self, path: &Path) -> Self {
        self.data_dir = Some(path.to_path_buf());
        self
    }

    /// Build the engine.
    ///
    /// # Errors
    /// Returns [`Error::DataPath`] if the data directory does not exist.
    pub fn build(self) -> Result<EspeakNg> {
        let dir = self.data_dir
            .unwrap_or_else(|| PathBuf::from(default_data_dir()));

        let mut engine = EspeakNg::with_data_dir(&self.lang, &dir)?;
        engine.rate   = self.rate;
        engine.volume = self.volume;
        engine.pitch  = self.pitch;
        engine.range  = self.range;
        Ok(engine)
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_default_values() {
        let b = Builder::default();
        assert_eq!(b.lang,   "en");
        assert_eq!(b.rate,   175);
        assert_eq!(b.pitch,  50);
        assert_eq!(b.volume, 100);
    }

    #[test]
    fn voice_spec_by_name() {
        let v = VoiceSpec::by_name("de");
        assert_eq!(v.effective_lang(), "de");
    }

    #[test]
    fn voice_spec_builder() {
        let v = VoiceSpec::builder()
            .language("fr")
            .gender(Gender::Female)
            .age(25)
            .build();
        assert_eq!(v.language.as_deref(), Some("fr"));
        assert_eq!(v.gender, Gender::Female);
        assert_eq!(v.age, 25);
    }

    #[test]
    fn engine_new_missing_dir() {
        let res = EspeakNg::with_data_dir("en", Path::new("/nonexistent/path"));
        assert!(res.is_err());
    }

    #[test]
    fn engine_sample_rate() {
        let data_dir = PathBuf::from(default_data_dir());
        if !data_dir.exists() { return; }
        let engine = EspeakNg::new("en").unwrap();
        assert_eq!(engine.sample_rate(), 22050);
    }

    #[test]
    fn engine_set_get_parameter() {
        let data_dir = PathBuf::from(default_data_dir());
        if !data_dir.exists() { return; }
        let mut engine = EspeakNg::new("en").unwrap();

        engine.set_parameter(Parameter::Rate, 200);
        assert_eq!(engine.get_parameter(Parameter::Rate), 200);

        engine.set_parameter(Parameter::Pitch, 70);
        assert_eq!(engine.get_parameter(Parameter::Pitch), 70);

        // Clamp behaviour
        engine.set_parameter(Parameter::Rate, 9999);
        assert_eq!(engine.get_parameter(Parameter::Rate), 450);

        engine.set_parameter(Parameter::Rate, -9999);
        assert_eq!(engine.get_parameter(Parameter::Rate), 80);
    }

    #[test]
    fn engine_set_parameter_relative() {
        let data_dir = PathBuf::from(default_data_dir());
        if !data_dir.exists() { return; }
        let mut engine = EspeakNg::new("en").unwrap();
        engine.set_parameter(Parameter::Pitch, 50);
        engine.set_parameter_relative(Parameter::Pitch, 10);
        assert_eq!(engine.get_parameter(Parameter::Pitch), 60);
    }

    #[test]
    fn engine_text_to_phonemes_en() {
        let data_dir = PathBuf::from(default_data_dir());
        if !data_dir.join("en_dict").exists() { return; }
        let engine = EspeakNg::new("en").unwrap();
        let ipa = engine.text_to_phonemes("hello").unwrap();
        assert!(ipa.contains('h'), "expected IPA with 'h', got: {ipa}");
    }

    #[test]
    fn engine_synth_returns_samples() {
        let data_dir = PathBuf::from(default_data_dir());
        if !data_dir.join("en_dict").exists() { return; }
        let engine = EspeakNg::new("en").unwrap();
        let (samples, rate) = engine.synth("hello").unwrap();
        assert_eq!(rate, 22050);
        assert!(!samples.is_empty());
    }

    #[test]
    fn engine_version_nonempty() {
        assert!(!EspeakNg::version().is_empty());
    }

    #[test]
    fn engine_builder_chain() {
        let data_dir = PathBuf::from(default_data_dir());
        if !data_dir.exists() { return; }
        let engine = EspeakNg::builder()
            .voice("en")
            .rate(200)
            .pitch(60)
            .volume(80)
            .build()
            .unwrap();
        assert_eq!(engine.get_parameter(Parameter::Rate),   200);
        assert_eq!(engine.get_parameter(Parameter::Pitch),   60);
        assert_eq!(engine.get_parameter(Parameter::Volume),  80);
    }
}
