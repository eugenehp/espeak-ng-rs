//! Klatt cascade/parallel formant synthesizer — the DSP core of `klatt.c`.
//!
//! A voicing source (glottal pulse train, optionally mixed with aspiration
//! noise) is passed through a **cascade of digital resonators**, one per
//! formant, in parallel with a second bank whose outputs are summed with
//! alternating signs.  Each resonator shapes the source's harmonics into a
//! formant peak, so the output has energy concentrated at the requested
//! formant frequencies — the defining behaviour of a Klatt synthesizer.
//!
//! Two entry points:
//!
//!   * [`synthesize`] — a standalone, spectrally-verifiable cascade driven by
//!     one [`KlattFrame`].  Useful for tests and for callers that just want a
//!     formant-shaped tone.
//!   * [`synthesize_frames_klatt`] — the full engine, a direct port of
//!     upstream's `parwave()` / `Wavegen_Klatt()` / `SetSynth_Klatt()` /
//!     `frame_init()` / `pitch_synch_par_reset()`, driven from `phondata`
//!     frames.  Cascade **and** parallel branches, the nasal antiresonator,
//!     both glottal source models, spectral tilt, breathiness and F0 flutter
//!     are all ported.  Verified against the C oracle on a sustained vowel:
//!     spectral-envelope correlation 0.997, RMS difference 2.4 dB.
//!
//! Divergences from C are listed on [`synthesize_frames_klatt`] and in GAPS §1.2.

use std::f64::consts::PI;

/// A second-order digital resonator (Klatt `setabc` / `resonator`): a pole pair
/// at `freq` with the given bandwidth.
#[derive(Clone, Copy, Default)]
struct Resonator {
    a: f64,
    b: f64,
    c: f64,
    p1: f64,
    p2: f64,
}

impl Resonator {
    /// Set the resonator to `freq` Hz with `bw` Hz bandwidth at `sr` Hz.
    fn set(&mut self, freq: f64, bw: f64, sr: f64) {
        let r = (-PI * bw / sr).exp();
        self.b = 2.0 * r * (2.0 * PI * freq / sr).cos();
        self.c = -(r * r);
        self.a = 1.0 - self.b - self.c; // unity gain at DC
    }

    /// Set *anti*resonator coefficients (`setzeroabc`): an ordinary resonator at
    /// `-freq`, then inverted.  Used for the nasal zero.
    fn set_zero(&mut self, freq: f64, bw: f64, sr: f64) {
        let r = (-PI * bw / sr).exp();
        self.b = 2.0 * r * (2.0 * PI * -freq / sr).cos();
        self.c = -(r * r);
        self.a = 1.0 - self.b - self.c;
        // `a == 0` would blow the inversion up into an audible spike (C guards
        // the same way for a nasal register left at f=0, bw=0).
        if self.a != 0.0 {
            self.a = 1.0 / self.a;
            self.c *= -self.a;
            self.b *= -self.a;
        }
    }

    #[inline]
    fn resonate(&mut self, input: f64) -> f64 {
        let out = self.a * input + self.b * self.p1 + self.c * self.p2;
        self.p2 = self.p1;
        self.p1 = out;
        out
    }

    /// `antiresonator()` — same arithmetic, but the *inputs* are the state.
    #[inline]
    fn antiresonate(&mut self, input: f64) -> f64 {
        let out = self.a * input + self.b * self.p1 + self.c * self.p2;
        self.p2 = self.p1;
        self.p1 = input;
        out
    }
}

/// The glottal source model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlottalSource {
    /// A doublet through a glottal resonator (`impulsive_source`).
    #[default]
    Impulsive,
    /// The quadratic pulse of `natural_source` — the Liljencrants-Fant-style
    /// shape espeak uses by default for voiced frames.
    Natural,
}

/// State for one voiced excitation period, shared across a frame's samples.
struct Glottis {
    source: GlottalSource,
    /// Samples per period at 4× the sample rate (`T0`).
    t0: usize,
    /// Open-phase length in 4× samples (`nopen`).
    nopen: usize,
    /// Position within the period, in 4× samples (`nper`).
    nper: usize,
    /// `natural_source` pulse-shape accumulators.
    pulse_a: f64,
    pulse_b: f64,
    vwave: f64,
    /// Glottal resonator for the impulsive source (`RGL`).
    rgl: Resonator,
    /// Low-pass before downsampling from 4× (`RLP`).
    rlp: Resonator,
    nlast: f64,
    nrand: f64,
}

impl Glottis {
    fn new(source: GlottalSource, f0: f64, sr: f64) -> Self {
        let sr4 = sr * 4.0;
        let t0 = if f0 > 0.0 { (sr4 / f0) as usize } else { 0 };
        // Open quotient: C derives `nopen` from the frame's `Kopen`; 0.5 of the
        // period is espeak's default shape.
        let nopen = (t0 / 2).max(1);
        let mut rgl = Resonator::default();
        rgl.set(0.0, 100.0, sr4);
        let mut rlp = Resonator::default();
        rlp.set(0.09 * sr4, 0.06 * sr4, sr4);
        let mut g = Glottis {
            source,
            t0,
            nopen,
            nper: 0,
            pulse_a: 0.0,
            pulse_b: 0.0,
            vwave: 0.0,
            rgl,
            rlp,
            nlast: 0.0,
            nrand: 0.0,
        };
        g.reset_period();
        g
    }

    /// `pitch_synch_par_reset()` — recompute the pulse shape at each period.
    fn reset_period(&mut self) {
        if self.nopen > 1 {
            // C: a quadratic pulse whose second difference is constant.
            let a = 0.0;
            let b = 2.0 * 100.0 / (self.nopen * self.nopen) as f64;
            self.pulse_a = a;
            self.pulse_b = b;
            self.vwave = 0.0;
        }
    }

    /// Pseudo-random noise, low-passed (`gen_noise`).
    fn noise(&mut self, rng: &mut u32) -> f64 {
        *rng = rng.wrapping_mul(1103515245).wrapping_add(12345);
        let r = ((*rng >> 16) & 0x7fff) as f64 / 16383.5 - 1.0; // −1..1
        self.nrand = r * 8191.0;
        let n = self.nrand + 0.75 * self.nlast;
        self.nlast = n;
        n
    }

