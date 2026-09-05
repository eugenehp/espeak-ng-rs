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
use crate::translate::{default_data_dir, normalize_voice_tag, Translator};

/// Controls formatting of IPA returned by [`EspeakNg::text_to_phonemes_with_options`].
///
/// The default preserves the output format of [`EspeakNg::text_to_phonemes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextToPhonemesOptions {
    /// Reinsert input punctuation into the generated IPA.
    pub preserve_punctuation: bool,
    /// Replace clause-separating newlines with spaces.
    pub flatten_clauses: bool,
    /// Interpret SSML markup in the input (equivalent to the `-m` CLI flag).
    pub markup: bool,
}

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
    /// `+variant` acoustic modifier parsed from the voice name (`en+f3` →
    /// `Some("f3")`), corresponding to a file in `espeak-ng-data/voices/!v/`.
    /// The variant's acoustic parameters are not yet applied to synthesis.
    pub variant: Option<String>,
}

impl VoiceSpec {
    /// Create a voice spec that selects by language code only.
    ///
    /// Equivalent to calling `espeak_ng_SetVoiceByName("en")`.
    pub fn by_name(lang: &str) -> Self {
        let (base, variant) = crate::translate::split_voice_variant(lang);
        VoiceSpec {
            language: Some(normalize_voice_tag(base)),
            variant: variant.map(|v| v.to_string()),
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
    /// Set the language tag (e.g. `"en"`, `"de"`, `"fr"`).  A `+variant`
    /// suffix (`en+f3`) is split off into [`VoiceSpec::variant`].
    pub fn language(mut self, lang: &str) -> Self {
        let (base, variant) = crate::translate::split_voice_variant(lang);
        self.spec.language = Some(normalize_voice_tag(base));
        if variant.is_some() {
            self.spec.variant = variant.map(|v| v.to_string());
        }
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
    /// An SSML `<mark name="…"/>` reference point was reached.  Payload is the
    /// mark name.  Mirrors `espeak_EVENT_MARK`.
    Mark(String),
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
    /// Capital-letter indication (`-k`): 0 = none, 2 = announce "capital".
    capitals:   u8,
    /// Announce punctuation by name (`--punct`): `None` = off, `Some(empty)` =
    /// all, `Some(chars)` = only those.
    punct:      Option<Vec<char>>,
    /// Suppress the end-of-utterance trailing silence (`-z`).
    no_final_pause: bool,
    /// Seed for the unvoiced-noise generator (`espeak_ng_SetRandSeed`, `-D`).
    rand_seed: u32,
    /// Path to the espeak-ng data directory.
    data_dir:   PathBuf,
    /// Registered sound icons (external WAV clips spliced into the output).
    soundicons: crate::soundicon::SoundIconTable,
    /// When set, `synth` interprets SSML `<audio src="…">` markup.
    markup:     bool,
    /// Post-synthesis tempo scale (libsonic, `1.0` = off): `>1` faster/shorter,
    /// pitch preserved.  Applied to the final PCM in [`synth`](Self::synth).
    post_tempo: f64,
    /// Post-synthesis pitch scale (libsonic, `1.0` = off): `>1` higher.  Unlike
    /// the base-F0 `pitch`, this resamples, so formants shift too (an effect
    /// voice — chipmunk/robot).  Applied to the final PCM.
    post_pitch: f64,
}

/// Drop trailing near-silence from a PCM run.  Used when splicing sound icons so
/// each spliced text run's own end-of-utterance silence doesn't accumulate into
/// long internal gaps.
fn trim_trailing_silence(pcm: &mut Vec<i16>) {
    const THRESHOLD: u16 = 64; // ≈ -54 dBFS
    while matches!(pcm.last(), Some(&s) if s.unsigned_abs() <= THRESHOLD) {
        pcm.pop();
    }
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
            capitals:   0,
            punct:      None,
            no_final_pause: false,
            rand_seed: 0,
            data_dir:   data_dir.to_path_buf(),
            soundicons: crate::soundicon::SoundIconTable::new(22_050),
            markup:     false,
            post_tempo: 1.0,
            post_pitch: 1.0,
        })
    }

    /// Set a post-synthesis libsonic **tempo** scale (`1.0` = off): the final PCM
    /// is time-scaled by `factor` (`>1` faster/shorter) with pitch preserved.
    /// Builder-style; returns `self`.
    pub fn with_post_tempo(mut self, factor: f64) -> Self {
        self.post_tempo = factor;
        self
    }

