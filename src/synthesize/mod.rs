//! Phoneme-list → audio PCM synthesis.
//!
//! Rust port of the eSpeak NG synthesis engine:
//!
//! | C source file | Lines | What it does |
//! |---|---|---|
//! | `synthesize.c` | 1607 | Phoneme interpreter, `InterpretPhoneme` |
//! | `synthdata.c` | 998 | `LoadPhData`, formant data access |
//! | `wavegen.c` | 1486 | Harmonic additive synthesizer, AGC |
//! | `klatt.c` | 1124 | Klatt cascade/parallel filter |
//! | `setlengths.c` | 806 | Phoneme duration: `CalcLengths` |
//! | `phonemelist.c` | 593 | `MakePhonemeList`, stress promotion |
//!
//! # Synthesis pipeline
//!
//! ```text
//! PhonemeCode[]
//!     │  synthesize_codes()          (mod.rs)
//!     ▼
//! SpectSeq[]  (formant frame sequences from phondata)
//!     │  synthesize_frames()         (wavegen.rs)
//!     ▼
//! Vec<i32>  (unnormalised samples)
//!     │  agc_clip()                  (mod.rs)
//!     ▼
//! PcmBuffer  (Vec<i16>, 22 050 Hz, mono)
//! ```
//
// Status: IMPLEMENTED (cascade formant synthesizer)
//
// ## Pipeline
//
// ```text
//   IPA string
//       │  parse_ipa()          (engine.rs)
//       ▼
//   Vec<Segment>  (phoneme + timing + stress)
//       │  synthesize_segments()  (engine.rs)
//       ▼
//   Vec<f64>  (raw samples, un-normalised)
//       │  f64_to_i16()          (engine.rs)
//       ▼
//   PcmBuffer (i16, 22 050 Hz, mono)
// ```
//
// ## Formant Synthesis
//
pub mod envelopes;
pub mod intonation;
pub mod targets;
pub mod engine;
pub mod phondata;
pub mod bytecode;
pub mod wavegen;
pub mod setlengths;
pub mod sample;
pub mod klatt;
pub mod tempo;

use crate::error::{Error, Result};
use crate::phoneme::{PH_PAUSE, PH_VOWEL};

/// Extra duration a `:` length modifier adds to a vowel, in ms — C looks up
/// `phoneme_tab[phonLENGTHEN]->std_length`, which is 50 in every shipped table.
const LENGTHEN_MS: u16 = 50;

/// eSpeak NG appends a fixed ~0.30 s of silence at the end of every clause.
/// Tuned against the C reference via the wav-analysis harness; suppressed by
/// `-z` (`no_final_pause`).
const END_SILENCE_MS: f64 = 300.0;

// ---------------------------------------------------------------------------
// Frame – formant parameters for one time slice
// Mirrors `frame_t` from synthesize.h (64 bytes in C)
// ---------------------------------------------------------------------------

/// One frame of formant parameters.
///
/// The C struct is 64 bytes with hand-packed `unsigned char` arrays.
/// We use named fields and let Rust handle packing.  When reading binary
/// data files with `#[repr(C)]` we will need a separate raw type.
#[derive(Debug, Clone, Default)]
pub struct Frame {
    /// Frame flags (FRFLAG_XXX bitmask)
    pub flags: u16,
    /// Formant frequencies F0–F6 (Hz × 2 in the C code)
    pub ffreq: [i16; 7],
    /// Frame length (units of STEPSIZE = 2.9ms @ 22050 Hz)
    pub length: u8,
    /// RMS amplitude
    pub rms: u8,
    /// Formant heights (amplitude of each formant)
    pub fheight: [u8; 8],
    /// Formant widths / 4, F0–F5
    pub fwidth: [u8; 6],
    /// Right-side formant widths / 4, F0–F2
    pub fright: [u8; 3],
    /// Klatt bandwidth / 2: BNZ, F1, F2, F3
    pub bw: [u8; 4],
    /// Klatt parameters: AV, FNZ, Tilt, Aspr, Skew
    pub klattp: [u8; 5],
    /// Extended Klatt parameters, continuing `klattp` from index 5:
    /// Kopen, AVp, Fric, FricBP, Turb
    pub klattp2: [u8; 5],
    /// Klatt parallel amplitudes, F0–F6
    pub klatt_ap: [u8; 7],
    /// Klatt parallel bandwidths / 2, F0–F6
    pub klatt_bp: [u8; 7],
    /// Pad byte
    pub spare: u8,
}

impl Frame {
    /// The size of the equivalent C struct in bytes.
    pub const C_SIZE: usize = 64;
}

// ---------------------------------------------------------------------------
// Resonator – digital filter for one formant
// Mirrors `RESONATOR` struct from synthesize.h
// ---------------------------------------------------------------------------

/// A second-order IIR resonator (one formant).
///
/// Direct port of the `RESONATOR` C struct + the `Resonator()` macro.
///
/// Coefficients follow the Klatt (1980) convention:
/// ```text
///   y[n] = a·x[n] + b·y[n-1] + c·y[n-2]
/// ```
/// where:
/// ```text
///   r = exp(−π·BW/fs)
///   c = −r²
///   b = 2·r·cos(2π·F/fs)
///   a = 1 − b − c
/// ```
#[derive(Debug, Clone, Default)]
pub struct Resonator {
    /// Feed-forward coefficient `a` (see Klatt 1980).
    pub a:  f64,
    /// First feedback coefficient `b`.
    pub b:  f64,
    /// Second feedback coefficient `c`.
    pub c:  f64,
    /// Delay element `y[n-1]`.
    pub x1: f64,
    /// Delay element `y[n-2]`.
    pub x2: f64,
}

impl Resonator {
    /// Run one sample through the resonator.
    ///
    /// Mirrors the `Resonator(rp, in)` macro from wavegen.c:
    /// ```text
    ///   y = a·in + b·x1 + c·x2;  x2 = x1;  x1 = y;  y
    /// ```
    #[inline]
    pub fn tick(&mut self, input: f64) -> f64 {
        let y = self.a * input + self.b * self.x1 + self.c * self.x2;
        self.x2 = self.x1;
        self.x1 = y;
        y
    }

    /// Reset the filter state (clear delay elements).
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
    }
}

// ---------------------------------------------------------------------------
// Voice parameters
// ---------------------------------------------------------------------------

/// Voice configuration.
///
/// A subset of `voice_t` from voice.h; synthesis-relevant fields only.
#[derive(Debug, Clone)]
pub struct VoiceParams {
    /// Speaking rate multiplier (100 = normal, 200 = double speed)
    pub speed_percent: u32,
    /// Pitch in Hz (the base F0)
    pub pitch_hz: u32,
    /// Pitch range: 0 = monotone, 100 = normal
    pub pitch_range: u32,
    /// The voice's pitch range in espeak's internal units — `(pitch2 - pitch1) *
    /// 108` from the voice file's `pitch <base> <range>` line (default
    /// `pitch 82 118` → 3888).  Scales the intonation module's 0–254 syllable
    /// pitch values into Hz.
    pub pitch_range_units: f64,
    /// `stressLength <l0…l7>` from the language's voice file, in 1/128ths —
    /// twenty shipped languages replace the built-in table this way
    /// (upstream #1466: read the language configuration from data).
    pub stress_lengths: Option<[u32; 8]>,
    /// `langopts.intonation_group` — selects a row of `punct_to_tone`.
    pub intonation_group: usize,
    /// Formant frequency scale factor (100 = normal)
    pub formant_scale: u32,
    /// Sample rate in Hz (always 22050 for espeak-ng)
    pub sample_rate: u32,
    /// Amplitude 0–100
    pub amplitude: u32,
    /// Suppress the end-of-utterance trailing silence (`-z`).
    pub no_final_pause: bool,
    /// Seed for the unvoiced-noise generator (`espeak_ng_SetRandSeed`, the CLI's
    /// `-D`).  `0` keeps the built-in default.
    ///
    /// The port is deterministic either way — unlike C, it never seeds from the
    /// clock — so this exists to *choose* a noise sequence, not to make one
    /// reproducible.
    pub rand_seed: u32,
    /// The utterance's own rate in words per minute (175 = normal), pitch
    /// (0-100, 50 = normal) and volume (0-200, 100 = normal).
    ///
    /// Inline embedded commands (`\x01<n>S`) set these *absolutely*, so the
    /// render loop needs the baseline to turn one into a relative change.
    pub user_rate_wpm: u32,
    pub user_pitch: u32,
    pub user_volume: u32,
    /// `voice->formant_factor` × 256: the nominal formant shift for a voice
    /// pitched away from the default (`AdjustFormants` scales its F2 target by
    /// it).  `256` = no shift.
    pub formant_factor: i32,
    /// The voice's `tone` curve: per-harmonic amplitude adjustment in 8 Hz
    /// steps (`wvoice->tone_adjust`, built by `SetToneAdjust`).
    ///
    /// Every voice starts from `tone_points = 600 170 1200 135 2000 110
    /// 3000 110` — flat to 600 Hz, then tilting down — and a `tone` line in a
    /// voice or `+variant` file replaces it.
    pub tone_adjust: [u8; wavegen::N_TONE_ADJUST],
    /// Render formant runs through the Klatt cascade (`klatt.rs`) instead of the
    /// harmonic path.  Opt-in; the default (harmonic) path is unchanged.
    pub klatt: bool,
    /// Per-formant frequency scaling (percent, 100 = unity) for F0–F6 — the
    /// `voice.freq[i]` of `voices.c`, driven by a `+variant`'s `formant <i>
    /// <freq%>` lines.  All-100 (the default) leaves the harmonic path untouched.
    pub formant_freq_pct: [i32; 7],
    /// Per-formant *height* (amplitude) scaling (percent, 100 = unity) for F0–F6
    /// — `voice.height[i]`, from a `+variant`'s `formant <i> <freq%> <height%>`.
    pub formant_height_pct: [i32; 7],
    /// Per-formant *width* (bandwidth) scaling (percent, 100 = unity) for F0–F6 —
    /// `voice.width[i]`, from a `+variant`'s `formant <i> … <width%>`.
    pub formant_width_pct: [i32; 7],
    /// Echo delay in samples (`+variant` `echo <delay_ms> <amp>` → samples).
    /// `0` disables the echo post-pass (the default).
    pub echo_delay_samples: usize,
    /// Echo feedback amplitude; the delayed sample is scaled by `echo_amp >> 8`
    /// (matching `wavegen.c`'s `(echo_buf[t]*echo_amp)>>8`).  `0` = no echo.
    pub echo_amp: i32,
    /// Pitch flutter strength (`+variant` `flutter <n>`) — a slow pseudo-random
    /// F0 wobble that de-robotises the voice.  `0` (the default) = steady pitch.
    pub flutter: i32,
    /// Per-stress-level amplitude override (`+variant` `stressAmp <a0>…<a7>`, i.e.
    /// `voice.stress_amps[]`), indexed by stress level 0–7.  A `0` entry means
    /// "unset" for that level (fall back to the built-in `STRESS_AMPS_EN`); the
    /// all-zero default therefore leaves every phoneme's amplitude unchanged.
    pub stress_amps: [i32; 8],
}

impl VoiceParams {
    /// True when any formant carries a non-unity freq/height/width scale (so the
    /// frame formants must be pre-scaled before synthesis).
    pub fn formant_scaling_active(&self) -> bool {
        self.formant_freq_pct.iter()
            .chain(self.formant_height_pct.iter())
            .chain(self.formant_width_pct.iter())
            .any(|&p| p != 100)
    }

    /// Amplitude ratio for a `+variant`'s `stressAmp` override at stress `level`,
    /// relative to the built-in `STRESS_AMPS_EN` baseline that `amp_factor`
    /// already bakes in.  Returns exactly `1.0` when unset (`0`) — so the default
    /// path is unchanged — else `stress_amps[level] / STRESS_AMPS_EN[level]`.
    pub fn stress_amp_ratio(&self, level: u8) -> f64 {
        let i = (level as usize).min(7);
        let base = setlengths::STRESS_AMPS_EN[i] as f64;
        if self.stress_amps[i] <= 0 || base <= 0.0 {
            return 1.0;
        }
        self.stress_amps[i] as f64 / base
    }
}