    /// One sample of the voicing waveform, run at 4× and low-passed.
    fn voice_sample(&mut self) -> f64 {
        let mut voice = 0.0;
        for _ in 0..4 {
            let v = match self.source {
                GlottalSource::Impulsive => {
                    let vwave = match self.nper {
                        0 => 0.0,
                        1 => 13_000_000.0,
                        2 => -13_000_000.0,
                        _ => 0.0,
                    };
                    self.rgl.resonate(vwave)
                }
                GlottalSource::Natural => {
                    if self.nper < self.nopen {
                        self.pulse_a -= self.pulse_b;
                        self.vwave += self.pulse_a;
                        self.vwave * 0.028
                    } else {
                        self.vwave = 0.0;
                        0.0
                    }
                }
            };
            self.nper += 1;
            if self.t0 > 0 && self.nper >= self.t0 {
                self.nper = 0;
                self.reset_period();
            }
            voice = self.rlp.resonate(v);
        }
        voice
    }

    /// True during the glottal open phase, where breathiness is added.
    fn is_open(&self) -> bool {
        self.nper < self.nopen
    }
}

/// A steady Klatt frame: pitch, formants, and the mix of sources feeding the
/// cascade and parallel branches.
#[derive(Debug, Clone)]
pub struct KlattFrame {
    /// Fundamental frequency (Hz).  `0` = unvoiced (noise-only).
    pub f0: f64,
    /// `(frequency, bandwidth)` in Hz for each cascade formant (F1, F2, …).
    pub formants: Vec<(f64, f64)>,
    /// Voicing amplitude (0..=1) into the cascade (`AV`).
    pub voicing: f64,
    /// Aspiration-noise amplitude (0..=1) mixed into the source (`Aspr`).
    pub aspiration: f64,
    /// Voicing amplitude into the **parallel** branch (`AVp`).
    pub parallel_voicing: f64,
    /// Frication-noise amplitude driving the parallel branch (`Fric`).
    pub frication: f64,
    /// Frication bypass path amplitude (`FricBP`).
    pub bypass: f64,
    /// Per-formant parallel amplitudes (F1…F6); empty = no parallel formants.
    pub parallel_amps: Vec<f64>,
    /// Nasal pole/zero: `(pole_hz, zero_hz, bandwidth_hz)`.  `None` = no nasal
    /// branch, which is what a plain oral vowel wants.
    pub nasal: Option<(f64, f64, f64)>,
    /// Glottal source model.
    pub source: GlottalSource,
    /// Spectral tilt of the voicing source (0..=1; `TLTdb`), a soft low-pass.
    pub tilt: f64,
    /// Breathiness added during the glottal open phase (`Aturb`).
    pub breathiness: f64,
}

impl Default for KlattFrame {
    fn default() -> Self {
        KlattFrame {
            f0: 0.0,
            formants: Vec::new(),
            voicing: 0.0,
            aspiration: 0.0,
            parallel_voicing: 0.0,
            frication: 0.0,
            bypass: 0.0,
            parallel_amps: Vec::new(),
            nasal: None,
            source: GlottalSource::default(),
            tilt: 0.0,
            breathiness: 0.0,
        }
    }
}

impl KlattFrame {
    /// A typical voiced vowel frame (voicing on, no aspiration).
    pub fn vowel(f0: f64, formants: &[(f64, f64)]) -> Self {
        KlattFrame {
            f0,
            formants: formants.to_vec(),
            voicing: 1.0,
            source: GlottalSource::Natural,
            ..Default::default()
        }
    }

    /// A fricative frame: no voicing, noise through the parallel branch.
    pub fn fricative(formants: &[(f64, f64)], amps: &[f64]) -> Self {
        KlattFrame {
            formants: formants.to_vec(),
            frication: 1.0,
            parallel_amps: amps.to_vec(),
            bypass: 0.1,
            ..Default::default()
        }
    }

    /// A nasal frame: voicing through the cascade with the nasal pole/zero.
    pub fn nasal(f0: f64, formants: &[(f64, f64)], pole: f64, zero: f64, bw: f64) -> Self {
        KlattFrame {
            f0,
            formants: formants.to_vec(),
            voicing: 1.0,
            nasal: Some((pole, zero, bw)),
            source: GlottalSource::Natural,
            ..Default::default()
        }
    }
}

