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
    /// Extended Klatt parameters: AVp, Fric, FricBP, Turb, (spare)
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
    /// Formant frequency scale factor (100 = normal)
    pub formant_scale: u32,
    /// Sample rate in Hz (always 22050 for espeak-ng)
    pub sample_rate: u32,
    /// Amplitude 0–100
    pub amplitude: u32,
    /// Suppress the end-of-utterance trailing silence (`-z`).
    pub no_final_pause: bool,
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
            pitch_range:    100,
            formant_scale:  100,
            sample_rate:    22050,
            amplitude:      80,
            no_final_pause: false,
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
        if codes.is_empty() {
            return Ok(Vec::new());
        }

        // The port's raw per-phoneme durations run ~1.20× the C oracle's at the
        // default rate (a uniform offset measured across a diverse corpus with
        // the built C reference), because the length pipeline omits eSpeak NG's
        // `SetSpeed`/`CalcLengths` `len_speeds` speed scaling.  Until that full
        // per-phoneme port lands, apply the measured calibration so the overall
        // pace matches the oracle (geometric-mean duration ratio 1.20 → ~1.0).
        const RATE_CALIBRATION: f64 = 0.836;
        let speed_factor =
            (100.0 / self.voice.speed_percent.max(1) as f64) * RATE_CALIBRATION;

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
        // Absolute frame indices of each vowel's centre within the current run
        // (syllable anchors for SmoothSpect).
        let mut run_centres: Vec<usize> = Vec::new();

        // Basic intonation: a falling pitch declination across the utterance
        // (a rising tail for questions).  Mirrors the dominant behaviour of
        // `CalcPitches` — pitch starts above the base and falls below it.
        // Verified via the `wav-analysis` harness against the C reference.
        let total_phonemes = annotated
            .iter()
            .filter(|a| matches!(a, AnnCode::Phoneme(_)))
            .count();
        let is_question = codes.iter().any(|c| c.clause_char == Some('?'));
        let mut phoneme_idx = 0usize;
        let base_pitch = self.voice.pitch_hz as f64;

        let sil_samples = |ms: f64| -> usize {
            ((ms / 1000.0) * 22050.0 * speed_factor) as usize
        };

        // Inter-word gap and end-of-utterance trailing silence.  eSpeak NG
        // appends a fixed ~0.30 s of silence at the end of every clause; the
        // Rust engine previously emitted none, making short utterances end
        // abruptly.  Tuned against the C reference via the wav-analysis harness.
        const WORD_GAP_MS: f64 = 10.0;
        const END_SILENCE_MS: f64 = 300.0;
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

        for ann in &annotated {
            match ann {
                AnnCode::Pause(ms) => {
                    flush_run(&mut run_frames, &mut run_amps, &mut run_pitch, &mut run_centres,
                              &mut output_i16, &self.voice, &mut wavephase);
                    let n = sil_samples(*ms);
                    output_i16.extend(std::iter::repeat(0i16).take(n));
                }
                AnnCode::ClauseBoundary(ms) => {
                    // Acoustically identical to the old `Pause(200)` clause gap
                    // (so the audio is byte-for-byte unchanged, incl. `word_idx`
                    // which drives non-final lengthening).  Only the *marks* are
                    // new: the following clause starts a new sentence and a new
                    // word.
                    flush_run(&mut run_frames, &mut run_amps, &mut run_pitch, &mut run_centres,
                              &mut output_i16, &self.voice, &mut wavephase);
                    let n = sil_samples(*ms);
                    output_i16.extend(std::iter::repeat(0i16).take(n));
                    let start = output_i16.len();
                    sentence_marks.push(start);
                    word_marks.push(start);
                }
                AnnCode::WordBoundary => {
                    // Small inter-word gap.  espeak lets words flow together
                    // (the perceived gap comes mostly from stop prepauses), so
                    // this is short; a large fixed gap over-lengthens sentences.
                    flush_run(&mut run_frames, &mut run_amps, &mut run_pitch, &mut run_centres,
                              &mut output_i16, &self.voice, &mut wavephase);
                    let n = sil_samples(WORD_GAP_MS);
                    output_i16.extend(std::iter::repeat(0i16).take(n));
                    word_idx += 1;
                    // Start sample of the word that follows this boundary.
                    word_marks.push(output_i16.len());
                }
                AnnCode::PrepauseSamples(n) => {
                    flush_run(&mut run_frames, &mut run_amps, &mut run_pitch, &mut run_centres,
                              &mut output_i16, &self.voice, &mut wavephase);
                    output_i16.extend(std::iter::repeat(0i16).take(*n));
                }
                AnnCode::Phoneme(info) => {
                    // Position within the utterance, 0.0 (start) → 1.0 (end).
                    let t = if total_phonemes > 1 {
                        phoneme_idx as f64 / (total_phonemes - 1) as f64
                    } else {
                        0.0
                    };
                    phoneme_idx += 1;
                    let factor = pitch_envelope(t, is_question);
                    // `+variant` pitch flutter (no-op unless a variant set it).
                    let ph_pitch = (base_pitch * factor).max(25.0)
                        * flutter_factor(phoneme_idx, self.voice.flutter);
                    // Shorten non-final words (approximate final-word lengthening).
                    let ph_speed = if word_idx + 1 < total_words {
                        speed_factor * NONFINAL_LEN
                    } else {
                        speed_factor
                    };
                    let is_vowel = info.ph_type == 2;
                    match synthesize_phoneme_info(info, phdata, ph_speed) {
                        PhonemeRender::Frames { frames, amp_factor } => {
                            // `+variant` per-stress amplitude override (no-op by default).
                            let amp_factor = amp_factor * self.voice.stress_amp_ratio(info.stress_level);
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
                            run_pitch.extend(std::iter::repeat(ph_pitch).take(frames.len()));
                            run_frames.extend(frames);
                        }
                        PhonemeRender::Pcm(pcm) => {
                            flush_run(&mut run_frames, &mut run_amps, &mut run_pitch, &mut run_centres,
                                      &mut output_i16, &self.voice, &mut wavephase);
                            output_i16.extend_from_slice(&pcm);
                        }
                    }
                }
            }
        }
        // Flush any trailing formant run.
        flush_run(&mut run_frames, &mut run_amps, &mut run_pitch, &mut run_centres,
                  &mut output_i16, &self.voice, &mut wavephase);

        // End-of-utterance trailing silence (only when there was speech).
        // Suppressed by `-z` (`no_final_pause`).
        if !output_i16.is_empty() && !self.voice.no_final_pause {
            output_i16.extend(std::iter::repeat(0i16).take(sil_samples(END_SILENCE_MS)));
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

    // Build a flat list of (code, is_boundary) for look-ahead
    let flat: Vec<(u8, bool)> = codes.iter().map(|c| (c.code, c.is_boundary)).collect();
    let n = flat.len();

    let mut i = 0;
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

/// Synthesize one phoneme using its full annotation context.
/// Pitch multiplier (relative to the base F0) at position `t` ∈ [0, 1] within
/// an utterance.  Statements fall (declination); questions fall then rise.
fn pitch_envelope(t: f64, is_question: bool) -> f64 {
    const TOP: f64 = 1.25;
    const BOTTOM: f64 = 0.60;
    let t = t.clamp(0.0, 1.0);
    if is_question {
        if t < 0.7 {
            TOP - (TOP - BOTTOM) * (t / 0.7)
        } else {
            // Rise through the final 30%, ending above the starting pitch.
            BOTTOM + (TOP + 0.15 - BOTTOM) * ((t - 0.7) / 0.3)
        }
    } else {
        TOP - (TOP - BOTTOM) * t
    }
}

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
fn adjust_formants(
    fr: &mut phondata::SpectFrame,
    target: i32, min: i32, max: i32,
    f1_adj: i32, mut f3_adj: i32, hf_reduce: i32, flags: i32,
) {
    // voice->formant_factor is 256 (1.0) for the default voice.
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
fn apply_vowel_transition(frames: &mut Vec<phondata::SpectFrame>, data1: u32, data2: u32, which: u8) {
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
            adjust_formants(fr, t.f2, t.f2_min, t.f2_max, t.f1, t.f3_adj, t.f3_amp, t.flags);
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
                adjust_formants(&mut fr, t.f2, t.f2_min, t.f2_max, t.f1, t.f3_adj, t.f3_amp, t.flags);
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
    /// when the run is rendered).
    Frames { frames: Vec<phondata::SpectFrame>, amp_factor: f64 },
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
    // Route through the Klatt cascade when explicitly requested (`VoiceParams.klatt`)
    // or when the run's frames are Klatt frames (`FRFLAG_KLATT`) — Klatt frames are
    // meant for Klatt synthesis, not the harmonic path.  Voices with no Klatt frames
    // (e.g. `en`) are unaffected.
    let is_klatt_run = run_frames
        .iter()
        .any(|f| f.frflags & phondata::FRFLAG_KLATT != 0);
    let raw = if voice.klatt || is_klatt_run {
        klatt::synthesize_frames_klatt(run_frames, run_amps, run_pitch, voice.sample_rate)
    } else {
        wavegen::synthesize_frames_seq(run_frames, run_amps, run_pitch, voice, wavephase)
    };
    output.extend_from_slice(&agc_clip(&raw));
    run_frames.clear();
    run_amps.clear();
    run_pitch.clear();
    run_centres.clear();
}

fn synthesize_phoneme_info(
    info: &PhonemeInfo,
    phdata: &crate::phoneme::PhonemeData,
    speed_factor: f64,
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

    // phSTOP(4) and phFRICATIVE(6): use WAV noise sample
    // phVSTOP(5) and phVFRICATIVE(7): fall through to formant synthesis
    if info.ph_type == 4 || info.ph_type == 6 {
        // A noise burst breaks formant continuity with the next phoneme (it ends
        // the current render run in the caller).
        if let Some(wav_addr) = extract.wav_addr {
            if let Some(pcm) = sample::parse_wav_sample(
                wav_addr, &phdata.phondata, speed_factor, extract.wav_param as i32,
            ) {
                return PhonemeRender::Pcm(pcm);
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
                apply_vowel_transition(&mut seq.frames, prev_vt[0], prev_vt[1], 1);
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
                    apply_vowel_transition(&mut seq.frames, next_vt[2], next_vt[3], 2);
                }
            }
        }
    }

    // ── Duration calculation ──────────────────────────────────────────────
    // For vowels (typ=2): use CalcLengths formula.
    // For others: use raw frame lengths from the SPECT_SEQ.
    if info.ph_type == 2 /* phVOWEL */ {
        let length_mod = calc_vowel_length_mod(
            info.stress_level,
            info.next_lm,
            info.next2_lm,
            info.more_syllables,
            info.end_of_clause,
            info.std_length,
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
    PhonemeRender::Frames { frames: seq.frames, amp_factor }
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
        flush_run(&mut frames, &mut amps, &mut pitch, &mut centres, &mut out, &voice, &mut phase);
        assert!(!out.is_empty(), "Klatt-flagged run produced no audio");
    }

    #[test]
    fn pitch_envelope_falls_for_statements() {
        // Declination: pitch decreases monotonically from start to end.
        assert!(pitch_envelope(0.0, false) > pitch_envelope(0.5, false));
        assert!(pitch_envelope(0.5, false) > pitch_envelope(1.0, false));
        assert!(pitch_envelope(0.0, false) > 1.0); // starts above base
        assert!(pitch_envelope(1.0, false) < 1.0); // ends below base
    }

    #[test]
    fn pitch_envelope_rises_at_end_for_questions() {
        // Falls through the body, then rises on the final stretch.
        assert!(pitch_envelope(0.6, true) < pitch_envelope(0.0, true));
        assert!(pitch_envelope(1.0, true) > pitch_envelope(0.7, true));
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