impl Default for VoiceParams {
    fn default() -> Self {
        VoiceParams {
            speed_percent:  100,
            pitch_hz:       118,  // male default
            pitch_range_units: 3888.0, // espeak's default `pitch 82 118`
            stress_lengths: None,
            intonation_group: 1,
            pitch_range:    100,
            formant_scale:  100,
            sample_rate:    22050,
            amplitude:      80,
            no_final_pause: false,
            rand_seed:      0,
            user_rate_wpm:  175,
            user_pitch:     50,
            user_volume:    100,
            formant_factor: 256,
            tone_adjust:    wavegen::set_tone_adjust(&wavegen::DEFAULT_TONE_POINTS),
            klatt:          false,
            formant_freq_pct:   [100; 7],
            formant_height_pct: [100; 7],
            formant_width_pct:  [100; 7],
            echo_delay_samples: 0,
            echo_amp:           0,
            flutter:            0,
            stress_amps:        [0; 8],
        }
    }
}

/// Per-phoneme pitch flutter multiplier (`+variant` `flutter <n>`) — a slow
/// pseudo-random F0 wobble approximating `wavegen.c`'s `Flutter()`.  Two
/// incommensurate sinusoids give a smooth, non-repeating deviation in [-1, 1],
/// scaled by `flutter/1000` (so `flutter=20` ⇒ up to ±2 %).  `flutter <= 0`
/// returns exactly `1.0`, keeping the default path unchanged.
pub fn flutter_factor(idx: usize, flutter: i32) -> f64 {
    if flutter <= 0 {
        return 1.0;
    }
    let i = idx as f64;
    let lfo = (0.9 * i).sin().mul_add(0.5, 0.5 * (2.3 * i).sin()); // ∈ [-1, 1]
    1.0 + (flutter as f64 / 1000.0) * lfo
}

/// Apply a feedback echo (`+variant` `echo <delay_ms> <amp>`), mirroring the
/// per-sample echo of `wavegen.c`: each sample gets a delayed, attenuated copy of
/// the *already-echoed* output added back (`echo = (delayed * amp) >> 8`),
/// producing exponentially-decaying repeats.  The delayed value read is the
/// stored (i16-saturated) sample, so the feedback is bounded by clipping exactly
/// as in the C. A `delay` of 0 or `amp` of 0 is a no-op.
pub fn apply_echo(pcm: &mut [i16], delay: usize, amp: i32) {
    if delay == 0 || amp == 0 || delay >= pcm.len() {
        return;
    }
    for i in delay..pcm.len() {
        let echo = (pcm[i - delay] as i32 * amp) >> 8;
        pcm[i] = (pcm[i] as i32 + echo).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    }
}

// ---------------------------------------------------------------------------
// Synthesizer
// ---------------------------------------------------------------------------

/// PCM output buffer (16-bit signed mono at 22050 Hz).
pub type PcmBuffer = Vec<i16>;

/// Top-level synthesizer: takes an IPA phoneme string and produces PCM.
///
/// ## Usage
/// ```rust,no_run
/// use espeak_ng::synthesize::{Synthesizer, VoiceParams};
///
/// let synth = Synthesizer::new(VoiceParams::default());
/// let pcm = synth.synthesize("hɛloʊ").expect("synthesis failed");
/// // pcm is a Vec<i16> at 22 050 Hz, mono
/// ```
pub struct Synthesizer {
    /// Voice / acoustic parameters in use.
    pub voice: VoiceParams,
}

impl Synthesizer {
    /// Create a new synthesizer with the given voice parameters.
    pub fn new(voice: VoiceParams) -> Self {
        Synthesizer { voice }
    }

    /// Synthesize an IPA phoneme string to 16-bit PCM samples at 22 050 Hz.
    ///
    /// The string is expected in the format produced by
    /// [`Translator::text_to_ipa`](crate::translate::Translator::text_to_ipa):
    /// IPA characters, with optional stress marks (`ˈ`/`ˌ`), length marks
    /// (`ː`), and ASCII spaces as word separators.
    ///
    /// # Returns
    /// A `Vec<i16>` of mono samples at 22 050 Hz.  The output is normalised
    /// to 90 % of full scale so it should not clip, but can be further scaled
    /// by the caller.
    ///
    /// # Example
    /// ```rust,no_run
    /// use espeak_ng::synthesize::{Synthesizer, VoiceParams};
    ///
    /// let synth = Synthesizer::new(VoiceParams::default());
    /// let pcm = synth.synthesize("ðə").unwrap(); // "the"
    /// assert!(!pcm.is_empty());
    /// ```
    pub fn synthesize(&self, phonemes: &str) -> Result<PcmBuffer> {
        if phonemes.is_empty() {
            return Ok(Vec::new());
        }
        let segments = engine::parse_ipa(phonemes, &self.voice);
        if segments.is_empty() {
            return Err(Error::InvalidData(
                format!("no recognisable phonemes in {:?}", phonemes)
            ));
        }
        let pcm = engine::synthesize_segments(&segments, &self.voice);
        Ok(pcm)
    }

    /// Synthesize phoneme codes directly using espeak-ng's binary acoustic data.
    ///
    /// This is the high-quality path that reads actual formant frame sequences
    /// from the `phondata` file and drives the harmonic synthesizer — the same
    /// acoustic model as the C `espeak-ng` library.
    ///
    /// # Arguments
    /// * `codes`   — slice of `PhonemeCode` items from `Translator::translate_to_codes`.
    /// * `phdata`  — the loaded phoneme data (`PhonemeData::load(data_dir)`).
    ///
    /// # Returns
    /// A `Vec<i16>` at 22 050 Hz.  Returns `Ok(vec![])` if `codes` is empty.
    pub fn synthesize_codes(
        &self,
        codes: &[crate::translate::PhonemeCode],
        phdata: &crate::phoneme::PhonemeData,
    ) -> Result<PcmBuffer> {
        self.synthesize_codes_marked(codes, phdata, &mut Vec::new(), &mut Vec::new())
    }

    /// Build the MBROLA phoneme list for a clause (`MbrolaTranslate`'s input).
    ///
    /// The same annotate + intonation passes as
    /// [`synthesize_codes`](Self::synthesize_codes), but instead of rendering
    /// audio each phoneme is described for the MBROLA handoff: its mnemonic,
    /// type, stress, duration and the syllable's pitch envelope.  Durations come
    /// from rendering the phoneme's own program and measuring it — C reaches
    /// into `DoSample3`/`DoSpect2` for the same number.
    ///
    /// Feed the result to [`MbrolaTable::translate`](crate::mbrola::MbrolaTable::translate).
    pub fn mbrola_phonemes(
        &self,
        codes: &[crate::translate::PhonemeCode],
        phdata: &crate::phoneme::PhonemeData,
    ) -> Vec<crate::mbrola::MbrPhoneme> {
        use crate::mbrola::MbrPhoneme;

        let speed_factor = 100.0 / self.voice.speed_percent.max(1) as f64;
        let sr = self.voice.sample_rate as f64;
        let ms = |samples: usize| -> u32 { (samples as f64 * 1000.0 / sr).round() as u32 };

        let annotated = annotate_codes(codes, phdata);

        // Clause intonation, exactly as the audio path computes it: one syllable
        // per vowel, then the tune assigns each an envelope and pitch span.
        let clause_type = match codes.iter().rev().find_map(|c| c.clause_char) {
            Some('.') => 0usize,
            Some(',') | Some(';') | Some(':') => 1,
            Some('?') => 2,
            Some('!') => 3,
            _ => 0,
        };
        let mut syllables: Vec<intonation::Syllable> = annotated
            .iter()
            .filter_map(|a| match a {
                AnnCode::Phoneme(info) if info.ph_type == PH_VOWEL => {
                    Some(intonation::Syllable::new(info.stress_level))
                }
                _ => None,
            })
            .collect();
        intonation::calc_pitches(
            &mut syllables,
            intonation::tone_for_clause(self.voice.intonation_group, clause_type),
            None,
        );

        // The voice's pitch base/range in the units `SetPitch2` works in (Hz << 12).
        let voice_pitch_base = (self.voice.pitch_hz as i32) << 12;
        let voice_pitch_range = self.voice.pitch_range_units as i32;

        let mut out: Vec<MbrPhoneme> = Vec::new();
        let mut syllable_idx = 0usize;
        // The next real phoneme starts a word (the first one always does).
        let mut newword = true;
        // Highest stress seen in the current word, for the "stressed syllable
        // only" table rows.
        let mut word_start = 0usize;

        let push_pause = |out: &mut Vec<MbrPhoneme>, ms: u32| {
            out.push(MbrPhoneme {
                mnemonic: "_".into(),
                ph_type: PH_PAUSE,
                duration_ms: ms,
                newword: true,
                ..Default::default()
            });
        };

        for ann in &annotated {
            match ann {
                AnnCode::Pause(d) | AnnCode::ClauseBoundary(d) => {
                    push_pause(&mut out, (d * speed_factor).round() as u32);
                    newword = true;
                }
                AnnCode::PrepauseSamples(n) => {
                    push_pause(&mut out, ms(*n));
                }
                // An embedded rate/pitch change doesn't affect the `.pho`
                // phoneme list (MBROLA gets explicit durations and pitch).
                AnnCode::Embedded(_) => {}
                AnnCode::WordBoundary => {
                    // Close the word: every phoneme in it shares its top stress.
                    let top = out[word_start..].iter().map(|p| p.stress_level).max().unwrap_or(0);
                    for p in &mut out[word_start..] {
                        p.word_stress = top;
                    }
                    word_start = out.len();
                    newword = true;
                }
                AnnCode::Phoneme(info) => {
                    let syllable = if info.ph_type == PH_VOWEL {
                        let syl = syllables.get(syllable_idx).copied();
                        syllable_idx += 1;
                        syl
                    } else {
                        None
                    };
                    let syl = syllable.unwrap_or_default();
                    let ph = phdata.get(info.code);
                    let (samples, prepause) = match synthesize_phoneme_info(
                        info, phdata, speed_factor, self.voice.formant_factor,
                        self.voice.stress_lengths.as_ref(),
                    )
                    {
                        PhonemeRender::Frames { frames, lead_silence, trail_silence, .. } => (
                            frames.iter().map(|f| f.length as usize * 64).sum::<usize>()
                                + trail_silence,
                            lead_silence,
                        ),
                        PhonemeRender::Pcm(pcm) => (pcm.len(), 0),
                    };
                    out.push(MbrPhoneme {
                        mnemonic: ph.map(|p| p.mnemonic_display()).unwrap_or_default(),
                        ph_type: info.ph_type,
                        stress_level: info.stress_level,
                        word_stress: info.stress_level,
                        newword,
                        lengthen: info.lengthen,
                        // `std_length` is in mS/2 units.
                        std_length: u16::from(info.std_length) * 2,
                        lengthen_ms: LENGTHEN_MS,
                        length: 256,
                        // A phoneme whose program renders nothing still needs a
                        // duration in the `.pho` — fall back to C's pre-switch
                        // default, `(80 * speed.wav_factor) / 256`.
                        duration_ms: match ms(samples) {
                            0 => (80.0 * speed_factor).round() as u32,
                            d => d,
                        },
                        prepause_ms: ms(prepause) as u16,
                        env: syl.env,
                        pitch1: syl.pitch1,
                        pitch2: syl.pitch2,
                        voice_pitch_base,
                        voice_pitch_range,
                    });
                    newword = false;
                }
            }
        }
        let top = out[word_start..].iter().map(|p| p.stress_level).max().unwrap_or(0);
        for p in &mut out[word_start..] {
            p.word_stress = top;
        }

        // End-of-utterance silence.  C's phoneme list carries it as a trailing
        // pause phoneme; ours adds it at render time, so put it back here or
        // MBROLA would cut the last phoneme off abruptly.
        if !out.is_empty() && !self.voice.no_final_pause {
            push_pause(&mut out, (END_SILENCE_MS * speed_factor).round() as u32);
        }
        out
    }