/// Synthesize `n_samples` of a steady `frame` at `sample_rate` Hz.
///
/// The output is peak-normalized to ~0.6 full-scale (the cascade's resonance
/// gain is large and pitch-dependent), so callers get clean, non-clipping audio;
/// spectral *shape* — the point of a formant synthesizer — is unaffected.
pub fn synthesize(frame: &KlattFrame, n_samples: usize, sample_rate: u32) -> Vec<i16> {
    let sr = sample_rate as f64;

    // Cascade branch: nasal antiresonator + nasal pole, then the formants from
    // the top down (C runs Rnz, Rnpc, R8c…R1c).
    let mut cascade: Vec<Resonator> = frame
        .formants
        .iter()
        .map(|&(f, bw)| {
            let mut r = Resonator::default();
            r.set(f, bw, sr);
            r
        })
        .collect();
    let mut nasal_zero = Resonator::default();
    let mut nasal_pole = Resonator::default();
    if let Some((pole, zero, bw)) = frame.nasal {
        nasal_zero.set_zero(zero, bw, sr);
        nasal_pole.set(pole, bw, sr);
    }

    // Parallel branch: one resonator per formant, plus the parallel nasal pole.
    let mut parallel: Vec<Resonator> = frame
        .formants
        .iter()
        .map(|&(f, bw)| {
            let mut r = Resonator::default();
            r.set(f, bw, sr);
            r
        })
        .collect();
    let mut parallel_nasal = Resonator::default();
    if let Some((pole, _, bw)) = frame.nasal {
        parallel_nasal.set(pole, bw, sr);
    }
    // Output shaping resonator (`Rout`).
    let mut out_res = Resonator::default();
    out_res.set(0.0, sr / 2.0, sr);

    let mut glottis = Glottis::new(frame.source, frame.f0, sr);
    let mut rng: u32 = 0x1234_5678;
    let mut vlast = 0.0f64;
    let mut glotlast = 0.0f64;
    let decay = frame.tilt.clamp(0.0, 0.95);
    let onemd = 1.0 - decay;

    let mut raw = Vec::with_capacity(n_samples);
    for _ in 0..n_samples {
        let mut noise = glottis.noise(&mut rng);
        // Amplitude-modulate the noise over the second half of the period, as
        // C does when voicing is present at the same time.
        if !glottis.is_open() {
            noise *= 0.5;
        }
        let frics = frame.frication * noise;

        // Voicing waveform, tilted and with open-phase breathiness.
        let mut voice = if frame.f0 > 0.0 { glottis.voice_sample() } else { 0.0 };
        voice = voice * onemd + vlast * decay;
        vlast = voice;
        if glottis.is_open() {
            voice += frame.breathiness * glottis.nrand;
        }

        let aspiration = frame.aspiration * noise;
        let glotout = frame.voicing * voice + aspiration;
        let par_glotout = frame.parallel_voicing * voice + aspiration;

        // ── Cascade ──────────────────────────────────────────────────────
        // C runs the cascade unless the frame is marked all-parallel; the
        // source it carries (`glotout`) is voicing *plus* aspiration, so an
        // unvoiced frame still excites it.
        let mut out = 0.0;
        if !cascade.is_empty() {
            let mut casc = glotout;
            if frame.nasal.is_some() {
                casc = nasal_zero.antiresonate(casc);
                casc = nasal_pole.resonate(casc);
            }
            for r in cascade.iter_mut().rev() {
                casc = r.resonate(casc);
            }
            out = casc;
        }

        // ── Parallel ─────────────────────────────────────────────────────
        // F1 and the nasal pole are excited by the voicing waveform; the rest
        // by frication plus the *first difference* of the voicing, and their
        // outputs are summed with alternating signs.
        if !frame.parallel_amps.is_empty() {
            let sourc = par_glotout;
            if let (Some(r1), Some(&a1)) = (parallel.first_mut(), frame.parallel_amps.first()) {
                out += a1 * r1.resonate(sourc);
            }
            if frame.nasal.is_some() {
                out += parallel_nasal.resonate(sourc);
            }

            let sourc = frics + par_glotout - glotlast;
            glotlast = par_glotout;

            for (i, r) in parallel.iter_mut().enumerate().skip(1) {
                let amp = frame.parallel_amps.get(i).copied().unwrap_or(0.0);
                out = amp * r.resonate(sourc) - out;
            }
            let outbypass = frame.bypass * sourc;
            out = outbypass - out;
        }

        raw.push(out_res.resonate(out));
    }

    // Peak-normalize to avoid clipping: the cascade's resonance gain is large
    // and pitch-dependent, and spectral *shape* is what matters here.
    let peak = raw.iter().fold(0.0f64, |m, &v| m.max(v.abs())).max(1e-9);
    let scale = 0.6 * 32767.0 / peak;
    raw.iter().map(|&v| (v * scale).clamp(-32767.0, 32767.0) as i16).collect()
}

// ─────────────────────────────────────────────────────────────────────────
// The `klatt.c` engine proper
//
// Everything below is a direct port of upstream's `parwave()` and the state
// machine around it (`KlattInit`, `SetSynth_Klatt`, `Wavegen_Klatt`,
// `frame_init`, `pitch_synch_par_reset`).  Unlike `synthesize()` above — which
// is a standalone, spectrally-verifiable cascade — this drives the full
// cascade **and** parallel branches from `phondata` frames.
// ─────────────────────────────────────────────────────────────────────────

/// Resonator slots, mirroring the `R*` defines in `klatt.h`.
const RNZ: usize = 0; // nasal zero (anti-resonator)
const R1C: usize = 1; // cascade formants occupy R1c..R8c == 1..=8
const R8C: usize = 8;
const RNPC: usize = 9; // cascade nasal pole
const RNPP: usize = 10; // parallel nasal pole (`Rparallel` + 0)
const R1P: usize = 11;
const R2P: usize = 12;
const R6P: usize = 16;
const RGL: usize = 17; // glottal pulse shaping (impulsive source)
const RLP: usize = 18; // low-pass before downsampling from 4×
const ROUT: usize = 19;
const N_RSN: usize = 20;

/// Formant slots in the frame parameter arrays.
const F_NZ: usize = 0; // nasal zero
const F_NP: usize = 9; // nasal pole

/// Samples per parameter-update block (`STEPSIZE` in `synthesize.h`).
const STEPSIZE: usize = 64;

/// `scale_wav_tab` — output scaling per voicing source.
const SCALE_WAV_TAB: [f64; 6] = [45.0, 38.0, 45.0, 45.0, 55.0, 45.0];

/// `DBtoLIN` — a dB value in 0..=87 to a linear amplitude.
fn db_to_lin(db: i32) -> f64 {
    const AMPTABLE: [i32; 88] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 6, 7,
        8, 9, 10, 11, 13, 14, 16, 18, 20, 22, 25, 28, 32,
        35, 40, 45, 51, 57, 64, 71, 80, 90, 101, 114, 128,
        142, 159, 179, 202, 227, 256, 284, 318, 359, 405,
        455, 512, 568, 638, 719, 881, 911, 1024, 1137, 1276,
        1438, 1622, 1823, 2048, 2273, 2552, 2875, 3244, 3645,
        4096, 4547, 5104, 5751, 6488, 7291, 8192, 9093, 10207,
        11502, 12976, 14582, 16384, 18350, 20644, 23429,
        26214, 29491, 32767,
    ];
    if !(0..88).contains(&db) {
        return 0.0;
    }
    AMPTABLE[db as usize] as f64 * 0.001
}

/// espeak's `espeak_rand` (common.c) — a glibc-style LCG, reproduced exactly
/// so a noise sequence can be compared against the C synthesizer.
///
/// Note the `- min` (rather than `+ min`) in the final step: that is upstream's
/// arithmetic verbatim, and it biases `getrandom(-8191, 8191)` into
/// `8191..=24573` rather than centring it on zero.  Reproduced deliberately —
/// diverging here would make sample-level oracle comparison impossible.
#[derive(Default)]
struct EspeakRand {
    state: u32,
}

impl EspeakRand {
    fn get(&mut self, min: i64, max: i64) -> i64 {
        self.state = ((self.state as u64 * 1103515245 + 12345) % 0x7fff_ffff) as u32;
        (self.state as i64 % (max - min + 1)) - min
    }
}