    /// Set a post-synthesis libsonic **pitch** scale (`1.0` = off): the final PCM
    /// is pitch-shifted by `factor` (`>1` higher) at unchanged duration.  Note
    /// this resamples, so formants shift too (an effect voice).  Returns `self`.
    pub fn with_post_pitch(mut self, factor: f64) -> Self {
        self.post_pitch = factor;
        self
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

    /// Register a punctuation character to play an external WAV clip (a "sound
    /// icon") instead of being spoken.
    ///
    /// Mirrors eSpeak NG's per-voice `soundicon <char> <file>` directive.  The
    /// file is loaded immediately (and resampled / down-mixed to the engine
    /// rate); relative paths resolve against `<data>/soundicons`.  Sound icons
    /// are then spliced into [`synth`](Self::synth) output wherever the character
    /// appears.
    pub fn add_soundicon(&mut self, ch: char, path: impl AsRef<Path>) -> Result<()> {
        let base = self.soundicon_base_dir();
        self.soundicons
            .register_punct(ch, path.as_ref().to_path_buf(), &base)?;
        Ok(())
    }

    /// Enable or disable SSML markup interpretation in [`synth`](Self::synth).
    ///
    /// When on, `<audio src="…">fallback</audio>` plays the referenced WAV as a
    /// sound icon (speaking the fallback text if it can't be loaded).  Off by
    /// default.
    pub fn set_markup(&mut self, on: bool) {
        self.markup = on;
    }

    /// Base directory for resolving relative sound-icon paths
    /// (`<data>/soundicons`, mirroring C's `path_home/soundicons`).
    fn soundicon_base_dir(&self) -> PathBuf {
        self.data_dir.join("soundicons")
    }

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

    /// Select the best-matching installed voice for a property query (language /
    /// name / gender), scored by [`crate::voices::find_voice`].  Returns `true`
    /// if a voice matched (and was selected).  This resolves voice *names* and
    /// locale fall-backs that [`set_voice`](Self::set_voice) does not — e.g.
    /// `name = "English (Great Britain)"` → the `en-gb` voice.
    ///
    /// Equivalent to `espeak_ng_SetVoiceByProperties(voice_selector)`.
    pub fn set_voice_by_query(&mut self, query: &crate::voices::VoiceQuery) -> bool {
        let voices = crate::voices::list_voices(&self.data_dir);
        match crate::voices::find_voice(&voices, query) {
            Some(v) => {
                self.voice_spec = VoiceSpec::by_name(&v.language);
                true
            }
            None => false,
        }
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
            // `Punctuation`: 0 = off, non-zero = announce all punctuation.  Use
            // [`set_punctuation_list`] to restrict to specific characters.
            Parameter::Punctuation => {
                self.punct = (value != 0).then(Vec::new);
            }
            Parameter::Capitals => self.capitals = value.clamp(0, 255) as u8,
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
            Parameter::Punctuation => self.punct.is_some() as i32,
            Parameter::Capitals    => self.capitals as i32,
        }
    }

    /// Restrict `--punct` to specific characters, or clear the restriction.
    ///
    /// `Some(chars)` announces only those punctuation characters by name;
    /// `Some(empty)` announces all; `None` disables announcement.  Mirrors
    /// `espeak_ng_SetPunctuationList`.
    pub fn set_punctuation_list(&mut self, chars: Option<Vec<char>>) {
        self.punct = chars;
    }