    /// Like [`synthesize_codes`](Self::synthesize_codes), but also returns the
    /// sample offset at which each spoken **word** begins and at which each
    /// **sentence** (clause) begins — the timing needed for `EventKind::Word` and
    /// `EventKind::Sentence` events.  Element `[i]` is the first sample of the
    /// `i`-th word / sentence (word/sentence 0 starts at 0, so `sentence_marks`
    /// holds only the *subsequent* sentence starts).
    pub fn synthesize_codes_with_marks(
        &self,
        codes: &[crate::translate::PhonemeCode],
        phdata: &crate::phoneme::PhonemeData,
    ) -> Result<(PcmBuffer, Vec<usize>, Vec<usize>)> {
        let mut word_marks = Vec::new();
        let mut sentence_marks = Vec::new();
        let pcm = self.synthesize_codes_marked(codes, phdata, &mut word_marks, &mut sentence_marks)?;
        Ok((pcm, word_marks, sentence_marks))
    }

    fn synthesize_codes_marked(
        &self,
        codes: &[crate::translate::PhonemeCode],
        phdata: &crate::phoneme::PhonemeData,
        word_marks: &mut Vec<usize>,
        sentence_marks: &mut Vec<usize>,
    ) -> Result<PcmBuffer> {
        self.render(codes, phdata, word_marks, sentence_marks, &mut None)
    }

    /// Render `codes` incrementally, handing each **clause**'s audio to `sink`
    /// as soon as it is finished instead of accumulating the whole utterance.
    ///
    /// The rendering order and the samples produced are identical to
    /// [`synthesize_codes`](Self::synthesize_codes) — only the delivery differs
    /// — so a caller that concatenates every chunk gets the same buffer, but the
    /// first clause is available after rendering just that clause.  `sink`
    /// returns `true` to stop early.
    pub fn synthesize_codes_streaming(
        &self,
        codes: &[crate::translate::PhonemeCode],
        phdata: &crate::phoneme::PhonemeData,
        sink: &mut dyn FnMut(&[i16], bool) -> bool,
    ) -> Result<()> {
        // `render` hands the trailing clause back rather than emitting it, so
        // the sink sees exactly one `is_final` chunk — even for empty input,
        // where there is no clause boundary at all.  A sink that asked to stop
        // gets no further calls, final chunk included.
        let mut stopped = false;
        let (tail, _, _) = {
            let mut watch = |chunk: &[i16], is_final: bool| -> bool {
                let stop = sink(chunk, is_final);
                stopped |= stop;
                stop
            };
            self.synthesize_codes_streaming_with_marks(codes, phdata, &mut watch)?
        };
        if !stopped {
            sink(&tail, true);
        }
        Ok(())
    }

    /// Like [`synthesize_codes_streaming`](Self::synthesize_codes_streaming),
    /// but the **final** clause is returned instead of emitted, together with
    /// the word and sentence sample offsets.
    ///
    /// This lets a caller that needs events (which are only known once the whole
    /// utterance is rendered) still stream the earlier clauses: emit them as
    /// they arrive, then deliver the returned tail as the final chunk with the
    /// events attached.  `sink` is called with `is_final == false` throughout.
    pub fn synthesize_codes_streaming_with_marks(
        &self,
        codes: &[crate::translate::PhonemeCode],
        phdata: &crate::phoneme::PhonemeData,
        sink: &mut dyn FnMut(&[i16], bool) -> bool,
    ) -> Result<(PcmBuffer, Vec<usize>, Vec<usize>)> {
        let mut word_marks = Vec::new();
        let mut sentence_marks = Vec::new();
        let mut sink = Some(sink);
        let tail = self.render(codes, phdata, &mut word_marks, &mut sentence_marks, &mut sink)?;
        Ok((tail, word_marks, sentence_marks))
    }

    /// The render loop shared by the buffered and streaming entry points.
    ///
    /// With `sink` present, every clause boundary flushes the audio produced so
    /// far; the final clause is *returned* rather than emitted so the caller can
    /// mark it as the last chunk.  Without a sink the whole utterance is
    /// returned and nothing is emitted.
    fn render(
        &self,
        codes: &[crate::translate::PhonemeCode],
        phdata: &crate::phoneme::PhonemeData,
        word_marks: &mut Vec<usize>,
        sentence_marks: &mut Vec<usize>,
        sink: &mut Option<&mut dyn FnMut(&[i16], bool) -> bool>,
    ) -> Result<PcmBuffer> {
        if codes.is_empty() {
            return Ok(Vec::new());
        }

        // `VoiceParams::speed_percent` is derived from upstream's `speed_lookup`
        // curve (`setlengths::speed_duration_factor`) and already carries the
        // calibration against the C reference, so this is the whole factor.
        let speed_factor = 100.0 / self.voice.speed_percent.max(1) as f64;

        // ── Pass 1: annotate each code with synthesis context ─────────────
        let annotated = annotate_codes(codes, phdata);

        // ── Pass 2: synthesize ─────────────────────────────────────────────
        let mut output_i16: Vec<i16> = Vec::new();
        let mut wavephase: i32 = i32::MAX;
        // Consecutive formant phonemes are accumulated into a "run" and rendered
        // together (`synthesize_frames_seq`) so their frames interpolate
        // continuously across phoneme boundaries; a WAV consonant or any silence
        // flushes the run.  `run_amps`/`run_pitch` are parallel to `run_frames`.
        let mut run_frames: Vec<phondata::SpectFrame> = Vec::new();
        let mut run_amps: Vec<f64> = Vec::new();
        let mut run_pitch: Vec<f64> = Vec::new();
        // Per-frame duration multiplier from inline `\x01<n>S` commands.
        let mut run_speed: Vec<f64> = Vec::new();
        // Absolute frame indices of each vowel's centre within the current run
        // (syllable anchors for SmoothSpect).
        let mut run_centres: Vec<usize> = Vec::new();

        // Clause intonation (`intonation.rs`, a port of `CalcPitches`): build the
        // syllable table — one entry per vowel, carrying its stress — and let the
        // tune assign each syllable a pitch span and contour.  This replaces the
        // old approximation (one falling declination over the whole utterance).
        let total_phonemes = annotated
            .iter()
            .filter(|a| matches!(a, AnnCode::Phoneme(_)))
            .count();
        let clause_type = match codes.iter().rev().find_map(|c| c.clause_char) {
            Some('.') => 0usize,
            Some(',') | Some(';') | Some(':') => 1,
            Some('?') => 2,
            Some('!') => 3,
            _ => 0,
        };
        let mut syllables: Vec<intonation::Syllable> = annotated
            .iter()
            .filter_map(|a| match a {
                AnnCode::Phoneme(info) if info.ph_type == 2 /* phVOWEL */ => {
                    Some(intonation::Syllable::new(info.stress_level))
                }
                _ => None,
            })
            .collect();
        intonation::calc_pitches(
            &mut syllables,
            intonation::tone_for_clause(self.voice.intonation_group, clause_type),
            None,
        );
        let mut syllable_idx = 0usize;
        let mut phoneme_idx = 0usize;
        let base_pitch = self.voice.pitch_hz as f64;
        // Pitch carried across consonants from the syllable around them.
        let mut carried_pitch = base_pitch;

        // Silences follow the rate too, including an inline `\x01<n>S` change.
        let sil_samples = |ms: f64, embed: f64| -> usize {
            ((ms / 1000.0) * 22050.0 * speed_factor * embed) as usize
        };

        // Inter-word gap and end-of-utterance trailing silence.  eSpeak NG
        // appends a fixed ~0.30 s of silence at the end of every clause; the
        // Rust engine previously emitted none, making short utterances end
        // abruptly.  Tuned against the C reference via the wav-analysis harness.
        const WORD_GAP_MS: f64 = 10.0;
        // eSpeak lengthens the final word of a clause (and reduces the rest).
        // We don't model per-syllable final lengthening, but our base lengths
        // already match the *lengthened* value, so shorten non-final words to
        // approximate it.  Tuned against the C reference.
        const NONFINAL_LEN: f64 = 0.85;
        let total_words = annotated
            .iter()
            .filter(|a| matches!(a, AnnCode::WordBoundary))
            .count()
            + 1;
        let mut word_idx = 0usize;

        // Word 0 begins at the start of the audio; each later `WordBoundary`
        // records the first sample of the next word (for `EventKind::Word`).
        word_marks.push(0);
        // Samples already handed to `sink`; marks are absolute, so they are
        // offset by everything flushed so far.
        let mut flushed = 0usize;
        // Set when the sink asks to stop; the loop then unwinds cleanly.
        let mut stopped = false;

        // Inline embedded commands take effect from where they appear, so these
        // run alongside the loop rather than being folded into `VoiceParams`.
        // 1.0 = the utterance's own rate / pitch / amplitude.
        let mut embed_speed = 1.0f64;
        let mut embed_pitch = 1.0f64;
        let mut embed_amp = 1.0f64;
        let mut embed_rate = self.voice.user_rate_wpm as i32;
        let mut embed_pitch_v = self.voice.user_pitch as i32;
        let mut embed_volume = self.voice.user_volume as i32;

        for ann in &annotated {
            match ann {
                AnnCode::Embedded(cmd) => {
                    // The command carries an absolute (or `±` relative) value in
                    // the same units as the `-s`/`-p`/`-a` parameters, so track
                    // the running value and express it against the utterance's
                    // own — and clamp to the parameter ranges, or `\x01 20S`
                    // would stretch the audio 8× where C stops at 2.2×.
                    let next = |cur: i32| -> i32 {
                        let v = cmd.value.max(0);
                        match cmd.relative {
                            1 => cur + v,
                            -1 => cur - v,
                            _ => v,
                        }
                    };
                    match cmd.letter {
                        'S' => {
                            embed_rate = next(embed_rate).clamp(80, 450);
                            // Through the same `speed_lookup` curve the voice's
                            // own rate uses, or a scoped `<prosody rate>` would
                            // stretch further than the whole-utterance form.
                            let f = setlengths::speed_duration_factor(embed_rate as u32);
                            let base =
                                setlengths::speed_duration_factor(self.voice.user_rate_wpm);
                            embed_speed = if base > 0.0 { f / base } else { 1.0 };
                        }
                        'P' => {
                            embed_pitch_v = next(embed_pitch_v).clamp(0, 100);
                            let base = intonation::base_pitch_hz(82.0, self.voice.user_pitch);
                            let now = intonation::base_pitch_hz(82.0, embed_pitch_v as u32);
                            embed_pitch = if base > 0.0 { now / base } else { 1.0 };
                        }
                        'A' => {
                            embed_volume = next(embed_volume).clamp(0, 200);
                            embed_amp = if self.voice.user_volume > 0 {
                                embed_volume as f64 / self.voice.user_volume as f64
                            } else {
                                1.0
                            };
                        }
                        _ => {}
                    }
                }
                AnnCode::Pause(ms) => {
                    flush_run(&mut run_frames, &mut run_amps, &mut run_pitch, &mut run_speed, &mut run_centres,
                              &mut output_i16, &self.voice, &mut wavephase);
                    let n = sil_samples(*ms, embed_speed);
                    output_i16.extend(std::iter::repeat(0i16).take(n));
                }
                AnnCode::ClauseBoundary(ms) => {
                    // Acoustically identical to the old `Pause(200)` clause gap
                    // (so the audio is byte-for-byte unchanged, incl. `word_idx`
                    // which drives non-final lengthening).  Only the *marks* are
                    // new: the following clause starts a new sentence and a new
                    // word.
                    flush_run(&mut run_frames, &mut run_amps, &mut run_pitch, &mut run_speed, &mut run_centres,
                              &mut output_i16, &self.voice, &mut wavephase);
                    let n = sil_samples(*ms, embed_speed);
                    output_i16.extend(std::iter::repeat(0i16).take(n));
                    let start = flushed + output_i16.len();
                    sentence_marks.push(start);
                    word_marks.push(start);
                    // A clause is complete: stream it now rather than holding
                    // the whole utterance in memory.  The echo post-pass rings
                    // across clause boundaries, so a voice with echo is never
                    // split — it emits one final chunk instead.
                    if let Some(sink) = sink.as_mut().filter(|_| self.voice.echo_amp == 0) {
                        if sink(&output_i16, false) {
                            stopped = true;
                            break;
                        }
                        flushed += output_i16.len();
                        output_i16.clear();
                    }
                }
                AnnCode::WordBoundary => {
                    // Small inter-word gap.  espeak lets words flow together
                    // (the perceived gap comes mostly from stop prepauses), so
                    // this is short; a large fixed gap over-lengthens sentences.
                    flush_run(&mut run_frames, &mut run_amps, &mut run_pitch, &mut run_speed, &mut run_centres,
                              &mut output_i16, &self.voice, &mut wavephase);
                    let n = sil_samples(WORD_GAP_MS, embed_speed);
                    output_i16.extend(std::iter::repeat(0i16).take(n));
                    word_idx += 1;
                    // Start sample of the word that follows this boundary.
                    word_marks.push(flushed + output_i16.len());
                }
                AnnCode::PrepauseSamples(n) => {
                    flush_run(&mut run_frames, &mut run_amps, &mut run_pitch, &mut run_speed, &mut run_centres,
                              &mut output_i16, &self.voice, &mut wavephase);
                    output_i16.extend(std::iter::repeat(0i16).take(*n));
                }
                AnnCode::Phoneme(info) => {
                    let _ = total_phonemes;
                    phoneme_idx += 1;
                    // A vowel takes its syllable's pitch contour; a consonant
                    // carries the neighbouring syllable's pitch, as C does by
                    // holding `wdata.pitch` between vowels.
                    let syllable = if info.ph_type == 2 {
                        let syl = syllables.get(syllable_idx).copied();
                        syllable_idx += 1;
                        syl
                    } else {
                        None
                    };
                    if let Some(syl) = syllable {
                        carried_pitch = intonation::pitch_to_hz(
                            syl.pitch_at(0.5),
                            base_pitch,
                            self.voice.pitch_range_units,
                        );
                    }
                    // A vowel *glides* through its syllable's pitch envelope —
                    // C advances `wdata.pitch` every STEPSIZE samples.  Holding
                    // one value per syllable gave a stepped contour where the
                    // oracle's falls smoothly (97→117→105→95→85 Hz over a
                    // sentence, against flat plateaus here).
                    let glide = syllable;
                    // `+variant` pitch flutter (no-op unless a variant set it).
                    let ph_pitch = carried_pitch.max(25.0)
                        * flutter_factor(phoneme_idx, self.voice.flutter)
                        * embed_pitch;
                    // Shorten non-final words (approximate final-word lengthening).
                    let ph_speed = if word_idx + 1 < total_words {
                        speed_factor * NONFINAL_LEN
                    } else {
                        speed_factor
                    } * embed_speed;
                    let is_vowel = info.ph_type == 2;
                    match synthesize_phoneme_info(
                        info, phdata, ph_speed, self.voice.formant_factor,
                        self.voice.stress_lengths.as_ref(),
                    ) {
                        PhonemeRender::Frames { frames, amp_factor, lead_silence, trail_silence } => {
                            // `PauseBefore <ms>`: a real gap, so the formant run
                            // ends here (German puts 15-30 ms before /r/ after a
                            // stop).
                            if lead_silence > 0 {
                                flush_run(&mut run_frames, &mut run_amps, &mut run_pitch,
                                          &mut run_speed, &mut run_centres, &mut output_i16,
                                          &self.voice, &mut wavephase);
                                output_i16.extend(std::iter::repeat(0i16).take(lead_silence));
                            }
                            // `+variant` per-stress amplitude override (no-op by default).
                            let amp_factor = amp_factor
                                * self.voice.stress_amp_ratio(info.stress_level)
                                * embed_amp;
                            // A vowel is a syllable centre for SmoothSpect: anchor
                            // on its most stable (highest-RMS) frame.
                            if is_vowel && !frames.is_empty() {
                                let (rel, _) = frames
                                    .iter()
                                    .enumerate()
                                    .max_by_key(|(_, f)| f.rms)
                                    .unwrap();
                                run_centres.push(run_frames.len() + rel);
                            }
                            run_amps.extend(std::iter::repeat(amp_factor).take(frames.len()));
                            match glide {
                                Some(syl) if frames.len() > 1 => {
                                    let last = (frames.len() - 1) as f64;
                                    let scale = flutter_factor(phoneme_idx, self.voice.flutter)
                                        * embed_pitch;
                                    for i in 0..frames.len() {
                                        let hz = intonation::pitch_to_hz(
                                            syl.pitch_at(i as f64 / last),
                                            base_pitch,
                                            self.voice.pitch_range_units,
                                        );
                                        run_pitch.push(hz.max(25.0) * scale);
                                    }
                                    // Carry the syllable's *end* pitch into the
                                    // following consonants, as C does by holding
                                    // `wdata.pitch` between vowels.
                                    carried_pitch = intonation::pitch_to_hz(
                                        syl.pitch_at(1.0),
                                        base_pitch,
                                        self.voice.pitch_range_units,
                                    );
                                }
                                _ => run_pitch
                                    .extend(std::iter::repeat(ph_pitch).take(frames.len())),
                            }
                            run_speed.extend(std::iter::repeat(embed_speed).take(frames.len()));
                            run_frames.extend(frames);
                            if trail_silence > 0 {
                                flush_run(&mut run_frames, &mut run_amps, &mut run_pitch,
                                          &mut run_speed, &mut run_centres, &mut output_i16,
                                          &self.voice, &mut wavephase);
                                output_i16.extend(std::iter::repeat(0i16).take(trail_silence));
                            }
                        }
                        PhonemeRender::Pcm(pcm) => {
                            flush_run(&mut run_frames, &mut run_amps, &mut run_pitch, &mut run_speed, &mut run_centres,
                                      &mut output_i16, &self.voice, &mut wavephase);
                            output_i16.extend_from_slice(&pcm);
                        }
                    }
                }
            }
        }
        // Flush any trailing formant run.
        flush_run(&mut run_frames, &mut run_amps, &mut run_pitch, &mut run_speed, &mut run_centres,
                  &mut output_i16, &self.voice, &mut wavephase);

        // End-of-utterance trailing silence (only when there was speech, and
        // not when a streaming sink asked to stop early).
        // Suppressed by `-z` (`no_final_pause`).
        if (!output_i16.is_empty() || flushed > 0) && !self.voice.no_final_pause && !stopped {
            output_i16.extend(std::iter::repeat(0i16).take(sil_samples(END_SILENCE_MS, embed_speed)));
        }

        // `+variant` echo post-pass (no-op unless a variant set it) — applied last
        // so the echo tail can ring into the trailing silence.
        apply_echo(&mut output_i16, self.voice.echo_delay_samples, self.voice.echo_amp);

        Ok(output_i16)
    }