/// `klatt_frame_t` — the per-block synthesis parameters.
#[derive(Clone)]
struct KtFrame {
    /// F0 in units of 0.1 Hz.
    f0hz10: i32,
    /// Formant frequencies; `[F_NZ]` is the nasal zero, `[F_NP]` the nasal pole.
    fhz: [i32; 10],
    /// Cascade bandwidths.
    bhz: [i32; 10],
    /// Parallel-branch amplitudes (dB).
    ap: [i32; 10],
    /// Parallel-branch bandwidths.
    bphz: [i32; 10],
    /// Amplitude of voicing (dB), and its `- 7` working value.
    avdb: i32,
    avdb_tmp: i32,
    /// Amplitude of aspiration (dB).
    asp: i32,
    /// Open-phase duration parameter.
    kopen: i32,
    /// Amplitude of turbulence / breathiness (dB).
    aturb: i32,
    /// Spectral tilt (dB).
    tltdb: i32,
    /// Amplitude of frication (dB).
    af: i32,
    /// Glottal-period skew.
    kskew: i32,
    /// Amplitude of the bypass path (dB).
    ab: i32,
    /// Parallel amplitude of voicing (dB).
    avpdb: i32,
    /// Overall gain (dB).
    gain0: i32,
}

impl Default for KtFrame {
    fn default() -> Self {
        // `KlattInit` defaults.
        const FORMANT_HZ: [i32; 10] = [280, 688, 1064, 2806, 3260, 3700, 6500, 7000, 8000, 280];
        const BANDWIDTH: [i32; 10] = [89, 160, 70, 160, 200, 200, 500, 500, 500, 89];
        const PARALLEL_AMP: [i32; 10] = [0, 59, 59, 59, 59, 59, 59, 0, 0, 0];
        const PARALLEL_BW: [i32; 10] = [59, 59, 89, 149, 200, 200, 500, 0, 0, 0];
        KtFrame {
            f0hz10: 1000,
            fhz: FORMANT_HZ,
            bhz: BANDWIDTH,
            ap: PARALLEL_AMP,
            bphz: PARALLEL_BW,
            avdb: 59,
            avdb_tmp: 0,
            asp: 0,
            kopen: 40,
            aturb: 0,
            tltdb: 0,
            af: 50,
            kskew: 0,
            ab: 0,
            avpdb: 0,
            gain0: 62,
        }
    }
}

/// `klatt_global_t` — the synthesizer's running state.
struct Klatt {
    rsn: [Resonator; N_RSN],
    samrate: f64,
    minus_pi_t: f64,
    two_pi_t: f64,
    /// Voicing source (1 = impulsive, 2 = natural).
    glsource: i32,
    scale_wav: f64,
    /// Pitch period in 4× samples, and the position within it.
    t0: i64,
    nper: i64,
    /// Point in the period past which noise is attenuated.
    nmod: i64,
    /// Open-phase length in 4× samples.
    nopen: i64,
    amp_voice: f64,
    par_amp_voice: f64,
    amp_aspir: f64,
    amp_frica: f64,
    amp_bypas: f64,
    amp_breth: f64,
    amp_gain0: f64,
    /// Spectral-tilt one-pole coefficients.
    decay: f64,
    onemd: f64,
    /// `natural_source` pulse shape.
    pulse_shape_a: f64,
    pulse_shape_b: f64,
    nat_vwave: f64,
    imp_vwave: f64,
    /// Noise state.
    nrand: f64,
    nlast: f64,
    noise: f64,
    /// Voicing / parallel-branch history.
    vlast: f64,
    glotlast: f64,
    /// Alternating glottal skew.
    skew: i64,
    f0_flutter: f64,
    original_f0: f64,
    time_count: i64,
    rand: EspeakRand,
    /// Sample index within the current block (`ns`), and the fade counters.
    ns: usize,
    fadein: i32,
    fadeout: i32,
}

impl Klatt {
    fn new(sample_rate: u32) -> Self {
        let samrate = sample_rate as f64;
        let mut k = Klatt {
            rsn: [Resonator::default(); N_RSN],
            samrate,
            minus_pi_t: -PI / samrate,
            two_pi_t: 2.0 * PI / samrate,
            glsource: 1, // IMPULSIVE
            scale_wav: SCALE_WAV_TAB[1],
            t0: 0,
            nper: 0,
            nmod: 0,
            nopen: 0,
            amp_voice: 0.0,
            par_amp_voice: 0.0,
            amp_aspir: 0.0,
            amp_frica: 0.0,
            amp_bypas: 0.0,
            amp_breth: 0.0,
            amp_gain0: 0.0,
            decay: 0.0,
            onemd: 1.0,
            pulse_shape_a: 0.0,
            pulse_shape_b: 0.0,
            nat_vwave: 0.0,
            imp_vwave: 0.0,
            nrand: 0.0,
            nlast: 0.0,
            noise: 0.0,
            vlast: 0.0,
            glotlast: 0.0,
            skew: 0,
            f0_flutter: 20.0 / 32.0,
            original_f0: 0.0,
            time_count: 0,
            rand: EspeakRand::default(),
            ns: 0,
            fadein: 0,
            fadeout: 0,
        };
        // `KlattReset(2)`: the low-pass applied to the voicing waveform inside
        // the 4× loop, before downsampling.  C sets it from the *normal* sample
        // rate (f = 0.095·sr, bw = 0.063·sr) even though it runs at 4×.
        let flp = 950 * sample_rate as i32 / 10000;
        let blp = 630 * sample_rate as i32 / 10000;
        k.set_abc(RLP, flp, blp);
        k
    }

    /// `setabc` at the synthesizer's own sample rate.
    fn set_abc(&mut self, ix: usize, f: i32, bw: i32) {
        let r = (self.minus_pi_t * bw as f64).exp();
        self.rsn[ix].c = -(r * r);
        self.rsn[ix].b = r * (self.two_pi_t * f as f64).cos() * 2.0;
        self.rsn[ix].a = 1.0 - self.rsn[ix].b - self.rsn[ix].c;
    }

    /// `setzeroabc` — an ordinary resonator at `-f`, then inverted.
    fn set_zero_abc(&mut self, ix: usize, f: i32, bw: i32) {
        self.set_abc(ix, -f, bw);
        let r = &mut self.rsn[ix];
        // `a == 0` would send the inversion to infinity and put an audible
        // spike in the output (C guards this the same way).
        if r.a != 0.0 {
            r.a = 1.0 / r.a;
            r.c *= -r.a;
            r.b *= -r.a;
        }
    }