    /// Suppress (or restore) the end-of-utterance trailing silence (`-z`).
    pub fn set_no_final_pause(&mut self, value: bool) {
        self.no_final_pause = value;
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

    /// Translate text to IPA and report how the last clause ended.
    ///
    /// Rust form of upstream's `espeak_TextToPhonemesWithTerminator` (1.53.0).
    /// See [`crate::translate::Translator::text_to_ipa_with_terminator`].
    pub fn text_to_phonemes_with_terminator(
        &self,
        text: &str,
    ) -> Result<(String, crate::translate::ClauseTerminator)> {
        let translator = self.make_translator()?;
        translator.text_to_ipa_with_terminator(text)
    }

    /// Translate text to IPA with configurable output formatting.
    ///
    /// Stress marks are retained in all modes. Use [`TextToPhonemesOptions::default`]
    /// for output equivalent to [`EspeakNg::text_to_phonemes`].
    pub fn text_to_phonemes_with_options(
        &self,
        text: &str,
        options: TextToPhonemesOptions,
    ) -> Result<String> {
        let translator = self.make_translator()?;
        translator.text_to_ipa_with_options(
            text,
            options.preserve_punctuation,
            options.flatten_clauses,
            options.markup,
            true,
        )
    }

    /// Translate text to IPA formatted for phonemizer-based TTS pipelines.
    ///
    /// Input punctuation is retained and clause separators are flattened to
    /// spaces.
    pub fn text_to_phonemes_phonemizer(&self, text: &str) -> Result<String> {
        self.text_to_phonemes_with_options(
            text,
            TextToPhonemesOptions {
                preserve_punctuation: true,
                flatten_clauses: true,
                markup: false,
            },
        )
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
        let rate = self.sample_rate();

        // Fast path: no sound icons and no markup → single utterance, unchanged.
        if self.soundicons.is_empty() && !self.markup {
            return Ok((self.apply_post_fx(self.synth_text(text)?), rate));
        }

        // Split into speakable runs and sound-icon events, then render each:
        // text runs go through the normal pipeline; icons are spliced in with a
        // 10 ms lead-in pause (mirroring C's `DoPause(10)` before `EMBED_I`).
        let mut table = self.soundicons.clone();
        let base = self.soundicon_base_dir();
        let pieces = crate::soundicon::split_pieces(text, &mut table, &base, self.markup);
        let pause = rate as usize / 100;

        let mut out: PcmBuffer = Vec::new();
        let n_pieces = pieces.len();
        for (pi, piece) in pieces.into_iter().enumerate() {
            let is_last = pi + 1 == n_pieces;
            match piece {
                crate::soundicon::Piece::Text(t) => {
                    // In markup mode the run still holds SSML tags (all but
                    // `<audio>`, which was already extracted); process them to
                    // plain speech text so they are not spoken literally — strips
                    // tags, decodes entities, applies `<say-as>` spelling/`<sub>`
                    // and the interpret modes (ordinal/date/time).
                    let t = if self.markup {
                        crate::translate::ssml_to_speech_text(&t, self.voice_spec.effective_lang())
                    } else {
                        t
                    };
                    if !t.trim().is_empty() {
                        let mut s = self.synth_text(&t)?;
                        // Each text run carries its own end-of-utterance trailing
                        // silence; drop it for non-final runs so splicing icons in
                        // doesn't accumulate long internal gaps.
                        if !is_last {
                            trim_trailing_silence(&mut s);
                        }
                        out.extend_from_slice(&s);
                    }
                }
                crate::soundicon::Piece::Icon(idx) => {
                    out.extend(std::iter::repeat(0i16).take(pause));
                    out.extend(crate::soundicon::scale_icon_samples(table.samples(idx)));
                }
            }
        }
        Ok((self.apply_post_fx(out), rate))
    }

    /// Apply the post-synthesis libsonic tempo/pitch scaling to a rendered PCM
    /// buffer.  A no-op (returns the buffer unchanged) when both factors are
    /// `1.0`, so the default path is byte-identical.
    fn apply_post_fx(&self, pcm: PcmBuffer) -> PcmBuffer {
        let rate = self.sample_rate();
        let pcm = if (self.post_tempo - 1.0).abs() > 1e-3 {
            crate::synthesize::tempo::change_tempo(&pcm, self.post_tempo, rate)
        } else {
            pcm
        };
        if (self.post_pitch - 1.0).abs() > 1e-3 {
            crate::synthesize::tempo::change_pitch(&pcm, self.post_pitch, rate)
        } else {
            pcm
        }
    }

    /// Synthesize a plain (icon-free) text run through the normal pipeline.
    fn synth_text(&self, text: &str) -> Result<PcmBuffer> {
        let translator = self.make_translator()?;
        let mut phdata  = self.load_phdata()?;
        // A regional locale may have no phoneme table of its own
        // (`en-gb-scotland`, `pt-br`); `select_phoneme_table` honours the
        // voice's `phonemes` directive and then falls back to the base
        // subtag, where `select_table_by_name` simply fails.
        crate::translate::select_phoneme_table(
            &mut phdata,
            &self.data_dir,
            self.voice_spec.effective_lang(),
        )
        .map_err(|_| Error::VoiceNotFound(self.voice_spec.effective_lang().to_string()))?;

        let codes   = translator.translate_to_codes(text)?;
        // Inline embedded commands (`\x01…P/S/A`) adjust the utterance levels.
        let voice   = self.make_voice_params_levels(self.rate, self.pitch, self.volume);
        let synth   = Synthesizer::new(voice);
        synth.synthesize_codes(&codes, &phdata)
    }

    /// Synthesize text and also return the associated [`SynthEvent`] stream —
    /// the equivalent of `espeak_ng_Synthesize()` with an `espeak_SetSynthCallback`
    /// registered, providing word/sentence timing alongside the PCM.
    ///
    /// For the plain-text path this yields a `Sentence` event, one
    /// [`EventKind::Word`] per spoken word (with its start time in the audio and a
    /// best-effort source `text_position`), and `End`/`MsgTerminated`.  When
    /// sound icons or SSML markup are present the runs are spliced, so only
    /// message-level events are produced.
    ///
    /// # Note on `text_position`
    /// The audio timing is exact; the source char offset is derived by scanning
    /// the input for word starts and pairing by index, so it can drift where a
    /// source token expands to several spoken words (numbers, abbreviations).
    pub fn synth_with_events(&self, text: &str) -> Result<(PcmBuffer, u32, Vec<SynthEvent>)> {
        let rate = self.sample_rate();
        let ms = |s: usize| (s as u64 * 1000 / rate.max(1) as u64) as u32;

        // Icon/markup splicing reorders audio; word-level timing isn't tracked
        // through it, so fall back to a message-terminated event only — plus any
        // SSML `<mark>` events, whose audio time is interpolated from their text
        // position (exact per-word timing isn't tracked through the markup path).
        if !self.soundicons.is_empty() || self.markup {
            let (samples, _) = self.synth(text)?;
            let end_ms = ms(samples.len());
            let mut events = Vec::new();
            if self.markup {
                let (_segs, marks) = crate::translate::ssml::process_markup_with_marks(text);
                let total = crate::translate::ssml::strip_markup(text).chars().count().max(1);
                for m in marks {
                    let at = (m.position as u64 * end_ms as u64 / total as u64) as u32;
                    events.push(SynthEvent {
                        kind: EventKind::Mark(m.name),
                        text_position: m.position,
                        audio_position_ms: at,
                    });
                }
            }
            events.push(SynthEvent {
                kind: EventKind::MsgTerminated,
                text_position: text.len(),
                audio_position_ms: end_ms,
            });
            return Ok((samples, rate, events));
        }

        let (samples, word_marks, sentence_marks) = self.synth_text_with_marks(text)?;
        // The tempo/pitch post-pass changes the buffer length, so the sample
        // offsets recorded during synthesis no longer address the audio the
        // caller receives — rescale them (upstream #2470: "rescale event sample
        // positions after libsonic compresses the buffer").  `synth()` already
        // applies this pass, so without it the event path also returned audio
        // that differed from `synth()` for the same text.
        let raw_len = samples.len();
        let samples = self.apply_post_fx(samples);
        let time_scale = if raw_len == 0 {
            1.0
        } else {
            samples.len() as f64 / raw_len as f64
        };
        // Marks were recorded against the pre-scaling buffer; the final length
        // is already in post-fx samples.
        let mark_ms = |s: usize| ms((s as f64 * time_scale) as usize);
        let word_offsets = word_char_offsets(text);
        let end_ms = ms(samples.len());

        let events = build_events(text, &word_offsets, &word_marks, &sentence_marks, mark_ms, end_ms);
        Ok((samples, rate, events))
    }

    /// Choose the seed for the unvoiced-noise generator
    /// (`espeak_ng_SetRandSeed`; the CLI's `-D`).
    ///
    /// Unlike C — which seeds from `time(NULL)` at startup and uses `-D` to pin
    /// the seed to 1 — this port never seeds from the clock, so its output is
    /// already reproducible run to run.  Setting a seed therefore *selects* a
    /// noise sequence rather than making one repeatable; `1` reproduces what
    /// upstream's `-D` uses.
    pub fn set_rand_seed(&mut self, seed: u32) {
        self.rand_seed = seed;
    }

    /// Synthesize several utterances in parallel across the rayon thread pool.
    ///
    /// Each utterance is rendered independently, so the results are exactly what
    /// calling [`synth`](Self::synth) on each in turn would produce — this only
    /// changes how long it takes.  Results are returned in input order, with a
    /// per-item `Result` so one bad input doesn't sink the batch.
    ///
    /// Requires the `parallel` feature.
    ///
    /// ```no_run
    /// # use espeak_ng::EspeakNg;
    /// # #[cfg(feature = "parallel")] {
    /// let engine = EspeakNg::new("en")?;
    /// let results = engine.synth_many(&["one", "two", "three"]);
    /// assert_eq!(results.len(), 3);
    /// # }
    /// # Ok::<(), espeak_ng::Error>(())
    /// ```
    #[cfg(feature = "parallel")]
    pub fn synth_many<S>(&self, texts: &[S]) -> Vec<Result<(PcmBuffer, u32)>>
    where
        S: AsRef<str> + Sync,
    {
        use rayon::prelude::*;
        texts.par_iter().map(|t| self.synth(t.as_ref())).collect()
    }

    /// Synthesize `text` incrementally *and* deliver its events.
    ///
    /// Like [`synth_streaming`](Self::synth_streaming), but the callback also
    /// receives the utterance's [`SynthEvent`]s — non-empty only on the final
    /// chunk, since word and sentence timings are not known until the whole
    /// utterance is rendered.  This is what [`AsyncSynth`] uses, so a queued
    /// utterance starts producing audio after its first clause rather than after
    /// its last.
    ///
    /// [`AsyncSynth`]: crate::async_synth::AsyncSynth
    pub fn synth_streaming_with_events<F>(&self, text: &str, mut callback: F) -> Result<u32>
    where
        F: FnMut(&[i16], bool, &[SynthEvent]) -> bool,
    {
        let rate = self.sample_rate();
        let ms = |s: usize| (s as u64 * 1000 / rate.max(1) as u64) as u32;
        let post_fx = (self.post_tempo - 1.0).abs() > 1e-3 || (self.post_pitch - 1.0).abs() > 1e-3;

        // The same cases `synth_streaming` can't split, plus anything needing
        // the post-fx pass over the whole buffer.
        if !self.soundicons.is_empty() || self.markup || post_fx {
            let (samples, _, events) = self.synth_with_events(text)?;
            callback(&samples, true, &events);
            return Ok(rate);
        }

        let translator = self.make_translator()?;
        let mut phdata = self.load_phdata()?;
        crate::translate::select_phoneme_table(
            &mut phdata,
            &self.data_dir,
            self.voice_spec.effective_lang(),
        )
        .map_err(|_| Error::VoiceNotFound(self.voice_spec.effective_lang().to_string()))?;
        let codes = translator.translate_to_codes(text)?;
        let synth = Synthesizer::new(self.make_voice_params_levels(self.rate, self.pitch, self.volume));

        let mut emitted = 0usize;
        let mut stopped = false;
        let (tail, word_marks, sentence_marks) = {
            let mut sink = |chunk: &[i16], _final: bool| -> bool {
                emitted += chunk.len();
                let stop = callback(chunk, false, &[]);
                stopped |= stop;
                stop
            };
            synth.synthesize_codes_streaming_with_marks(&codes, &phdata, &mut sink)?
        };
        if stopped {
            return Ok(rate);
        }

        let end_ms = ms(emitted + tail.len());
        let word_offsets = word_char_offsets(text);
        let events =
            build_events(text, &word_offsets, &word_marks, &sentence_marks, ms, end_ms);
        callback(&tail, true, &events);
        Ok(rate)
    }

    /// Synthesize `text`, delivering the audio **incrementally** — one callback
    /// per clause, as soon as that clause is rendered — instead of returning the
    /// whole utterance at once.
    ///
    /// The callback receives `(samples, is_final)` and returns `true` to stop.
    /// Concatenating every chunk gives exactly what [`synth`](Self::synth)
    /// returns, so this is a pure latency win: the first audio is ready after
    /// the first clause rather than after the last.  Returns the sample rate.
    ///
    /// Falls back to a single final chunk when the text needs the non-streaming
    /// path (sound icons or SSML markup, which splice audio out of order).
    ///
    /// ```no_run
    /// # use espeak_ng::EspeakNg;
    /// let engine = EspeakNg::new("en")?;
    /// let mut first_chunk_samples = 0;
    /// engine.synth_streaming("One. Two. Three.", |samples, _is_final| {
    ///     if first_chunk_samples == 0 { first_chunk_samples = samples.len(); }
    ///     false // keep going
    /// })?;
    /// # Ok::<(), espeak_ng::Error>(())
    /// ```
    pub fn synth_streaming<F>(&self, text: &str, mut callback: F) -> Result<u32>
    where
        F: FnMut(&[i16], bool) -> bool,
    {
        let rate = self.sample_rate();
        let post_fx = (self.post_tempo - 1.0).abs() > 1e-3 || (self.post_pitch - 1.0).abs() > 1e-3;

        // Icon/markup splicing reorders audio, and the tempo/pitch post-pass
        // works on the whole buffer — neither can be delivered clause by clause,
        // so those cases fall back to one final chunk.
        if !self.soundicons.is_empty() || self.markup || post_fx {
            let (samples, _) = self.synth(text)?;
            callback(&samples, true);
            return Ok(rate);
        }

        let translator = self.make_translator()?;
        let mut phdata = self.load_phdata()?;
        crate::translate::select_phoneme_table(
            &mut phdata,
            &self.data_dir,
            self.voice_spec.effective_lang(),
        )
        .map_err(|_| Error::VoiceNotFound(self.voice_spec.effective_lang().to_string()))?;

        let codes = translator.translate_to_codes(text)?;
        let synth = Synthesizer::new(self.make_voice_params_levels(self.rate, self.pitch, self.volume));
        synth.synthesize_codes_streaming(&codes, &phdata, &mut callback)?;
        Ok(rate)
    }

    /// Plain-text render (like [`synth_text`](Self::synth_text)) that also
    /// returns each spoken word's and sentence's start sample offset.
    fn synth_text_with_marks(&self, text: &str) -> Result<(PcmBuffer, Vec<usize>, Vec<usize>)> {
        let translator = self.make_translator()?;
        let mut phdata  = self.load_phdata()?;
        crate::translate::select_phoneme_table(
            &mut phdata,
            &self.data_dir,
            self.voice_spec.effective_lang(),
        )
        .map_err(|_| Error::VoiceNotFound(self.voice_spec.effective_lang().to_string()))?;

        let codes = translator.translate_to_codes(text)?;
        let voice = self.make_voice_params_levels(self.rate, self.pitch, self.volume);
        let synth = Synthesizer::new(voice);
        synth.synthesize_codes_with_marks(&codes, &phdata)
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
        let mut t = Translator::new(
            self.voice_spec.effective_lang(),
            Some(&self.data_dir),
        )?;
        // Carry the runtime text options through to the audio path so `-g`,
        // `-k` and `--punct` affect synthesis, not just `-x`/`--ipa`.
        t.options.word_gap = self.word_gap;
        t.options.capitals = self.capitals;
        t.options.punct = self.punct.clone();
        Ok(t)
    }

    fn load_phdata(&self) -> Result<PhonemeData> {
        PhonemeData::load(&self.data_dir)
            .map_err(|_| Error::VoiceNotFound(
                format!("phoneme data not found in {}", self.data_dir.display())
            ))
    }


    fn make_voice_params_levels(&self, rate: u32, pitch: u32, volume: u32) -> VoiceParams {
        // Map rate (wpm) to `speed_percent` through upstream's `speed_lookup`
        // curve rather than linearly: `175 / wpm` over-stretched below ~100 wpm,
        // where upstream's curve flattens towards its 80 wpm floor.  A larger
        // duration factor means a *smaller* `speed_percent`.
        let speed_percent = {
            let f = crate::synthesize::setlengths::speed_duration_factor(rate);
            ((100.0 / f).round() as u32).clamp(20, 800)
        };
        // `speed_duration_factor` already carries the calibration, so the
        // synthesizer's `100 / speed_percent` is the whole factor.
        // Base pitch, the way C derives it: the voice's `pitch <pitch1> <pitch2>`
        // line gives `pitch_base = (pitch1 - 9)` Hz (default `82 118` → 73 Hz),
        // scaled by `pitch_adjust_tab[user pitch]` (50 → unity).  The port used
        // to map the parameter straight onto an F0, which put every voice ~35 Hz
        // above upstream once real intonation drove the contour.
        // The default voice's `pitch` line, which C's `VoiceReset` encodes as
        // `pitch_base = 0x47000` = `(80 - 9) << 12` and `pitch_range = 4104` =
        // `(118 - 80) * 108`.  Deriving the reference from these instead of
        // repeating a literal is upstream #2524: the `82` in C's
        // `formant_factor` formula no longer matches its own default.
        const DEFAULT_PITCH_BASE: i32 = 0x47000;
        const DEFAULT_PITCH_RANGE: i32 = 4104;
        const DEFAULT_PITCH1: f64 = ((DEFAULT_PITCH_BASE >> 12) + 9) as f64;
        const DEFAULT_PITCH2: f64 =
            DEFAULT_PITCH1 + (DEFAULT_PITCH_RANGE as f64) / 108.0;

        // A voice file may declare its own `pitch <base> <range>`.
        let (voice_pitch1, voice_pitch2) =
            match crate::voices::voice_pitch(&self.data_dir, self.voice_spec.effective_lang()) {
                Some((p1, p2)) => (p1 as f64, p2 as f64),
                None => (DEFAULT_PITCH1, DEFAULT_PITCH2),
            };
        let mut pitch_hz =
            crate::synthesize::intonation::base_pitch_hz(voice_pitch1, pitch).round() as u32;
        let pitch_range_units = (voice_pitch2 - voice_pitch1) * 108.0;
        // `formant_factor`: a voice pitched away from the default has a
        // correspondingly different vocal-tract length, so its formants shift.
        let mut formant_factor = {
            let factor = (voice_pitch1 - DEFAULT_PITCH1) / DEFAULT_PITCH1;
            ((1.0 + factor / 4.0) * 256.0).round() as i32
        };

        // Apply a `+variant` voice's acoustic modifiers (pitch + per-formant
        // frequency/height/width scaling), if one is selected and its file
        // parses.  The no-variant path is unchanged.
        let mut formant_freq_pct = [100i32; 7];
        let mut formant_height_pct = [100i32; 7];
        let mut formant_width_pct = [100i32; 7];
        let (mut echo_delay_samples, mut echo_amp) = (0usize, 0i32);
        let mut flutter = 0i32;
        let mut stress_amps = [0i32; 8];
        let mut tone_adjust =
            crate::synthesize::wavegen::set_tone_adjust(&crate::synthesize::wavegen::DEFAULT_TONE_POINTS);
        if let Some(var) = &self.voice_spec.variant {
            if let Some(vp) = crate::voices::load_variant(&self.data_dir, var) {
                pitch_hz = variant_scaled_pitch(pitch_hz, &vp);
                if let Some((p1, _)) = vp.pitch {
                    let factor = (p1 as f64 - DEFAULT_PITCH1) / DEFAULT_PITCH1;
                    formant_factor = ((1.0 + factor / 4.0) * 256.0).round() as i32;
                }
                variant_formant_pct(&mut formant_freq_pct, &mut formant_height_pct, &mut formant_width_pct, &vp);
                if let Some((delay_ms, amp)) = vp.echo {
                    if delay_ms > 0 && amp > 0 {
                        echo_delay_samples = (self.sample_rate() as i64 * delay_ms as i64 / 1000).max(0) as usize;
                        echo_amp = amp;
                    }
                }
                flutter = vp.flutter.unwrap_or(0).max(0);
                for (i, &a) in vp.stress_amp.iter().take(8).enumerate() {
                    stress_amps[i] = a.max(0);
                }
                if let Some(points) = vp.tone {
                    tone_adjust = crate::synthesize::wavegen::set_tone_adjust(&points);
                }
            }
        }

        // Map volume (0–200) to amplitude (0–100)
        let amplitude = (volume / 2).clamp(0, 100);

        VoiceParams {
            speed_percent,
            pitch_hz,
            pitch_range_units,
            amplitude,
            no_final_pause: self.no_final_pause,
            rand_seed: self.rand_seed,
            user_rate_wpm: rate,
            user_pitch: pitch,
            user_volume: volume,
            formant_factor,
            // `stressLength <l0…l7>` from the language's voice file, when it
            // declares one (the built-in table is English's).
            stress_lengths: crate::voices::voice_stress_length(
                &self.data_dir,
                self.voice_spec.effective_lang(),
            )
            .and_then(|v| {
                let mut out = [0u32; 8];
                // C reads eight values; a short list leaves the rest at the
                // English default rather than at zero.
                out.copy_from_slice(&crate::synthesize::setlengths::STRESS_LENGTHS_EN);
                for (i, &x) in v.iter().take(8).enumerate() {
                    out[i] = x.max(0) as u32;
                }
                (v.len() >= 8).then_some(out)
            }),
            // `intonation <n>` from the language's voice file, when it sets one
            // (upstream #1466: read the language configuration from data).
            // `intonation <n>` from the language's voice file, when it sets one
            // (upstream #1466: read the language configuration from data).
            intonation_group: crate::voices::voice_intonation(
                &self.data_dir,
                self.voice_spec.effective_lang(),
            )
            .map(usize::from)
            .unwrap_or(1),
            tone_adjust,
            formant_freq_pct,
            formant_height_pct,
            formant_width_pct,
            echo_delay_samples,
            echo_amp,
            flutter,
            stress_amps,
            ..VoiceParams::default()
        }
    }
}

/// Scale a base pitch (Hz) by a variant voice's `pitch <base> <range>`
/// directive, relative to eSpeak's default-voice pitch base (82).
///
/// This is a **directional** mapping — a female variant (`f3`, base 140) raises
/// the pitch, a male one (`m3`, base 80) leaves it about the same — not a
/// byte-exact port of `voices.c`'s pitch math (that is oracle-gated, GAPS §11).
/// A variant with no `pitch` line returns the base unchanged.
fn variant_scaled_pitch(base_hz: u32, variant: &crate::voices::VariantParams) -> u32 {
    /// eSpeak's built-in default voice pitch base.
    const DEFAULT_PITCH_BASE: f64 = 82.0;
    match variant.pitch {
        Some((p1, _)) if p1 > 0 => {
            let scaled = base_hz as f64 * (p1 as f64 / DEFAULT_PITCH_BASE);
            scaled.round().clamp(25.0, 600.0) as u32
        }
        _ => base_hz,
    }
}

/// Byte offsets of each word start in `text` — a word being a maximal run of
/// alphanumeric/apostrophe characters.  Used to give `EventKind::Word` events a
/// best-effort source `text_position` (paired with spoken words by index).
/// Assemble the `Sentence` / `Word` / `End` / `MsgTerminated` event list from
/// the sample offsets recorded during synthesis.
fn build_events(
    text: &str,
    word_offsets: &[usize],
    word_marks: &[usize],
    sentence_marks: &[usize],
    mark_ms: impl Fn(usize) -> u32,
    end_ms: u32,
) -> Vec<SynthEvent> {
    let mut events = Vec::with_capacity(word_marks.len() + sentence_marks.len() + 3);
    // Sentence 0 begins at the start; each later clause boundary opens another.
    events.push(SynthEvent { kind: EventKind::Sentence, text_position: 0, audio_position_ms: 0 });
    for &start in sentence_marks {
        events.push(SynthEvent {
            kind: EventKind::Sentence,
            text_position: 0,
            audio_position_ms: mark_ms(start),
        });
    }
    for (i, &start) in word_marks.iter().enumerate() {
        events.push(SynthEvent {
            kind: EventKind::Word(i as u32),
            text_position: word_offsets.get(i).copied().unwrap_or(text.len()),
            audio_position_ms: mark_ms(start),
        });
    }
    // Keep events in audio order (Sentence markers interleaved with words).
    events.sort_by_key(|e| e.audio_position_ms);
    events.push(SynthEvent { kind: EventKind::End, text_position: text.len(), audio_position_ms: end_ms });
    events.push(SynthEvent {
        kind: EventKind::MsgTerminated,
        text_position: text.len(),
        audio_position_ms: end_ms,
    });
    events
}

fn word_char_offsets(text: &str) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut in_word = false;
    for (i, c) in text.char_indices() {
        let is_word = c.is_alphanumeric() || c == '\'';
        if is_word && !in_word {
            offsets.push(i);
        }
        in_word = is_word;
    }
    offsets
}