    /// Return the sample rate used by this synthesizer.
    ///
    /// Always 22 050 Hz in the current implementation.
    pub fn sample_rate(&self) -> u32 {
        self.voice.sample_rate
    }
}

// ---------------------------------------------------------------------------
// Phoneme synthesis context (annotated code stream)
// ---------------------------------------------------------------------------

/// Per-phoneme synthesis information extracted in pass 1.
struct PhonemeInfo {
    code: u8,
    /// Phoneme type from PhonemeTab (phVOWEL=2, phSTOP=4, phFRICATIVE=6, …)
    ph_type: u8,
    /// Espeak stress level (0–7), already after the 0↔1 swap.
    stress_level: u8,
    /// LENGTHEN (:) modifier — extend duration.
    lengthen: bool,
    // ── CalcLengths inputs for vowels ─────────────────────────────────────
    /// `ph->length_mod` of the NEXT phoneme (0–9).
    next_lm: u8,
    /// `ph->length_mod` of the phoneme after that (0–9).
    next2_lm: u8,
    /// `false` = this is the last syllable in its word.
    more_syllables: bool,
    /// `true` = this is the last vowel before the clause boundary.
    end_of_clause: bool,
    /// `ph->std_length` (mS/2 units).
    std_length: u8,
    // ── Neighbour context for coarticulation (GAPS §36, Stage 1) ──────────
    // Consumed from Stage 2 onward (InterpretPhoneme on neighbours for
    // vowel_transition + VWLSTART/VWLEND selection).
    /// Phoneme type of the previous real phoneme (0 if none / pause).
    #[allow(dead_code)]
    prev_type: u8,
    /// Phoneme code of the previous real phoneme (0 if none / pause).
    #[allow(dead_code)]
    prev_code: u8,
    /// Phoneme type of the next real phoneme.
    #[allow(dead_code)]
    next_type: u8,
    /// Phoneme code of the next real phoneme.
    #[allow(dead_code)]
    next_code: u8,
    /// Phoneme code of the phoneme after the next (for `next2Ph` conditions).
    next2_code: u8,
    /// Word-boundary flags (`sourceix != 0`) at each window position, used by
    /// the context-aware bytecode interpreter for the `*PhW` condition variants.
    this_wordstart: bool,
    prev_wordstart: bool,
    next_wordstart: bool,
    next2_wordstart: bool,
}

/// Annotated synthesis command.
enum AnnCode {
    /// Silence of `ms` milliseconds.
    Pause(f64),
    /// End-of-clause silence of `ms` milliseconds (a sentence boundary — like
    /// [`Pause`](AnnCode::Pause) acoustically, but marks where a `Sentence` event
    /// should fire for the clause that follows).
    ClauseBoundary(f64),
    /// Word-boundary gap.
    WordBoundary,
    /// Pre-phoneme silence already computed in samples.
    PrepauseSamples(usize),
    /// A real phoneme with full context.
    Phoneme(PhonemeInfo),
    /// An inline embedded command (`\x01[±]<value><letter>`): rate, pitch or
    /// amplitude applies from here to the end of the utterance (or the next
    /// command), rather than to the whole of it.
    Embedded(crate::translate::EmbeddedCmd),
}

// ---------------------------------------------------------------------------
// annotate_codes — pass 1
// ---------------------------------------------------------------------------