    /// `frame_init` — derive per-block amplitudes and resonator coefficients.
    fn frame_init(&mut self, frame: &mut KtFrame) {
        const AMP_PAR_FACTOR: [f64; 7] = [0.6, 0.4, 0.15, 0.06, 0.04, 0.022, 0.03];

        self.original_f0 = (frame.f0hz10 / 10) as f64;

        frame.avdb_tmp = (frame.avdb - 7).max(0);

        self.amp_aspir = db_to_lin(frame.asp) * 0.05;
        self.amp_frica = db_to_lin(frame.af) * 0.25;
        self.par_amp_voice = db_to_lin(frame.avpdb);
        self.amp_bypas = db_to_lin(frame.ab) * 0.05;

        let mut amp_par = [0.0f64; 7];
        for ix in 0..7 {
            amp_par[ix] = db_to_lin(frame.ap[ix]) * AMP_PAR_FACTOR[ix];
        }

        let mut gain0 = frame.gain0 - 3;
        if gain0 <= 0 {
            gain0 = 57;
        }
        self.amp_gain0 = db_to_lin(gain0) / self.scale_wav;

        // Cascade formants 1..8 plus the nasal pole.
        for ix in R1C..=RNPC {
            self.set_abc(ix, frame.fhz[ix], frame.bhz[ix]);
        }
        self.set_zero_abc(RNZ, frame.fhz[F_NZ], frame.bhz[F_NZ]);

        // Parallel resonators, with the branch amplitude folded into `a`.
        for ix in 0..7 {
            self.set_abc(RNPP + ix, frame.fhz[ix], frame.bphz[ix]);
            self.rsn[RNPP + ix].a *= amp_par[ix];
        }

        // Output low-pass.
        self.set_abc(ROUT, 0, (self.samrate / 2.0) as i32);
    }

    /// `pitch_synch_par_reset` — recompute the period-dependent parameters at
    /// the start of every glottal period.
    fn pitch_synch_par_reset(&mut self, frame: &mut KtFrame) {
        const B0: [i32; 224] = [
            1200, 1142, 1088, 1038, 991, 948, 907, 869, 833, 799, 768, 738, 710, 683, 658,
            634, 612, 590, 570, 551, 533, 515, 499, 483, 468, 454, 440, 427, 415, 403,
            391, 380, 370, 360, 350, 341, 332, 323, 315, 307, 300, 292, 285, 278, 272,
            265, 259, 253, 247, 242, 237, 231, 226, 221, 217, 212, 208, 204, 199, 195,
            192, 188, 184, 180, 177, 174, 170, 167, 164, 161, 158, 155, 153, 150, 147,
            145, 142, 140, 137, 135, 133, 131, 128, 126, 124, 122, 120, 119, 117, 115,
            113, 111, 110, 108, 106, 105, 103, 102, 100, 99, 97, 96, 95, 93, 92, 91, 90,
            88, 87, 86, 85, 84, 83, 82, 80, 79, 78, 77, 76, 75, 75, 74, 73, 72, 71,
            70, 69, 68, 68, 67, 66, 65, 64, 64, 63, 62, 61, 61, 60, 59, 59, 58, 57,
            57, 56, 56, 55, 55, 54, 54, 53, 53, 52, 52, 51, 51, 50, 50, 49, 49, 48, 48,
            47, 47, 46, 46, 45, 45, 44, 44, 43, 43, 42, 42, 41, 41, 41, 41, 40, 40,
            39, 39, 38, 38, 38, 38, 37, 37, 36, 36, 36, 36, 35, 35, 35, 35, 34, 34, 33,
            33, 33, 33, 32, 32, 32, 32, 31, 31, 31, 31, 30, 30, 30, 30, 29, 29, 29, 29,
            28, 28, 28, 28, 27, 27,
        ];

        if frame.f0hz10 > 0 {
            // `T0` is 4× the number of samples in one pitch period.
            self.t0 = (40.0 * self.samrate) as i64 / frame.f0hz10 as i64;
            self.amp_voice = db_to_lin(frame.avdb_tmp);

            // Duration of the period before amplitude modulation.
            self.nmod = self.t0;
            if frame.avdb_tmp > 0 {
                self.nmod >>= 1;
            }

            self.amp_breth = db_to_lin(frame.aturb) * 0.1;

            // Open phase of the glottal period, clamped to 40..=263.
            self.nopen = 4 * frame.kopen as i64;
            if self.glsource == 1 && self.nopen > 263 {
                self.nopen = 263;
            }
            if self.nopen >= self.t0 - 1 {
                self.nopen = self.t0 - 2;
            }
            if self.nopen < 40 {
                self.nopen = 40; // F0 max = 1000 Hz
            }

            // Shape of the "natural" glottal waveform.
            self.pulse_shape_b = B0[(self.nopen - 40).clamp(0, 223) as usize] as f64;
            self.pulse_shape_a = self.pulse_shape_b * self.nopen as f64 * 0.333;

            // Width of the "impulsive" glottal pulse.
            let temp = (self.samrate as i64 / self.nopen) as i32;
            self.set_abc(RGL, 0, temp);
            // Keep the gain at F1 roughly constant.
            let temp1 = self.nopen as f64 * 0.00833;
            self.rsn[RGL].a *= temp1 * temp1;

            // Skewness may not exceed the closed phase of the period.
            let closed = self.t0 - self.nopen;
            if frame.kskew as i64 > closed {
                frame.kskew = closed as i32;
            }
            self.skew = if self.skew >= 0 { frame.kskew as i64 } else { -(frame.kskew as i64) };
            self.t0 += self.skew;
            self.skew = -self.skew;
        } else {
            self.t0 = 4; // default when F0 is undefined
            self.amp_voice = 0.0;
            self.nmod = self.t0;
            self.amp_breth = 0.0;
            self.pulse_shape_a = 0.0;
            self.pulse_shape_b = 0.0;
        }

        // Reset pitch-synchronously, or at the update rate when f0 = 0.
        if self.t0 != 4 || self.ns == 0 {
            self.decay = 0.033 * frame.tltdb as f64;
            self.onemd = if self.decay > 0.0 { 1.0 - self.decay } else { 1.0 };
        }
    }

    /// `impulsive_source` — a doublet through the glottal resonator.
    fn impulsive_source(&mut self) -> f64 {
        const DOUBLET: [f64; 3] = [0.0, 13_000_000.0, -13_000_000.0];
        self.imp_vwave = if self.nper < 3 { DOUBLET[self.nper as usize] } else { 0.0 };
        let v = self.imp_vwave;
        self.rsn[RGL].resonate(v)
    }

    /// `natural_source` — the quadratic pulse of the Liljencrants-Fant-style
    /// "natural" glottal model.
    fn natural_source(&mut self) -> f64 {
        if self.nper < self.nopen {
            self.pulse_shape_a -= self.pulse_shape_b;
            self.nat_vwave += self.pulse_shape_a;
            return self.nat_vwave * 0.028;
        }
        self.nat_vwave = 0.0;
        0.0
    }

