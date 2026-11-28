//! The SpeechPlayer synthesizer, selected by `klatt 6` in a voice.
//!
//! Port of `sPlayer.c` and the parts of `src/speechPlayer` that it drives: a
//! sawtooth glottal source through a cascade of formant resonators.  eSpeak
//! never sets the turbulence, vibrato, frication or parallel-branch parameters,
//! so those stages are always silent there and are left out here.

use super::phondata::SpectFrame;
use super::VoiceParams;
use crate::bytes::Bounded;

/// Samples over which one frame fades into the next (`minFadeLength`).
const MIN_FADE: usize = 110;
const N_CASCADE: usize = 6;

/// `speechPlayer_frame_t`, without the parameters eSpeak leaves at zero.
#[derive(Debug, Clone, Copy, Default)]
struct Frame {
    voice_pitch: f64,
    voice_amplitude: f64,
    aspiration: f64,
    /// Cascade formant frequencies and bandwidths, F1–F6.
    cf: [f64; N_CASCADE],
    cb: [f64; N_CASCADE],
    /// Nasal zero and pole.
    cf_n0: f64,
    cb_n0: f64,
    cf_np: f64,
    cb_np: f64,
    /// How much of the nasal branch is heard.
    ca_np: f64,
    pre_formant_gain: f64,
    output_gain: f64,
    end_voice_pitch: f64,
}

impl Frame {
    /// `fillSpeechPlayerFrame`.
    fn new(fr: &SpectFrame, pitch: f64, amp: f64, voice: &VoiceParams) -> Self {
        let mut frame = Frame {
            voice_pitch: pitch,
            voice_amplitude: f64::from(voice.voicing) / 64.0,
            aspiration: f64::from(voice.breath.at(1)) / 64.0,
            cf_n0: 250.0,
            cb_n0: 100.0,
            cf_np: 200.0,
            cb_np: 100.0,
            pre_formant_gain: 1.0,
            output_gain: 3.0 * amp / 100.0,
            end_voice_pitch: pitch,
            ..Frame::default()
        };
        for (ix, cf) in frame.cf.iter_mut().enumerate() {
            *cf = voice.formants.freq(ix + 1, fr.ffreq.at(ix + 1)) as f64 / 256.0;
        }
        let widths = voice.formants.width.iter().skip(1);
        for ((cb, &bw), &width) in frame.cb.iter_mut().zip(&fr.bw).zip(widths) {
            *cb = f64::from(bw) * 2.0 * f64::from(width) / 256.0;
        }
        for cb in frame.cb.iter_mut().skip(4) {
            *cb = 1000.0;
        }
        let nasal_zero = fr.klattp.at(1);
        if nasal_zero > 0 {
            frame.ca_np = 1.0;
            frame.cf_n0 = f64::from(nasal_zero) * 2.0;
        }
        frame
    }

    /// The frame `ratio` of the way from `self` to `to`.
    fn fade(&self, to: &Frame, ratio: f64) -> Frame {
        let mix = |a: f64, b: f64| a + (b - a) * ratio;
        let mut out = Frame {
            voice_pitch: mix(self.voice_pitch, to.voice_pitch),
            voice_amplitude: mix(self.voice_amplitude, to.voice_amplitude),
            aspiration: mix(self.aspiration, to.aspiration),
            cf: self.cf,
            cb: self.cb,
            cf_n0: mix(self.cf_n0, to.cf_n0),
            cb_n0: mix(self.cb_n0, to.cb_n0),
            cf_np: mix(self.cf_np, to.cf_np),
            cb_np: mix(self.cb_np, to.cb_np),
            ca_np: mix(self.ca_np, to.ca_np),
            pre_formant_gain: mix(self.pre_formant_gain, to.pre_formant_gain),
            output_gain: mix(self.output_gain, to.output_gain),
            end_voice_pitch: mix(self.end_voice_pitch, to.end_voice_pitch),
        };
        for (x, &b) in out.cf.iter_mut().zip(&to.cf) {
            *x = mix(*x, b);
        }
        for (x, &b) in out.cb.iter_mut().zip(&to.cb) {
            *x = mix(*x, b);
        }
        out
    }
}