/// Pre-scan the code stream, resolving stress markers, word boundaries,
/// and CalcLengths context for each phoneme.
fn annotate_codes(
    codes: &[crate::translate::PhonemeCode],
    phdata: &crate::phoneme::PhonemeData,
) -> Vec<AnnCode> {
    let mut result = Vec::new();

    // Helper: get ph_type and length_mod of a real phoneme code
    let ph_info = |c: u8| -> (u8, u8, u8) {
        if let Some(ph) = phdata.get(c) {
            (ph.typ, ph.length_mod, ph.std_length)
        } else {
            (0, 0, 0)
        }
    };

    // Markers are not phonemes.  Drop them all before the look-ahead window is
    // built — a marker left in place is a spurious `code == 0` between two real
    // phonemes, which breaks their coarticulation.  Embedded commands are kept
    // aside, keyed by the index of the code they precede, so they can be
    // re-inserted in order.
    let mut pending_embedded: Vec<(usize, crate::translate::EmbeddedCmd)> = Vec::new();
    let codes: Vec<crate::translate::PhonemeCode> = {
        let mut kept = Vec::with_capacity(codes.len());
        for c in codes {
            match &c.marker {
                Some(crate::translate::CodeMarker::Embedded(cmd)) => {
                    pending_embedded.push((kept.len(), *cmd));
                }
                Some(_) => {}
                None => kept.push(c.clone()),
            }
        }
        kept
    };
    let codes = &codes[..];

    // Build a flat list of (code, is_boundary) for look-ahead
    let flat: Vec<(u8, bool)> = codes.iter().map(|c| (c.code, c.is_boundary)).collect();
    let n = flat.len();

    let mut i = 0;
    // Re-insert the embedded-command markers in order as the loop reaches the
    // codes they preceded.
    let mut next_embedded = 0usize;
    let mut pending_stress: u8 = 0;
    let mut pending_lengthen = false;
    // Previous real phoneme (for coarticulation); reset to pause at silences.
    let mut prev_code: u8 = 0;
    let mut prev_type: u8 = 0;
    // Word-boundary tracking for the *PhW condition variants (InterpretCondition):
    // a phoneme is a "word start" (C `sourceix != 0`) if it is the first real
    // phoneme after a word boundary, pause, or clause start.
    let mut word_start_pending = true; // clause start counts as a word start
    let mut prev_wordstart = false;

    while i < n {
        while pending_embedded.get(next_embedded).is_some_and(|(ix, _)| *ix <= i) {
            result.push(AnnCode::Embedded(pending_embedded[next_embedded].1));
            next_embedded += 1;
        }
        let (code, is_boundary) = flat[i];
        i += 1;

        match code {
            // Clause pause (sentence boundary)
            0 if is_boundary => {
                result.push(AnnCode::ClauseBoundary(200.0));
                pending_stress = 0;
                prev_code = 0; prev_type = 0; // phPAUSE breaks coarticulation
                word_start_pending = true;
            }
            0 => {
                // code=0 with is_boundary=false: ignore
            }
            // Stress markers (1–7 without is_boundary)
            1..=7 if !is_boundary => {
                pending_stress = code;
            }
            // Explicit pause phoneme
            9 => {
                result.push(AnnCode::Pause(80.0));
                prev_code = 0; prev_type = 0;
                word_start_pending = true;
            }
            // Lengthen (:)
            12 => {
                pending_lengthen = true;
            }
            // END_WORD (||)
            15 if is_boundary => {
                result.push(AnnCode::WordBoundary);
                pending_stress = 0;
                word_start_pending = true;
            }
            _ => {
                // Real phoneme (including code 13 = schwa)
                let (ph_type, ph_lm, std_length) = ph_info(code);

                // ── CalcLengths context ─────────────────────────────────

                // Find the NEXT real phoneme (skip stress/control codes), noting
                // whether a word boundary / pause was crossed to reach it
                // (→ `sourceix != 0` for that neighbour in C terms).
                let mut next_code = 0u8;
                let mut next_is_boundary = false;
                let mut next_wordstart = false;
                let mut j = i;
                while j < n {
                    let (nc, nb) = flat[j];
                    j += 1;
                    if nb || nc == 9 { next_wordstart = true; continue; }
                    if nc == 12 { continue; }
                    if nc >= 1 && nc <= 7 { continue; }
                    next_code = nc;
                    next_is_boundary = nb;
                    break;
                }

                // Find NEXT2 real phoneme
                let mut next2_code = 0u8;
                let mut next2_wordstart = false;
                while j < n {
                    let (nc, nb) = flat[j];
                    j += 1;
                    if nb || nc == 9 { next2_wordstart = true; continue; }
                    if nc == 12 { continue; }
                    if nc >= 1 && nc <= 7 { continue; }
                    next2_code = nc;
                    break;
                }
                let this_wordstart = word_start_pending;

                let (next_type, next_lm, _) = ph_info(next_code);
                let (next2_type, next2_lm, _) = ph_info(next2_code);

                // For EOC and more_syllables, scan forward in same word
                let end_of_clause = next_code == 0 || (next_code == 15 && next_is_boundary);
                let more_syllables = {
                    // Count vowels after this one before END_WORD / clause boundary
                    let mut has_more = false;
                    for jj in i..n {
                        let (c2, b2) = flat[jj];
                        if b2 { break; } // END_WORD or pause boundary
                        if c2 == 0 || c2 == 15 { break; }
                        if c2 >= 1 && c2 <= 7 { continue; }
                        if let Some(ph) = phdata.get(c2) {
                            if ph.typ == 2 /* phVOWEL */ { has_more = true; break; }
                        }
                    }
                    has_more
                };

                // Pre-pause for stops/fricatives (mirrors prepause in setlengths.c)
                let prepause_samples = compute_prepause(
                    ph_type, next_type, next2_type, ph_lm, code,
                    &mut result,
                );

                let stress_level = setlengths::stress_code_to_level(pending_stress);
                pending_stress = 0;

                if prepause_samples > 0 {
                    result.push(AnnCode::PrepauseSamples(prepause_samples));
                }

                result.push(AnnCode::Phoneme(PhonemeInfo {
                    code,
                    ph_type,
                    stress_level,
                    lengthen: pending_lengthen,
                    next_lm,
                    next2_lm,
                    more_syllables,
                    end_of_clause,
                    std_length,
                    prev_type,
                    prev_code,
                    next_type,
                    next_code,
                    next2_code,
                    this_wordstart,
                    prev_wordstart,
                    next_wordstart,
                    next2_wordstart,
                }));
                pending_lengthen = false;

                // This phoneme becomes the "previous" for the next one.
                prev_type = ph_type;
                prev_code = code;
                prev_wordstart = this_wordstart;
                word_start_pending = false;

                let _ = next2_type; // next2_type is only used for prepause
            }
        }
    }

    for &(_, cmd) in &pending_embedded[next_embedded..] {
        result.push(AnnCode::Embedded(cmd));
    }
    result
}

// ---------------------------------------------------------------------------
// compute_prepause — prepause silence before consonants
// ---------------------------------------------------------------------------