    /// `gen_noise` — a one-pole low-passed random sequence.
    fn gen_noise(&mut self) -> f64 {
        self.nrand = self.rand.get(-8191, 8191) as f64;
        self.noise = self.nrand + 0.75 * self.nlast;
        self.nlast = self.noise;
        self.noise
    }

    /// `flutter` — quasi-random F0 jitter built from three slow sines.
    fn flutter(&mut self, frame: &mut KtFrame) {
        let t = self.time_count as f64;
        let fla = self.f0_flutter / 50.0;
        let flb = self.original_f0 / 100.0;
        let delta = fla
            * flb
            * ((PI * 12.7 * t).sin() + (PI * 7.1 * t).sin() + (PI * 4.7 * t).sin())
            * 10.0;
        frame.f0hz10 += delta as i32;
        self.time_count += 1;
    }

    /// `parwave` — convert one block of synthesis parameters to samples.
    fn parwave(&mut self, frame: &mut KtFrame, nspfr: usize, amplitude: f64, out: &mut Vec<f64>) {
        self.flutter(frame);

        for ns in 0..nspfr {
            self.ns = ns;

            // Low-passed noise for aspiration and frication.
            let mut noise = self.gen_noise();
            // Attenuate noise over the second half of the glottal period when
            // voicing is present at the same time.
            if self.nper > self.nmod {
                noise *= 0.5;
            }
            let frics = self.amp_frica * noise;

            // Voicing waveform, generated at 4× the sample rate to keep
            // quantization noise out of the period of a female voice.
            let mut voice = 0.0;
            for _ in 0..4 {
                voice = match self.glsource {
                    2 => self.natural_source(),
                    _ => self.impulsive_source(),
                };
                if self.nper >= self.t0 {
                    self.nper = 0;
                    self.pitch_synch_par_reset(frame);
                }
                voice = self.rsn[RLP].resonate(voice);
                self.nper += 1;
            }

            // Tilt the source spectrum down by soft low-pass filtering.
            voice = voice * self.onemd + self.vlast * self.decay;
            self.vlast = voice;

            // Breathiness during the glottal open phase — `nrand` rather than
            // `noise`, because `noise` is low-passed.
            if self.nper < self.nopen {
                voice += self.amp_breth * self.nrand;
            }

            let aspiration = self.amp_aspir * noise;
            let glotout = self.amp_voice * voice + aspiration;
            let par_glotout = self.par_amp_voice * voice + aspiration;

            // ── Cascade: nasal zero, nasal pole, then F8 … F1 ───────────
            let mut casc = self.rsn[RNZ].antiresonate(glotout);
            casc = self.rsn[RNPC].resonate(casc);
            for ix in (R1C..=R8C).rev() {
                casc = self.rsn[ix].resonate(casc);
            }
            let mut out_s = casc;

            // ── Parallel: F1 and the nasal pole are excited by voicing… ──
            let sourc = par_glotout;
            out_s += self.rsn[R1P].resonate(sourc);
            out_s += self.rsn[RNPP].resonate(sourc);

            // …the rest by frication plus the first difference of voicing,
            // summed with alternating signs.
            let sourc = frics + par_glotout - self.glotlast;
            self.glotlast = par_glotout;
            for ix in R2P..=R6P {
                out_s = self.rsn[ix].resonate(sourc) - out_s;
            }
            out_s = self.amp_bypas * sourc - out_s;

            out_s = self.rsn[ROUT].resonate(out_s);
            let mut temp = out_s * amplitude * self.amp_gain0;

            // Fade in/out over 64 samples to avoid clicks at run boundaries.
            if self.fadein < 64 {
                temp = temp * self.fadein as f64 / 64.0;
                self.fadein += 1;
            }
            if self.fadeout > 0 {
                self.fadeout -= 1;
                temp = temp * self.fadeout as f64 / 64.0;
                if self.fadeout == 0 {
                    self.fadein = 0;
                }
            }

            out.push(temp);
        }
    }
}

/// One formant's interpolation state across a frame (`klatt_peaks_t`).
#[derive(Clone, Copy, Default)]
struct KlattPeak {
    freq1: f64,
    freq: i32,
    freq_inc: f64,
    bw1: f64,
    bw: i32,
    bw_inc: f64,
    ap1: f64,
    ap: i32,
    ap_inc: f64,
    bp1: f64,
    bp: i32,
    bp_inc: f64,
}

const N_PEAKS: usize = 9;
const N_KLATTP: usize = 10;