/// A queued frame (`frameRequest_t`).
#[derive(Debug, Clone, Copy)]
struct Request {
    min_samples: usize,
    fade_samples: usize,
    frame: Frame,
    pitch_inc: f64,
}

impl Request {
    fn new(frame: Frame, samples: usize) -> Self {
        Request {
            min_samples: samples,
            fade_samples: samples,
            frame,
            pitch_inc: (frame.end_voice_pitch - frame.voice_pitch) / samples.max(1) as f64,
        }
    }
}

/// `FrameManagerImpl`: steps through the queued frames a sample at a time.
#[derive(Default)]
struct FrameManager {
    queue: std::collections::VecDeque<Request>,
    old: Option<Request>,
    new: Option<Request>,
    current: Frame,
    counter: usize,
}

impl FrameManager {
    /// Advance one sample; `None` once the queue has run out.
    fn next_frame(&mut self) -> Option<Frame> {
        self.counter += 1;
        if let Some(new) = self.new {
            if self.counter > new.fade_samples {
                self.old = Some(new);
                self.new = None;
            } else if let Some(old) = self.old {
                let ratio = self.counter as f64 / new.fade_samples as f64;
                self.current = old.frame.fade(&new.frame, ratio);
            }
        } else if self.old.is_none_or(|old| self.counter > old.min_samples) {
            let mut new = self.queue.pop_front()?;
            // Nothing before the first frame: it fades in from silence.
            self.old.get_or_insert_with(|| {
                let mut silent = new;
                silent.frame.pre_formant_gain = 0.0;
                silent
            });
            self.counter = 0;
            new.frame.voice_pitch += new.pitch_inc * new.fade_samples as f64;
            self.new = Some(new);
        } else if let Some(old) = &mut self.old {
            self.current.voice_pitch += old.pitch_inc;
            old.frame.voice_pitch = self.current.voice_pitch;
        }
        Some(self.current)
    }
}

/// `Resonator`, or with `anti` an anti-resonator.
#[derive(Default)]
struct Resonator {
    anti: bool,
    p1: f64,
    p2: f64,
}

impl Resonator {
    fn resonate(&mut self, input: f64, frequency: f64, bandwidth: f64, rate: f64) -> f64 {
        use std::f64::consts::PI;
        let r = (-PI / rate * bandwidth).exp();
        let mut c = -(r * r);
        let mut b = r * (2.0 * PI / rate * frequency).cos() * 2.0;
        let mut a = 1.0 - b - c;
        if self.anti && frequency != 0.0 {
            a = 1.0 / a;
            c *= -a;
            b *= -a;
        }
        let out = a * input + b * self.p1 + c * self.p2;
        self.p2 = self.p1;
        self.p1 = if self.anti { input } else { out };
        out
    }
}

/// `SpeechWaveGeneratorImpl`.
struct Generator {
    rate: f64,
    cycle: f64,
    noise_seed: u32,
    noise: f64,
    nasal_zero: Resonator,
    nasal_pole: Resonator,
    cascade: [Resonator; N_CASCADE],
}

impl Generator {
    fn new(sample_rate: u32) -> Self {
        Generator {
            rate: sample_rate.into(),
            cycle: 0.0,
            noise_seed: 1,
            noise: 0.0,
            nasal_zero: Resonator {
                anti: true,
                ..Resonator::default()
            },
            nasal_pole: Resonator::default(),
            cascade: Default::default(),
        }
    }