/// Compute prepause samples for stops/fricatives (mirrors setlengths.c).
/// Returns the number of prepause samples.
fn compute_prepause(
    ph_type: u8,
    _next_type: u8,
    _next2_type: u8,
    _ph_lm: u8,
    _code: u8,
    _result: &mut Vec<AnnCode>,
) -> usize {
    // phSTOP=4, phFRICATIVE=6 get pre-pauses; others don't
    // Simplified: use typical values from setlengths.c
    let prepause_ms: f64 = match ph_type {
        4 /* phSTOP */ => 48.0,
        // phFRICATIVE at word boundary — skip for now, handled by WAV length
        _ => 0.0,
    };
    if prepause_ms > 0.0 {
        (prepause_ms / 1000.0 * 22050.0) as usize
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// synthesize_phoneme_info — pass 2 workhorse
// ---------------------------------------------------------------------------


// ---------------------------------------------------------------------------
// Coarticulation — vowel/consonant formant transitions (GAPS §36, Stages 3–5)
// Ports FormantTransition2 / AdjustFormants / set_frame_rms from synthesize.c.
// ---------------------------------------------------------------------------

/// Unpacked `vowel_transition` (C `FormantTransition2` `data1`/`data2`).
struct Transition {
    rms: i32,
    flags: i32,
    f2: i32,
    f2_min: i32,
    f2_max: i32,
    f3_adj: i32,
    f3_amp: i32,
    f1: i32,
}

fn unpack_transition(data1: u32, data2: u32) -> Transition {
    Transition {
        rms:    ((data1 >> 6) & 0x3f) as i32,
        flags:  (data1 >> 12) as i32,
        f2:     (data2 & 0x3f) as i32 * 50,
        f2_min: (((data2 >> 6) & 0x1f) as i32 - 15) * 50,
        f2_max: (((data2 >> 11) & 0x1f) as i32 - 15) * 50,
        f3_adj: (((data2 >> 16) & 0x1f) as i32 - 15) * 50,
        f3_amp: ((data2 >> 21) & 0x1f) as i32 * 8,
        f1:     ((data2 >> 26) & 0x7) as i32,
    }
}

/// Scale a frame's formant heights so its RMS becomes `new_rms` (heights ∝ √rms).
fn set_frame_rms(fr: &mut phondata::SpectFrame, new_rms: i32) {
    if new_rms <= 0 {
        return;
    }
    let old = fr.rms.max(1) as f64;
    let scale = ((new_rms as f64) / old).sqrt();
    for h in fr.fheight.iter_mut() {
        *h = ((*h as f64 * scale).round() as i32).clamp(0, 255) as u8;
    }
    fr.rms = new_rms.clamp(0, 255) as u8;
}

/// Bend a frame's formants toward a target locus (C `AdjustFormants`).
///
/// `formant_factor` (×256) scales the F2 target for a voice pitched away from
/// the default — a higher-pitched voice has a shorter vocal tract, so its
/// formants sit higher.
fn adjust_formants(
    fr: &mut phondata::SpectFrame,
    target: i32, min: i32, max: i32,
    f1_adj: i32, mut f3_adj: i32, hf_reduce: i32, flags: i32,
    formant_factor: i32,
) {
    let target = target * formant_factor / 256;
    let mut x = (target - fr.ffreq[2] as i32) / 2;
    x = x.clamp(min, max);
    fr.ffreq[2] = (fr.ffreq[2] as i32 + x).clamp(0, i16::MAX as i32) as i16;
    fr.ffreq[3] = (fr.ffreq[3] as i32 + f3_adj).clamp(0, i16::MAX as i32) as i16;
    if flags & 0x20 != 0 {
        f3_adj = -f3_adj;
    }
    fr.ffreq[4] = (fr.ffreq[4] as i32 + f3_adj).clamp(0, i16::MAX as i32) as i16;
    fr.ffreq[5] = (fr.ffreq[5] as i32 + f3_adj).clamp(0, i16::MAX as i32) as i16;
    if f1_adj == 1 {
        let x = (235 - fr.ffreq[1] as i32).clamp(-100, -60);
        fr.ffreq[1] = (fr.ffreq[1] as i32 + x).max(0) as i16;
    } else if f1_adj == 2 {
        let x = (235 - fr.ffreq[1] as i32).clamp(-300, -150);
        fr.ffreq[1] = (fr.ffreq[1] as i32 + x).max(0) as i16;
        fr.ffreq[0] = (fr.ffreq[0] as i32 + x).max(0) as i16;
    } else if f1_adj == 3 {
        let x = (100 - fr.ffreq[1] as i32).max(-400); // C clamps to -400
        fr.ffreq[1] = (fr.ffreq[1] as i32 + x).max(0) as i16;
        fr.ffreq[0] = (fr.ffreq[0] as i32 + x).max(0) as i16;
    }
    // formants_reduce_hf — only when a reduction is specified (guard against
    // f3_amp==0 zeroing the high formants).
    if hf_reduce > 0 {
        for ix in 2..8 {
            fr.fheight[ix] = (fr.fheight[ix] as i32 * hf_reduce / 100).clamp(0, 255) as u8;
        }
    }
}

/// Limit the rate of formant-frequency change between consecutive frames to
/// reduce "chirping" (C `SmoothSpect`, synthesize.c).  Applied within a
/// phoneme's finalised frame sequence — a per-phoneme approximation of C's
/// per-syllable, bidirectional smoothing (which also spans phoneme boundaries
/// via the wcmdq; that cross-phoneme part is not modelled here).
///
/// `formant_rate = {240,170,170,…}` at 22050 Hz; `length` is in STEPSIZE units
/// so the per-frame sample count is `length*64`.
///
/// Limit the frame-to-frame formant-change rate to reduce chirping (C
/// `SmoothSpect`), working outward from `centre` (the most stable frame of the
/// run) in both directions, so each frame's deviation from its neighbour
/// *closer to the centre* is capped.  Anchoring on the stable centre (rather
/// than the onset) preserves intended glides while smoothing chirpy jumps.
///
/// `formant_rate = {240,170,…}` at 22050 Hz; `length` is in STEPSIZE units so a
/// segment's sample count is `length*64`.  Called per-syllable from `flush_run`.
fn smooth_spect(frames: &mut [phondata::SpectFrame], centre: usize) {
    const FORMANT_RATE: [i32; 6] = [240, 170, 170, 170, 170, 170];
    let n = frames.len();
    if n < 2 {
        return;
    }
    let centre = centre.min(n - 1);

    // Cap frame[cur]'s formants toward frame[anchor] (the neighbour nearer the
    // centre), over a transition of `len` samples.  `break_lf` (FRFLAG_BREAK_LF)
    // keeps F1–F3 discontinuous (only smooth F4+).
    let limit = |frames: &mut [phondata::SpectFrame], cur: usize, anchor: usize, len: i32, break_lf: bool| {
        for pk in 0..6 {
            if break_lf && pk < 3 {
                continue;
            }
            let f1 = frames[anchor].ffreq[pk] as i32; // reference (nearer centre)
            let f2 = frames[cur].ffreq[pk] as i32;
            let diff = f2 - f1;
            let mut allowed = if diff > 0 { f1 * 2 + f2 } else { f1 + f2 * 2 };
            allowed = allowed * FORMANT_RATE[pk] / 3000;
            allowed = allowed * len / 256;
            if diff > allowed {
                frames[cur].ffreq[pk] = (f1 + allowed).clamp(0, i16::MAX as i32) as i16;
            } else if diff < -allowed {
                frames[cur].ffreq[pk] = (f1 - allowed).clamp(0, i16::MAX as i32) as i16;
            }
        }
    };

    // Backward from the centre toward the start; stop at a BREAK frame so
    // intended discontinuities (segment boundaries) are preserved.
    for i in (0..centre).rev() {
        let fl = frames[i].frflags;
        if fl & phondata::FRFLAG_BREAK != 0 {
            break;
        }
        let mut len = (frames[i].length as i32) * 64;
        if fl & phondata::FRFLAG_FORMANT_RATE != 0 {
            len = len * 12 / 10;
        }
        limit(frames, i, i + 1, len, fl & phondata::FRFLAG_BREAK_LF != 0);
    }
    // Forward from the centre toward the end; stop at a BREAK frame.
    for i in (centre + 1)..n {
        let fl = frames[i - 1].frflags;
        if fl & phondata::FRFLAG_BREAK != 0 {
            break;
        }
        let mut len = (frames[i - 1].length as i32) * 64;
        if fl & phondata::FRFLAG_FORMANT_RATE != 0 {
            len = len * 6 / 5;
        }
        limit(frames, i, i - 1, len, false);
    }
}

/// Apply a vowel entry (`which=1`) or exit (`which=2`) formant transition using
/// the adjacent consonant's `vowel_transition` data.  Core of `FormantTransition2`
/// (the glottal-stop and vowel-colour edge cases are not yet ported).
fn apply_vowel_transition(
    frames: &mut Vec<phondata::SpectFrame>,
    data1: u32,
    data2: u32,
    which: u8,
    formant_factor: i32,
) {
    const RMS_START: i32 = 28;
    if frames.len() < 2 {
        return;
    }
    let t = unpack_transition(data1, data2);
    if which == 1 {
        // entry to vowel — modify the first frame
        let next_rms = frames[1].rms as i32;
        let fr = &mut frames[0];
        if t.f2 != 0 {
            if t.rms & 0x20 != 0 {
                set_frame_rms(fr, next_rms * (t.rms & 0x1f) / 30);
            }
            adjust_formants(fr, t.f2, t.f2_min, t.f2_max, t.f1, t.f3_adj, t.f3_amp, t.flags,
                            formant_factor);
            if t.rms & 0x20 == 0 {
                set_frame_rms(fr, t.rms * 2);
            }
        } else {
            set_frame_rms(fr, RMS_START);
        }
    } else {
        // exit from vowel — append a bent duplicate of the last frame
        if t.f2 != 0 || t.flags != 0 {
            let mut fr = frames[frames.len() - 1].clone();
            if t.f2 != 0 {
                adjust_formants(&mut fr, t.f2, t.f2_min, t.f2_max, t.f1, t.f3_adj, t.f3_amp,
                                t.flags, formant_factor);
            }
            set_frame_rms(&mut fr, t.rms * 2);
            frames.push(fr);
        }
    }
}

/// Result of preparing one phoneme for synthesis.  Formant-path phonemes yield
/// a finalised frame sequence (rendered later as part of a cross-phoneme *run*
/// so boundaries interpolate continuously); WAV stops/fricatives and silence
/// yield PCM directly.
enum PhonemeRender {
    /// Formant frames plus this phoneme's amplitude factor (applied per frame
    /// when the run is rendered), and any silence its program asked for on
    /// either side (`PauseBefore` / `PauseAfter`, in samples).
    Frames {
        frames: Vec<phondata::SpectFrame>,
        amp_factor: f64,
        lead_silence: usize,
        trail_silence: usize,
    },
    /// Already-rendered PCM (WAV noise burst) or empty.
    Pcm(Vec<i16>),
}

/// Render an accumulated formant *run* (consecutive formant phonemes) as one
/// continuous frame sequence and append the PCM to `output`, then clear the run.
/// Because the whole run is rendered together, adjacent phonemes' frames
/// interpolate smoothly across their boundary (cross-phoneme continuity).
fn flush_run(
    run_frames: &mut Vec<phondata::SpectFrame>,
    run_amps: &mut Vec<f64>,
    run_pitch: &mut Vec<f64>,
    run_speed: &mut Vec<f64>,
    run_centres: &mut Vec<usize>,
    output: &mut Vec<i16>,
    voice: &VoiceParams,
    wavephase: &mut i32,
) {
    if run_frames.is_empty() {
        run_centres.clear();
        return;
    }
    // C `SmoothSpect` (formant-rate limiting) anchored **per syllable**: a run
    // can span several syllables, so split it at the midpoints between adjacent
    // vowel centres and smooth each syllable outward from its own centre.  This
    // caps chirpy within-syllable jumps without flattening the inter-syllable
    // transitions (which a single-centre run-wide pass did, regressing ASR).
    if !run_centres.is_empty() {
        let n = run_frames.len();
        let k = run_centres.len();
        let mid = |a: usize, b: usize| (a + b) / 2;
        for i in 0..k {
            let seg_start = if i == 0 { 0 } else { mid(run_centres[i - 1], run_centres[i]) };
            let seg_end = if i == k - 1 { n } else { mid(run_centres[i], run_centres[i + 1]) };
            let c = run_centres[i];
            if seg_end > seg_start + 1 && c >= seg_start && c < seg_end {
                smooth_spect(&mut run_frames[seg_start..seg_end], c - seg_start);
            }
        }
    }
    // Route through the Klatt cascade only when the *voice* asks for it, as C
    // does (`wcmd_spect = WCMD_KLATT` iff `voice->klattv[0]`).
    //
    // `FRFLAG_KLATT` on a frame is **not** that signal: it only means the frame
    // carries the extra parallel-resonator parameters, and every shipped `en`
    // frame has it.  Routing on the flag sent the standard voices through the
    // Klatt engine; against the C oracle that cost 1.4 dB of spectral-envelope
    // accuracy on average across an eight-sentence sweep (4.19 → 2.79 dB RMS,
    // correlation 0.938 → 0.945), and 9.6 dB on "hello world" alone.
    let raw = if voice.klatt {
        klatt::synthesize_frames_klatt(run_frames, run_amps, run_pitch, voice.sample_rate)
    } else {
        wavegen::synthesize_frames_seq(
            run_frames, run_amps, run_pitch, run_speed, voice, wavephase,
        )
    };
    output.extend_from_slice(&agc_clip(&raw));
    run_frames.clear();
    run_amps.clear();
    run_pitch.clear();
    run_speed.clear();
    run_centres.clear();
}

fn synthesize_phoneme_info(
    info: &PhonemeInfo,
    phdata: &crate::phoneme::PhonemeData,
    speed_factor: f64,
    // `voice->formant_factor` × 256 — the vowel-transition targets are scaled
    // by it, so a voice pitched away from the default shifts its formants too.
    formant_factor: i32,
    // The language's `stressLength` table, when its voice file declares one.
    stress_lengths: Option<&[u32; 8]>,
) -> PhonemeRender {
    use setlengths::{calc_vowel_length_mod, length_mod_to_samples};

    // Constants matching espeak-ng defaults
    const SAMPLERATE: u32 = 22050;

    // ── Look up bytecode ──────────────────────────────────────────────────
    let ph_tab = match phdata.get(info.code) {
        Some(p) => p,
        None => return PhonemeRender::Pcm(Vec::new()),
    };
    // Context-aware interpretation (ports InterpretPhoneme): evaluate the
    // phoneme's conditional program against its neighbours so consonants whose
    // FMT is chosen by `IF prevPh(..)/nextPh(..)` branches (e.g. `l`) get the
    // correct formant sequence instead of the first one the bytecode lists.
    let nb = bytecode::Neighbours {
        prev: info.prev_code,
        this: info.code,
        next: info.next_code,
        next2: info.next2_code,
        stress: info.stress_level,
        this_wordstart: info.this_wordstart,
        prev_wordstart: info.prev_wordstart,
        next_wordstart: info.next_wordstart,
        next2_wordstart: info.next2_wordstart,
        // Synthesis stage: the phoneme list is already built, so the
        // dictionary-vs-rules distinction no longer applies.
        translation_given: false,
        ..Default::default()
    };
    let mut extract = bytecode::interpret_phoneme(
        ph_tab.program,
        &phdata.phonindex,
        &nb,
        |c| phdata.get(c).cloned(),
    );

    // If no fmt_addr found, follow ChangePhoneme chain (for phonemes like @2→@, 02→0, etc.)
    // i_CHANGE_PHONEME(target_code) redirects synthesis to another phoneme's data.
    if extract.fmt_addr.is_none() && extract.wav_addr.is_none() {
        if let Some(target_code) = extract.change_phoneme_code {
            if let Some(target_ph) = phdata.get(target_code) {
                if target_ph.program > 0 {
                    let sub = bytecode::scan_phoneme(target_ph.program, &phdata.phonindex);
                    if extract.fmt_addr.is_none() { extract.fmt_addr = sub.fmt_addr; extract.fmt_param = sub.fmt_param; }
                    if extract.wav_addr.is_none() { extract.wav_addr = sub.wav_addr; extract.wav_param = sub.wav_param; }
                }
            }
        }
    }

    // `PauseBefore`/`PauseAfter <ms>` from the phoneme's own program (C keeps
    // them in `phdata.pd_param[]` and `synthesize.c` adds them to the phoneme's
    // prepause): German inserts 15-30 ms of silence before /r/ after a stop.
    let lead_silence = (extract.pause_before_ms as f64 / 1000.0 * SAMPLERATE as f64
        * speed_factor) as usize;
    let trail_silence = (extract.pause_after_ms as f64 / 1000.0 * SAMPLERATE as f64
        * speed_factor) as usize;

    // phSTOP(4) and phFRICATIVE(6): use WAV noise sample
    // phVSTOP(5) and phVFRICATIVE(7): fall through to formant synthesis
    if info.ph_type == 4 || info.ph_type == 6 {
        // A noise burst breaks formant continuity with the next phoneme (it ends
        // the current render run in the caller).
        if let Some(wav_addr) = extract.wav_addr {
            if let Some(pcm) = sample::parse_wav_sample(
                wav_addr, &phdata.phondata, speed_factor, extract.wav_param as i32,
            ) {
                if lead_silence == 0 && trail_silence == 0 {
                    return PhonemeRender::Pcm(pcm);
                }
                let mut out = vec![0i16; lead_silence];
                out.extend_from_slice(&pcm);
                out.extend(std::iter::repeat(0i16).take(trail_silence));
                return PhonemeRender::Pcm(out);
            }
        }
        // Fallback: short silence if no WAV data
        let n = (50.0 / 1000.0 * SAMPLERATE as f64 * speed_factor) as usize;
        return PhonemeRender::Pcm(vec![0i16; n]);
    }

    // ── Formant synthesis path (VOWEL, LIQUID, NASAL, VSTOP, VFRICATIVE) ─
    let fmt_addr = match extract.fmt_addr {
        Some(a) => a as usize,
        None => return PhonemeRender::Pcm(Vec::new()),
    };
    let mut seq = match phondata::SpectSeq::parse(&phdata.phondata, fmt_addr) {
        Some(s) => s,
        None => return PhonemeRender::Pcm(Vec::new()),
    };

    if seq.frames.is_empty() {
        return PhonemeRender::Pcm(Vec::new());
    }

    // ── Coarticulation (GAPS §36) ─────────────────────────────────────────
    // A vowel's onset glide is a *separate* SPECT_SEQ (`pd_VWLSTART`, the C
    // `which=1` pass) that must be rendered ahead of the body (`pd_FMT`,
    // `which=2`).  We extract `vwlstart_addr` but previously ignored it, so
    // vowels had flat formants (no onset transition — the main reason speech
    // was unintelligible; verified with scripts/dump_c_frames.sh).  The exit is
    // bent toward the next phoneme's VOWELOUT locus.
    // Number of onset (which=1) frames prepended below; they keep their own
    // (ms-based) duration and are excluded from the body length normalisation.
    let mut onset_count = 0usize;
    // Trailing exit (VWLEND / `fmt2_addr`) frames: appended after the body and,
    // like C `LookupSpect`, kept OUT of the body length normalisation (they keep
    // their own durations).  0 for the `use_vowelin` exit (that path leaves the
    // single appended frame as the sequence's final target frame).
    let mut tail_count = 0usize;
    if info.ph_type == 2 {
        // FRFLAG_VOWEL_CENTRE split (C `LookupSpect`): a vowel's own FMT sequence
        // often embeds onset frames *before* a centre-marked frame.  The body
        // (`which=2`) begins at that centre; the pre-centre frames are the
        // vowel's intrinsic onset (`which=1`), used only when no external
        // VowelStart applies.  Without this split the vowel body kept its
        // pre-centre frames and started on the wrong formants.
        let seq_break = seq
            .frames
            .iter()
            .enumerate()
            .filter(|(_, f)| f.frflags & phondata::FRFLAG_VOWEL_CENTRE != 0)
            .map(|(i, _)| i)
            .next_back()
            .unwrap_or(0);
        let pre_centre: Vec<_> = if seq_break > 0 {
            seq.frames[..seq_break].to_vec()
        } else {
            Vec::new()
        };
        if seq_break > 0 {
            seq.frames.drain(..seq_break); // body = frames[seq_break..]
        }

        // Interpret the previous phoneme with THIS vowel as its `next`, so a
        // consonant's `NextVowelStarts` switch — often hidden inside a CALLed
        // procedure (e.g. `l`'s `CALL vowelstart_l`) that the linear scanner
        // cannot follow — resolves to the onset glide it specifies for this
        // vowel's category.  Mirrors the vowel branch of Synthesize().
        let prev_extract = if info.prev_code != 0 {
            phdata.get(info.prev_code).map(|prev| {
                let prev_nb = bytecode::Neighbours {
                    this: info.prev_code,
                    next: info.code,
                    next2: info.next_code,
                    // prev-of-prev unknown here; not needed for NextVowelStarts.
                    ..Default::default()
                };
                bytecode::interpret_phoneme(
                    prev.program, &phdata.phonindex, &prev_nb,
                    |c| phdata.get(c).cloned(),
                )
            })
        } else {
            None
        };

        // (a) Onset SPECT_SEQ (C DoSpect2 which=1): use the vowel's *own*
        // VowelStart unless it is a "for-next-phoneme" glide (pd_FORNEXTPH);
        // otherwise use the previous consonant's VowelStart, but only when that
        // consonant marked it pd_FORNEXTPH (i.e. it is meant for this vowel).
        let onset_addr = if extract.vwlstart_addr.is_some() && !extract.pd_fornextph {
            extract.vwlstart_addr
        } else {
            prev_extract.as_ref().and_then(|pe| {
                if pe.pd_fornextph { pe.vwlstart_addr } else { None }
            })
        };
        let mut used_external_onset = false;
        if let Some(vs_addr) = onset_addr {
            if let Some(vs) = phondata::SpectSeq::parse(&phdata.phondata, vs_addr as usize) {
                if !vs.frames.is_empty() {
                    onset_count = vs.frames.len();
                    used_external_onset = true;
                    let mut combined = vs.frames;
                    combined.append(&mut seq.frames);
                    seq.frames = combined;
                }
            }
        }

        // (b) No external VowelStart (C `use_vowelin` case): use the vowel's own
        // pre-centre frames as the onset and bend the first frame toward the
        // previous consonant's VOWELIN locus (FormantTransition2 which=1).
        if !used_external_onset {
            if !pre_centre.is_empty() {
                onset_count = pre_centre.len();
                let mut combined = pre_centre;
                combined.append(&mut seq.frames);
                seq.frames = combined;
            }
            let prev_vt = prev_extract
                .as_ref()
                .map(|pe| pe.vowel_transition)
                .unwrap_or([0; 4]);
            if (prev_vt[0] != 0 || prev_vt[1] != 0) && seq.frames.len() >= 2 {
                apply_vowel_transition(&mut seq.frames, prev_vt[0], prev_vt[1], 1, formant_factor);
            }
        }

        // (c) Exit (C DoSpect2 which=2): prefer a VWLEND SPECT_SEQ — the vowel's
        // own, or the next consonant's `PrevVowelEndings` glide (selected by this
        // vowel's end_type, resolved by interpreting the next phoneme with this
        // vowel as its `prev`).  Per C `LookupSpect`, the VWLEND's first frame
        // only sets the length of the vowel's last body frame; the remaining
        // frames are appended as a `tail` kept OUT of the body length
        // normalisation.  With no VWLEND, fall back to bending the tail toward
        // the next phoneme's VOWELOUT locus (`use_vowelin`).
        if !seq.frames.is_empty() && info.next_code != 0 {
            let next_extract = phdata.get(info.next_code).map(|next| {
                let next_nb = bytecode::Neighbours {
                    this: info.next_code,
                    prev: info.code,
                    next: info.next2_code,
                    ..Default::default()
                };
                bytecode::interpret_phoneme(
                    next.program, &phdata.phonindex, &next_nb,
                    |c| phdata.get(c).cloned(),
                )
            });
            let vwlend_addr = extract
                .vwlending_addr
                .or_else(|| next_extract.as_ref().and_then(|ne| ne.vwlending_addr));
            if let Some(ve_addr) = vwlend_addr {
                if let Some(ve) = phondata::SpectSeq::parse(&phdata.phondata, ve_addr as usize) {
                    if !ve.frames.is_empty() {
                        if let Some(last) = seq.frames.last_mut() {
                            last.length = ve.frames[0].length; // fmt2[0] sets last-body length
                        }
                        let appended: Vec<_> = ve.frames.into_iter().skip(1).collect();
                        tail_count = appended.len();
                        seq.frames.extend(appended);
                    }
                }
            } else if seq.frames.len() >= 2 {
                let next_vt = next_extract
                    .as_ref()
                    .map(|ne| ne.vowel_transition)
                    .unwrap_or([0; 4]);
                if next_vt[2] != 0 || next_vt[3] != 0 {
                    apply_vowel_transition(&mut seq.frames, next_vt[2], next_vt[3], 2, formant_factor);
                }
            }
        }
    }

    // ── Duration calculation ──────────────────────────────────────────────
    // For vowels (typ=2): use CalcLengths formula.
    // For others: use raw frame lengths from the SPECT_SEQ.
    if info.ph_type == 2 /* phVOWEL */ {
        // A `length <n>` (or `LengthAdd <n>`) instruction inside the phoneme's
        // program overrides its declared `std_length` for this context — C seeds
        // `pd_param[i_SET_LENGTH]` with `std_length` and lets the program change
        // it.  German uses this on most of its vowels.
        let std_length = extract
            .set_length
            .unwrap_or(info.std_length)
            .saturating_add_signed(extract.add_length);
        let length_mod = calc_vowel_length_mod(
            info.stress_level,
            info.next_lm,
            info.next2_lm,
            info.more_syllables,
            info.end_of_clause,
            std_length,
            stress_lengths,
        );

        // Extra lengthening from `:` modifier
        let length_mod = if info.lengthen { length_mod * 4 / 3 } else { length_mod };

        let target_samples = length_mod_to_samples(length_mod, SAMPLERATE, speed_factor);

        // The onset (which=1) frames keep their own duration: convert them
        // ms→STEPSIZE (like consonants) so the glide plays at natural speed
        // rather than being crushed into the vowel body's target length.
        if onset_count > 0 {
            let ms_to_stepsize = (SAMPLERATE as f64 / 1000.0) / 64.0;
            let end = onset_count.min(seq.frames.len());
            for fr in &mut seq.frames[..end] {
                fr.length = ((fr.length as f64 * ms_to_stepsize * speed_factor).round() as usize)
                    .clamp(1, 255) as u8;
            }
        }

        if target_samples > 0 {
            // Scale the BODY (which=2) frames to hit target_samples.  Exclude the
            // onset (which=1) frames, the trailing VWLEND `tail` frames, and the
            // final body frame (a target only): scale frames[body_start..body_end-1].
            let n = seq.frames.len();
            let body_end = n.saturating_sub(tail_count);
            let body_start = onset_count.min(body_end.saturating_sub(1));
            if body_end > body_start + 1 {
                let raw_sum: usize = seq.frames[body_start..body_end-1].iter()
                    .map(|f| f.length as usize).sum::<usize>().max(1);
                let scaled_sum = (raw_sum as f64 * 64.0 * speed_factor) as usize;
                if scaled_sum > 0 {
                    let scale256 = target_samples * 256 / scaled_sum.max(1);
                    for fr in &mut seq.frames[body_start..body_end-1] {
                        let new_len = ((fr.length as usize * scale256 / 256).max(1) as u8).min(255);
                        fr.length = new_len;
                    }
                }
            }
            // The VWLEND tail (and the last body frame whose length fmt2[0] set)
            // keep their own durations: convert ms→STEPSIZE like consonants.
            if tail_count > 0 && body_end >= 1 {
                let ms_to_stepsize = (SAMPLERATE as f64 / 1000.0) / 64.0;
                for fr in &mut seq.frames[body_end - 1..n] {
                    fr.length = ((fr.length as f64 * ms_to_stepsize * speed_factor).round() as usize)
                        .clamp(1, 255) as u8;
                }
            }
        }
    } else {
        // Consonant / sonorant (nasal, liquid, voiced stop/fricative): the
        // SPECT_SEQ `length` field is in milliseconds (C `DoSpect2`:
        // `len = frame_length * samplerate / 1000`).  Our `dur_samples`
        // multiplies by STEPSIZE (64), so convert ms → STEPSIZE units first:
        // factor = (samplerate/1000) / 64 ≈ 0.345.  (Vowels are rescaled to an
        // explicit target above, so this only affects consonants/sonorants.)
        let ms_to_stepsize = (SAMPLERATE as f64 / 1000.0) / 64.0;
        for fr in &mut seq.frames {
            let new_len = ((fr.length as f64 * ms_to_stepsize * speed_factor).round() as usize).max(1);
            fr.length = new_len.min(255) as u8;
        }
    }

    // Lengthen for consonants too (double middle frame)
    if info.lengthen && seq.frames.len() > 1 {
        let mid = seq.frames.len() / 2;
        let extra = seq.frames[mid].clone();
        seq.frames.insert(mid, extra);
    }

    // ── Harmonic synthesis ────────────────────────────────────────────────
    // Amplitude mirrors C wavegen.c:
    //   wdata.amplitude = stress_amp * general_amplitude / 16
    //   For primary stress: wdata.amplitude = 22 * 55 / 16 = 75
    //   Reference: wdata.amplitude_ref = 75 (primary), amplitude_fmt = 100
    //   => wdata.amplitude * amplitude_fmt = 75 * 100 = 7500 (primary)
    // amp_factor is normalized: 1.0 = primary stress = 7500 units.
    // wavegen_segment uses: amp_scale = global_amp * 7500 * amp_factor
    let stress_amp = setlengths::STRESS_AMPS_EN
        .get(info.stress_level as usize)
        .copied()
        .unwrap_or(20) as f64;
    let general_amp = 55.0f64; // GetAmplitude() default
    let wdata_amplitude = stress_amp * general_amp / 16.0;
    // Normalize to primary-stress reference (wdata_amplitude_primary = 22*55/16 = 75.625)
    let amp_primary = 22.0 * 55.0 / 16.0;
    let amp_factor = wdata_amplitude / amp_primary; // 1.0 for primary stress

    // Frame-dump for sample-exact verification against the C reference
    // (GAPS §36). Set ESPEAK_RS_DUMP_FRAMES=<file>; compare with the C dump
    // from scripts/dump_c_frames.sh via scripts/compare_frames.py.
    if let Ok(path) = std::env::var("ESPEAK_RS_DUMP_FRAMES") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let mnem = phdata.get(info.code).map(|p| p.mnemonic_str()).unwrap_or_default();
            let _ = writeln!(f, "PH {} nf={}", mnem, seq.frames.len());
            for fr in &seq.frames {
                let _ = writeln!(
                    f, "  len={} rms={} ffreq={},{},{},{},{},{},{}",
                    fr.length, fr.rms,
                    fr.ffreq[0], fr.ffreq[1], fr.ffreq[2], fr.ffreq[3],
                    fr.ffreq[4], fr.ffreq[5], fr.ffreq[6],
                );
            }
        }
    }

    // Return the finalised frames; the caller batches consecutive formant
    // phonemes into a run and renders them together (cross-phoneme continuity).
    PhonemeRender::Frames {
        frames: seq.frames,
        amp_factor,
        lead_silence,
        trail_silence,
    }
}