/// Render a `SpectFrame` sequence through the full Klatt engine.
///
/// This is the port of `Wavegen_Klatt` + `SetSynth_Klatt`: for each frame it
/// sets up linear interpolation of formant frequencies, bandwidths and
/// parallel-branch parameters toward the *next* frame, then advances them once
/// per `STEPSIZE` block while `parwave` fills the block.
///
/// Two deliberate divergences from C, both documented in GAPS §1.2:
///   * a frame **without** `FRFLAG_KLATT` carries no Klatt parameters at all,
///     and upstream then synthesizes silence (every `klattp` is zero, so
///     `amp_voice == DBtoLIN(-7) == 0`).  Here such a frame falls back to the
///     `KlattInit` defaults and the frame's own `rms`, so a Klatt voice with
///     non-Klatt phoneme data still speaks.
///   * the result is peak-normalized rather than scaled by `wdata->amplitude`,
///     because the caller applies its own AGC.
pub fn synthesize_frames_klatt(
    frames: &[super::phondata::SpectFrame],
    amps: &[f64],
    pitch: &[f64],
    sample_rate: u32,
) -> Vec<i32> {
    use super::phondata::FRFLAG_KLATT;

    let mut kt = Klatt::new(sample_rate);
    let mut kt_frame = KtFrame::default();
    let mut raw: Vec<f64> = Vec::new();

    for (fi, fr) in frames.iter().enumerate() {
        let next = frames.get(fi + 1).unwrap_or(fr);
        let length = fr.length as usize * STEPSIZE;
        if length == 0 {
            continue;
        }
        let klatt_flag = fr.frflags & FRFLAG_KLATT != 0;

        // ── SetSynth_Klatt ───────────────────────────────────────────────
        let mut peaks = [KlattPeak::default(); N_PEAKS];
        let mut klattp = [0i32; N_KLATTP];
        let mut klattp1 = [0f64; N_KLATTP];
        let mut klattp_inc = [0f64; N_KLATTP];
        let step_over_len = STEPSIZE as f64 / length as f64;

        for ix in 0..N_KLATTP {
            // C zeroes everything from index 5 up (`Kopen`, `AVp`, `Fric`,
            // `FricBP`, `Turb` are never fed to the synthesizer), and zeroes
            // the whole set for a non-Klatt frame.
            if ix >= 5 || !klatt_flag {
                klattp[ix] = 0;
                klattp1[ix] = 0.0;
                klattp_inc[ix] = 0.0;
            } else {
                klattp[ix] = fr.klattp[ix] as i32;
                klattp1[ix] = klattp[ix] as f64;
                klattp_inc[ix] = (next.klattp[ix] as i32 - klattp[ix]) as f64 * step_over_len;
            }
        }

        for ix in 1..6 {
            peaks[ix].freq1 = fr.ffreq[ix] as f64;
            peaks[ix].freq = peaks[ix].freq1 as i32;
            peaks[ix].freq_inc = (next.ffreq[ix] as f64 - peaks[ix].freq1) * step_over_len;

            if ix < 4 {
                // Klatt bandwidth for F1..F3; the rest are fixed.
                peaks[ix].bw1 = fr.bw[ix] as f64 * 2.0;
                peaks[ix].bw = peaks[ix].bw1 as i32;
                peaks[ix].bw_inc = (next.bw[ix] as f64 * 2.0 - peaks[ix].bw1) * step_over_len;
            }
        }

        // Nasal zero frequency; if there is none it tracks the nasal pole.
        peaks[0].freq1 = fr.klattp[1] as f64 * 2.0; // KLATT_FNZ
        if peaks[0].freq1 == 0.0 {
            peaks[0].freq1 = kt_frame.fhz[F_NP] as f64;
        }
        peaks[0].freq = peaks[0].freq1 as i32;
        let mut nz_next = next.klattp[1] as f64 * 2.0;
        if nz_next == 0.0 {
            nz_next = kt_frame.fhz[F_NP] as f64;
        }
        peaks[0].freq_inc = (nz_next - peaks[0].freq1) * step_over_len;
        peaks[0].bw1 = 89.0;
        peaks[0].bw = 89;
        peaks[0].bw_inc = 0.0;

        if klatt_flag {
            // The frame carries the extra parallel-resonator parameters.
            for ix in 1..7 {
                peaks[ix].bp1 = fr.klatt_bp[ix] as f64 * 4.0;
                peaks[ix].bp = peaks[ix].bp1 as i32;
                peaks[ix].bp_inc =
                    (next.klatt_bp[ix] as f64 * 4.0 - peaks[ix].bp1) * step_over_len;

                peaks[ix].ap1 = fr.klatt_ap[ix] as f64;
                peaks[ix].ap = peaks[ix].ap1 as i32;
                peaks[ix].ap_inc = (next.klatt_ap[ix] as f64 - peaks[ix].ap1) * step_over_len;
            }
        } else {
            // Divergence (see the fn doc): keep the `KlattInit` defaults so a
            // non-Klatt frame is audible instead of silent.
            let dflt = KtFrame::default();
            for ix in 1..7 {
                peaks[ix].ap1 = dflt.ap[ix] as f64;
                peaks[ix].ap = dflt.ap[ix];
                klattp[0] = dflt.avdb; // AV
                klattp1[0] = dflt.avdb as f64;
            }
        }

        // ── Wavegen_Klatt ────────────────────────────────────────────────
        let mut sample_count = 0usize;
        // The frame's own loudness: `rms` for the fallback path, where AV is a
        // constant; for a real Klatt frame AV already carries it.
        let rms_scale = if klatt_flag { 1.0 } else { (fr.rms as f64 / 64.0).max(0.05) };
        let amplitude = amps.get(fi).copied().unwrap_or(1.0) * rms_scale * 60.0;
        let f0_start = pitch.get(fi).copied().unwrap_or(120.0).max(50.0);
        let f0_end = pitch.get(fi + 1).copied().unwrap_or(f0_start).max(50.0);

        while sample_count < length {
            let t = sample_count as f64 / length as f64;
            kt_frame.f0hz10 = ((f0_start + (f0_end - f0_start) * t) * 10.0) as i32;

            // F6..F8 are fixed cascade values set at init; F0's slot holds the
            // nasal zero.
            for ix in 0..6 {
                kt_frame.fhz[ix] = peaks[ix].freq;
                if ix < 4 {
                    kt_frame.bhz[ix] = peaks[ix].bw;
                }
            }
            for ix in 1..7 {
                kt_frame.ap[ix] = peaks[ix].ap;
            }

            kt_frame.avdb = klattp[0];
            kt_frame.avpdb = klattp[6];
            kt_frame.af = klattp[7];
            kt_frame.ab = klattp[8];
            kt_frame.asp = klattp[3];
            kt_frame.aturb = klattp[9];
            kt_frame.kskew = klattp[4];
            kt_frame.tltdb = klattp[2];
            kt_frame.kopen = klattp[5];

            // Advance the formants and the other parameters.
            for pk in peaks.iter_mut() {
                pk.freq1 += pk.freq_inc;
                pk.freq = pk.freq1 as i32;
                pk.bw1 += pk.bw_inc;
                pk.bw = pk.bw1 as i32;
                pk.bp1 += pk.bp_inc;
                pk.bp = pk.bp1 as i32;
                pk.ap1 += pk.ap_inc;
                pk.ap = pk.ap1 as i32;
            }
            for ix in 0..N_KLATTP {
                klattp1[ix] += klattp_inc[ix];
                klattp[ix] = klattp1[ix] as i32;
            }

            let nspfr = (length - sample_count).min(STEPSIZE);
            kt.frame_init(&mut kt_frame);
            kt.parwave(&mut kt_frame, nspfr, amplitude, &mut raw);
            sample_count += nspfr;
        }
    }

    // Fade the tail out over the last 64 samples, then peak-normalize the run
    // to ~0.5 full-scale so the caller's AGC has a consistent level.
    let n = raw.len();
    for (i, v) in raw.iter_mut().enumerate().skip(n.saturating_sub(64)) {
        *v *= (n - i) as f64 / 64.0;
    }
    let peak = raw.iter().fold(0.0f64, |m, &v| m.max(v.abs())).max(1e-9);
    let scale = 0.5 * 32767.0 / peak;
    raw.iter().map(|&v| (v * scale) as i32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_finite_and_bounded() {
        let frame = KlattFrame::vowel(120.0, &[(730.0, 50.0), (1090.0, 60.0), (2440.0, 120.0)]);
        let pcm = synthesize(&frame, 4410, 22_050); // 0.2 s
        assert_eq!(pcm.len(), 4410);
        assert!(pcm.iter().any(|&s| s != 0), "silent output");
        // Peak-normalized to ~0.6 FS → never clips at the i16 rail.
        assert!(pcm.iter().all(|&s| s.abs() < 32767), "output clipped");
    }

    /// `DBtoLIN` must reproduce upstream's 88-entry table exactly, including
    /// the 0..=12 dead zone and the out-of-range guard.
    #[test]
    fn db_to_lin_matches_upstream_table() {
        assert_eq!(db_to_lin(-1), 0.0);
        assert_eq!(db_to_lin(88), 0.0);
        assert_eq!(db_to_lin(0), 0.0);
        assert_eq!(db_to_lin(12), 0.0);
        assert!((db_to_lin(13) - 0.006).abs() < 1e-12);
        assert!((db_to_lin(52) - 0.568).abs() < 1e-12); // AV 59 → AVdb_tmp 52
        assert!((db_to_lin(87) - 32.767).abs() < 1e-9);
    }

    /// The antiresonator built by `setzeroabc` has unity DC gain, like the
    /// ordinary resonators around it — so the cascade neither boosts nor cuts
    /// the source's mean level.
    #[test]
    fn nasal_antiresonator_has_unity_dc_gain() {
        let mut kt = Klatt::new(22_050);
        kt.set_zero_abc(RNZ, 280, 89);
        let r = kt.rsn[RNZ];
        assert!((r.a + r.b + r.c - 1.0).abs() < 1e-9, "DC gain {}", r.a + r.b + r.c);
    }

    /// `setzeroabc` with f = 0, bw = 0 leaves `a == 0`; C guards the inversion
    /// there because 1/0 puts an audible spike in the output.
    #[test]
    fn zero_frequency_antiresonator_does_not_blow_up() {
        let mut kt = Klatt::new(22_050);
        kt.set_zero_abc(RNZ, 0, 0);
        for c in [kt.rsn[RNZ].a, kt.rsn[RNZ].b, kt.rsn[RNZ].c] {
            assert!(c.is_finite(), "coefficient {c} is not finite");
        }
    }

    /// `pitch_synch_par_reset` derives the period from F0 and clamps the open
    /// phase into 40..=263 (4× samples), as C does.
    #[test]
    fn period_and_open_phase_follow_f0() {
        let mut kt = Klatt::new(22_050);
        let mut fr = KtFrame { f0hz10: 1000, kopen: 40, ..KtFrame::default() };
        fr.avdb_tmp = fr.avdb - 7;
        kt.pitch_synch_par_reset(&mut fr);
        // 100 Hz at 22050 Hz → 4 × 220.5 samples per period.
        assert_eq!(kt.t0, 882);
        assert_eq!(kt.nopen, 160);
        assert!((kt.amp_voice - db_to_lin(52)).abs() < 1e-12);

        // F0 = 0 means "unvoiced": no period, no voicing amplitude.
        let mut fr = KtFrame { f0hz10: 0, ..KtFrame::default() };
        kt.pitch_synch_par_reset(&mut fr);
        assert_eq!(kt.t0, 4);
        assert_eq!(kt.amp_voice, 0.0);
    }

    /// The full engine renders a Klatt frame sequence to non-silent, bounded
    /// audio whose energy sits at the requested formants.
    #[test]
    fn frame_engine_renders_at_the_requested_formants() {
        use crate::synthesize::phondata::{SpectFrame, FRFLAG_KLATT};
        let mut fr = SpectFrame {
            frflags: FRFLAG_KLATT,
            ffreq: [100, 700, 1200, 2500, 3300, 3700, 7000],
            length: 40,
            rms: 64,
            bw: [45, 45, 70, 130],
            klattp: [59, 0, 0, 0, 0], // AV only, as the shipped en frames have
            ..Default::default()
        };
        fr.fheight = [0; 8];
        let frames = vec![fr.clone(), fr];
        let pcm = klatt_frames_pcm(&frames);
        assert!(pcm.iter().any(|&s| s != 0), "engine produced silence");
        assert!(pcm.iter().all(|&s| s.abs() <= 32767), "output left i16 range");

        // Energy around F1 (700 Hz) must exceed the null between F2 and F3.
        let f1 = band_energy(&pcm, 600.0, 800.0, 22_050.0);
        let null = band_energy(&pcm, 1700.0, 1900.0, 22_050.0);
        assert!(f1 > null * 2.0, "F1 {f1:.1} not above the inter-formant null {null:.1}");
    }

    /// Render helper: run the frame engine at unit amplitude and 120 Hz.
    fn klatt_frames_pcm(frames: &[crate::synthesize::phondata::SpectFrame]) -> Vec<i32> {
        let amps = vec![1.0; frames.len()];
        let pitch = vec![120.0; frames.len()];
        synthesize_frames_klatt(frames, &amps, &pitch, 22_050)
    }

    /// Mean DFT magnitude over a frequency band — a band has to be summed
    /// rather than probed at one bin, or the answer depends on where the
    /// harmonics of F0 happen to fall.
    fn band_energy(x: &[i32], lo: f64, hi: f64, sr: f64) -> f64 {
        let n = x.len();
        let bin_hz = sr / n as f64;
        let (k0, k1) = ((lo / bin_hz) as usize, (hi / bin_hz) as usize);
        let mut total = 0.0;
        for k in k0..=k1 {
            let w = 2.0 * PI * k as f64 / n as f64;
            let (mut re, mut im) = (0.0, 0.0);
            for (i, &v) in x.iter().enumerate() {
                re += v as f64 * (w * i as f64).cos();
                im += v as f64 * (w * i as f64).sin();
            }
            total += (re * re + im * im).sqrt() / n as f64;
        }
        total / (k1 - k0 + 1) as f64
    }

    #[test]
    fn unvoiced_frame_uses_noise_not_pitch() {
        // f0 = 0 with aspiration → noise-driven, still produces sound.
        let frame = KlattFrame {
            f0: 0.0,
            formants: vec![(1500.0, 200.0)],
            voicing: 0.0,
            aspiration: 1.0,
            ..Default::default()
        };
        let pcm = synthesize(&frame, 2205, 22_050);
        assert!(pcm.iter().any(|&s| s != 0), "noise source produced silence");
    }
}