    fn sample(&mut self, frame: &Frame) -> i32 {
        self.cycle = (frame.voice_pitch / self.rate + self.cycle) % 1.0;
        self.noise_seed = self
            .noise_seed
            .wrapping_mul(1_103_515_245)
            .wrapping_add(12_345);
        let white = f64::from((self.noise_seed >> 16) & 0x7fff) / f64::from(0x7fff);
        self.noise = white + 0.75 * self.noise;

        let voice = (self.cycle * 2.0 - 1.0) * frame.voice_amplitude;
        let source = voice + self.noise * 0.2 * frame.aspiration;
        let input = source * frame.pre_formant_gain / 2.0;
        let zero = self
            .nasal_zero
            .resonate(input, frame.cf_n0, frame.cb_n0, self.rate);
        let nasal = self
            .nasal_pole
            .resonate(zero, frame.cf_np, frame.cb_np, self.rate);
        let mut out = input + (nasal - input) * frame.ca_np;
        let formants = self.cascade.iter_mut().zip(frame.cf.iter().zip(&frame.cb));
        for (resonator, (&cf, &cb)) in formants.rev() {
            out = resonator.resonate(out, cf, cb, self.rate);
        }
        (out * frame.output_gain * 4000.0).clamp(-32000.0, 32000.0) as i32
    }
}

/// Render one unbroken run of frames (`Wavegen_KlattSP`, one call per frame).
pub fn synthesize_frames_sp(
    frames: &[SpectFrame],
    amps: &[f64],
    pitch: &[f64],
    voice: &VoiceParams,
) -> Vec<i32> {
    let mut manager = FrameManager::default();
    let mut generator = Generator::new(voice.sample_rate);
    let mut out = Vec::new();
    // `SetAmplitude` at stress amplitude 1.0, compensated for an echo.
    let echo = f64::from(500 - voice.echo_amp.clamp(0, 100)) / 500.0;
    let general = 75.625 * f64::from(voice.amplitude.min(100)) / 50.0 * echo;

    for (i, fr) in frames.iter().enumerate() {
        let length = fr.duration();
        if length == 0 {
            continue;
        }
        let next = frames.get(i + 1).unwrap_or(fr);
        let amp = amps.get(i).copied().unwrap_or(1.0) * general;
        let start = pitch.get(i).copied().unwrap_or(120.0);
        let end = pitch.get(i + 1).copied().unwrap_or(start);

        let first = Frame::new(fr, start, amp, voice);
        let mut second = Frame::new(next, start, amp, voice);
        second.end_voice_pitch = end.trunc();
        let last = !frames.iter().skip(i + 1).any(|f| f.duration() > 0);

        manager.queue.push_back(Request::new(first, MIN_FADE));
        let fades = if last { 2 } else { 1 };
        let main = length.saturating_sub(MIN_FADE * fades);
        if main > 0 {
            manager.queue.push_back(Request::new(second, main));
        }
        if last {
            second.voice_pitch = second.end_voice_pitch;
            second.pre_formant_gain = 0.0;
            manager.queue.push_back(Request::new(second, MIN_FADE / 2));
            second.output_gain = 0.0;
            manager.queue.push_back(Request::new(second, MIN_FADE / 2));
        }
        while let Some(frame) = manager.next_frame() {
            out.push(generator.sample(&frame));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vowel() -> SpectFrame {
        SpectFrame {
            ffreq: [0, 700, 1200, 2500, 3300, 3700, 4500],
            bw: [30, 40, 60, 80],
            samples: Some(2000),
            ..SpectFrame::default()
        }
    }

    #[test]
    fn renders_about_as_many_samples_as_the_frames_last() {
        let voice = VoiceParams::default();
        let out = synthesize_frames_sp(&[vowel(), vowel()], &[1.0; 2], &[120.0; 2], &voice);
        assert!((3990..4020).contains(&out.len()), "{}", out.len());
        assert!(out.iter().any(|v| v.abs() > 1000));
        // It fades in and out.
        assert!(out[..3].iter().all(|v| v.abs() < 500));
        assert_eq!(out.last(), Some(&0));
    }

    #[test]
    fn no_frames_is_silence() {
        assert!(synthesize_frames_sp(&[], &[], &[], &VoiceParams::default()).is_empty());
    }
}