/// Copy a `+variant`'s per-formant `formant <i> <freq%> <height%> [width%]`
/// percentages into the `[i32; 7]` scale arrays read by the harmonic synthesizer
/// (F0–F6).  This is the `voice.freq/height/width[i]` mapping of `voices.c`;
/// formants the variant does not mention keep 100 (unity), and each field is
/// clamped to a sane range (25–400 %).
fn variant_formant_pct(
    freq: &mut [i32; 7],
    height: &mut [i32; 7],
    width: &mut [i32; 7],
    variant: &crate::voices::VariantParams,
) {
    for f in &variant.formants {
        let i = f.index as usize;
        if i >= freq.len() {
            continue;
        }
        if f.freq > 0 {
            freq[i] = f.freq.clamp(25, 400);
        }
        if f.height > 0 {
            height[i] = f.height.clamp(25, 400);
        }
        if f.width > 0 {
            width[i] = f.width.clamp(25, 400);
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
        self.lang = normalize_voice_tag(lang);
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
    fn variant_formant_pct_maps_variant_formants() {
        use crate::voices::{VariantFormant, VariantParams};
        let (mut freq, mut height, mut width) = ([100i32; 7], [100i32; 7], [100i32; 7]);
        let vp = VariantParams {
            formants: vec![
                VariantFormant { index: 1, freq: 120, height: 75, width: 150 },
                VariantFormant { index: 2, freq: 135, height: 70, width: 160 },
                VariantFormant { index: 9, freq: 110, height: 70, width: 150 }, // out of range
                VariantFormant { index: 3, freq: 0,   height: 80, width: 0 },   // freq/width 0 → skip those
            ],
            ..Default::default()
        };
        variant_formant_pct(&mut freq, &mut height, &mut width, &vp);
        assert_eq!(freq[1], 120);
        assert_eq!((height[2], width[2]), (70, 160));
        assert_eq!(freq[0], 100, "untouched formant stays unity");
        assert_eq!(freq[3], 100, "freq 0 leaves unity");
        assert_eq!((height[3], width[3]), (80, 100), "height applies, width 0 skipped");
        assert_eq!(freq[6], 100, "out-of-range index ignored (no panic)");
    }

    #[test]
    fn voice_spec_by_name() {
        let v = VoiceSpec::by_name("de");
        assert_eq!(v.effective_lang(), "de");
    }

    #[test]
    fn voice_spec_by_name_normalizes_region_tag() {
        let v = VoiceSpec::by_name("en_US");
        assert_eq!(v.effective_lang(), "en-us");
    }

    #[test]
    fn variant_pitch_scales_directionally() {
        use crate::voices::VariantParams;
        let f3 = VariantParams { pitch: Some((140, 240)), ..Default::default() };
        let m3 = VariantParams { pitch: Some((80, 122)), ..Default::default() };
        let none = VariantParams { pitch: None, ..Default::default() };

        // Female variant raises the base pitch; male stays about the same.
        assert!(variant_scaled_pitch(118, &f3) > 118, "f3 should raise pitch");
        assert!(
            variant_scaled_pitch(118, &f3) > variant_scaled_pitch(118, &m3),
            "f3 must be higher than m3"
        );
        // No `pitch` line → unchanged.
        assert_eq!(variant_scaled_pitch(118, &none), 118);
        // Clamped into a sane vocal range.
        let extreme = VariantParams { pitch: Some((10000, 0)), ..Default::default() };
        assert!(variant_scaled_pitch(118, &extreme) <= 600);
    }

    #[test]
    fn voice_spec_parses_variant_suffix() {
        // `en+f3`: base language selects the data, `+f3` is stored as the
        // acoustic variant (not yet applied).
        let v = VoiceSpec::by_name("en+f3");
        assert_eq!(v.effective_lang(), "en");
        assert_eq!(v.variant.as_deref(), Some("f3"));

        let v = VoiceSpec::by_name("en_US+m3");
        assert_eq!(v.effective_lang(), "en-us");
        assert_eq!(v.variant.as_deref(), Some("m3"));

        let v = VoiceSpec::by_name("de");
        assert_eq!(v.variant, None);

        let v = VoiceSpec::builder().language("fr+whisper").build();
        assert_eq!(v.effective_lang(), "fr");
        assert_eq!(v.variant.as_deref(), Some("whisper"));
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