// ---------------------------------------------------------------------------
// agc_clip — automatic gain control (mirrors wavegen.c AGC)
// ---------------------------------------------------------------------------

fn agc_clip(samples: &[i32]) -> Vec<i16> {
    if samples.is_empty() {
        return Vec::new();
    }
    let mut agc: i64 = 256;
    let mut out = Vec::with_capacity(samples.len());

    for &z1 in samples {
        let z = (z1 as i64 * agc) >> 8;

        if z >= 32768 {
            let ov = if z1 != 0 { 8_388_608i64 / (z1 as i64).abs() - 1 } else { 0 };
            if ov < agc { agc = ov.max(1); }
            let z2 = (z1 as i64 * agc) >> 8;
            out.push(z2.clamp(-32767, 32767) as i16);
        } else if z <= -32768 {
            let ov = if z1 != 0 { 8_388_608i64 / (z1 as i64).abs() - 1 } else { 0 };
            if ov < agc { agc = ov.max(1); }
            let z2 = (z1 as i64 * agc) >> 8;
            out.push(z2.clamp(-32767, 32767) as i16);
        } else {
            out.push(z.clamp(-32767, 32767) as i16);
        }

        // Gradually restore AGC (mirrors `if (agc < 256) agc++`)
        if agc < 256 { agc += 1; }
    }

    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stress_amp_ratio_unset_is_identity() {
        let v = VoiceParams::default(); // stress_amps all 0
        for level in 0..8u8 {
            assert_eq!(v.stress_amp_ratio(level), 1.0);
        }
    }

    #[test]
    fn stress_amp_ratio_scales_against_baseline() {
        // Override stress level 5 (primary) to double its baseline amplitude.
        let base5 = setlengths::STRESS_AMPS_EN[5] as i32; // 22
        let mut v = VoiceParams::default();
        v.stress_amps[5] = base5 * 2;
        assert!((v.stress_amp_ratio(5) - 2.0).abs() < 1e-9, "primary stress should double");
        // Unset levels stay at unity even when others are set.
        assert_eq!(v.stress_amp_ratio(0), 1.0);
        // Out-of-range level clamps to 7 (no panic).
        let _ = v.stress_amp_ratio(200);
    }

    #[test]
    fn flutter_factor_is_identity_when_off() {
        for idx in 0..20 {
            assert_eq!(flutter_factor(idx, 0), 1.0);
            assert_eq!(flutter_factor(idx, -5), 1.0);
        }
    }

    #[test]
    fn flutter_factor_wobbles_within_bounds() {
        let flutter = 20; // ±2 %
        let vals: Vec<f64> = (0..40).map(|i| flutter_factor(i, flutter)).collect();
        // Deviation is bounded by flutter/1000.
        assert!(vals.iter().all(|&v| (v - 1.0).abs() <= 0.020 + 1e-9), "flutter exceeds bound");
        // It actually varies (not a constant) and both raises and lowers pitch.
        assert!(vals.iter().any(|&v| v > 1.001), "flutter never raises pitch");
        assert!(vals.iter().any(|&v| v < 0.999), "flutter never lowers pitch");
        // Deterministic.
        assert_eq!(flutter_factor(7, flutter), flutter_factor(7, flutter));
    }

    #[test]
    fn apply_echo_adds_decaying_repeats() {
        // A burst followed by silence; echo should place a delayed, attenuated
        // copy of the burst into the previously-silent region, decaying each hop.
        let mut pcm = vec![0i16; 1000];
        for s in pcm.iter_mut().take(100) {
            *s = 10_000;
        }
        apply_echo(&mut pcm, 200, 128); // amp 128 → gain 128/256 = 0.5

        // First echo of the burst at [200,300): 10000 × 0.5 = 5000.
        assert_eq!(pcm[250], 5000, "first echo hop");
        // Second (feedback) echo at [400,500): 5000 × 0.5 = 2500 — decaying.
        assert_eq!(pcm[450], 2500, "second echo hop should decay");
        // Region before the delay is untouched.
        assert_eq!(pcm[50], 10_000);
    }

    #[test]
    fn apply_echo_zero_params_are_noop() {
        let orig = vec![100i16, -200, 300, -400, 500];
        let mut a = orig.clone();
        apply_echo(&mut a, 0, 128); // delay 0
        assert_eq!(a, orig);
        let mut b = orig.clone();
        apply_echo(&mut b, 2, 0); // amp 0
        assert_eq!(b, orig);
    }

    #[test]
    fn apply_echo_saturates_instead_of_diverging() {
        // A large amp feeds back but must stay bounded by i16 saturation (as the
        // C stores the echoed sample into a `short` buffer).
        let mut pcm = vec![0i16; 500];
        pcm[0] = 30_000;
        apply_echo(&mut pcm, 50, 10_000); // huge gain
        assert!(pcm.iter().all(|&s| s >= i16::MIN && s <= i16::MAX));
        assert_eq!(pcm[50], i16::MAX, "over-unity echo saturates, not overflows");
    }

    #[test]
    fn klatt_frame_run_auto_selects_cascade() {
        // A run of FRFLAG_KLATT frames must render through the Klatt cascade even
        // without the global `VoiceParams.klatt` flag, and still produce audio.
        let mut fr = phondata::SpectFrame::default();
        fr.frflags = phondata::FRFLAG_KLATT;
        fr.ffreq = [120, 730, 1090, 2440, 3400, 4000, 4500];
        fr.length = 20;
        fr.rms = 40;
        let mut frames = vec![fr; 4];
        let mut amps = vec![1.0; 4];
        let mut pitch = vec![120.0; 4];
        let mut centres = Vec::new();
        let mut out = Vec::new();
        let mut phase = i32::MAX;
        let voice = VoiceParams::default(); // klatt = false → relies on auto-select
        flush_run(&mut frames, &mut amps, &mut pitch, &mut Vec::new(), &mut centres, &mut out, &voice, &mut phase);
        assert!(!out.is_empty(), "Klatt-flagged run produced no audio");
    }



    #[test]
    fn resonator_tick_accumulates() {
        // With a=1, b=0, c=0 the resonator is just a pass-through
        let mut r = Resonator { a: 1.0, b: 0.0, c: 0.0, x1: 0.0, x2: 0.0 };
        assert!((r.tick(1.0) - 1.0).abs() < 1e-12);
        assert!((r.tick(2.0) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn resonator_tick_with_feedback() {
        // a=0, b=0.5, c=0 → exponential decay of x1
        let mut r = Resonator { a: 0.0, b: 0.5, c: 0.0, x1: 1.0, x2: 0.0 };
        let y0 = r.tick(0.0);
        assert!((y0 - 0.5).abs() < 1e-12);
        let y1 = r.tick(0.0);
        assert!((y1 - 0.25).abs() < 1e-12);
    }

    #[test]
    fn resonator_reset_clears_state() {
        let mut r = Resonator { a: 1.0, b: 0.5, c: 0.0, x1: 99.0, x2: 99.0 };
        r.reset();
        assert_eq!(r.x1, 0.0);
        assert_eq!(r.x2, 0.0);
    }

    #[test]
    fn frame_c_size() {
        // Structural assertion: if we ever add/remove fields the test fails.
        assert_eq!(Frame::C_SIZE, 64,
            "Frame::C_SIZE must match the C struct frame_t");
    }

    #[test]
    fn voice_params_default_sample_rate() {
        let v = VoiceParams::default();
        assert_eq!(v.sample_rate, 22050);
    }

    // ── Synthesizer ─────────────────────────────────────────────────────────

    #[test]
    fn synthesize_empty_string_returns_empty() {
        let s = Synthesizer::new(VoiceParams::default());
        let pcm = s.synthesize("").unwrap();
        assert!(pcm.is_empty());
    }

    #[test]
    fn synthesize_ipa_the() {
        let s = Synthesizer::new(VoiceParams::default());
        let pcm = s.synthesize("ðə").expect("should synthesise 'the'");
        // Must be non-empty; synthesizer clamps to ±32767 so no sample is i16::MIN.
        assert!(!pcm.is_empty());
        assert!(pcm.iter().all(|&x| x >= i16::MIN + 1));
    }

    #[test]
    fn synthesize_hello() {
        let s = Synthesizer::new(VoiceParams::default());
        let pcm = s.synthesize("hɛloʊ").expect("should synthesise 'hello'");
        assert!(!pcm.is_empty());
        // Roughly right duration: ~420 ms at 22050 Hz → at least 5000 samples.
        assert!(pcm.len() > 5_000, "too short: {} samples", pcm.len());
    }

    #[test]
    fn synthesize_produces_nonzero_audio() {
        let s = Synthesizer::new(VoiceParams::default());
        let pcm = s.synthesize("iː").unwrap();
        let peak = pcm.iter().map(|&x| x.unsigned_abs()).max().unwrap_or(0);
        assert!(peak > 1000, "expected non-trivial audio, got peak = {peak}");
    }

    #[test]
    fn synthesize_stress_words() {
        let s = Synthesizer::new(VoiceParams::default());
        // Stress marks must not cause a panic or empty output.
        let pcm = s.synthesize("ˈhɛloʊ ˌwɜːld").unwrap();
        assert!(!pcm.is_empty());
    }

    #[test]
    fn synthesize_unknown_phonemes_error() {
        // A string of only unrecognised chars should return an error.
        let s = Synthesizer::new(VoiceParams::default());
        let result = s.synthesize("☺☻♥");
        assert!(result.is_err(), "expected error for all-unrecognised input");
    }

    #[test]
    fn sample_rate_is_22050() {
        let s = Synthesizer::new(VoiceParams::default());
        assert_eq!(s.sample_rate(), 22050);
    }

    #[test]
    fn synthesize_speed_affects_duration() {
        let mut fast_voice = VoiceParams::default();
        fast_voice.speed_percent = 200; // double speed → half duration

        let s_normal = Synthesizer::new(VoiceParams::default());
        let s_fast   = Synthesizer::new(fast_voice);

        let pcm_normal = s_normal.synthesize("hɛloʊ").unwrap();
        let pcm_fast   = s_fast.synthesize("hɛloʊ").unwrap();

        assert!(pcm_fast.len() < pcm_normal.len(),
            "fast speech must be shorter: fast={}, normal={}",
            pcm_fast.len(), pcm_normal.len());
    }
}
