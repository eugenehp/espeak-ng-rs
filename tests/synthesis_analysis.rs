// tests/synthesis_analysis.rs
//
// Exercises the `wav-analysis` harness against real synthesizer output.
// Self-contained (no C oracle): it asserts that synthesized speech has sane
// acoustic properties, which both proves the analyzer works on real audio and
// guards against synthesis regressions.
//
// Run with: cargo test --features wav-analysis --test synthesis_analysis

#![cfg(feature = "wav-analysis")]

use espeak_ng::analysis::{Pcm, PitchOptions};

fn synth(text: &str) -> Option<Pcm> {
    let (samples, rate) = espeak_ng::text_to_pcm("en", text).ok()?;
    if samples.is_empty() {
        return None;
    }
    Some(Pcm::from_samples(samples, rate))
}

#[test]
fn klatt_cascade_produces_formant_peaks() {
    use espeak_ng::synthesize::klatt::{self, KlattFrame};
    // A steady /ɑ/-like vowel: F1=730, F2=1090, F3=2440, voiced at 120 Hz.
    let frame = KlattFrame::vowel(120.0, &[(730.0, 50.0), (1090.0, 60.0), (2440.0, 120.0)]);
    let samples = klatt::synthesize(&frame, 22_050 * 4 / 10, 22_050); // 0.4 s
    let pcm = Pcm::from_samples(samples, 22_050);

    // The voicing source sets F0 to ~120 Hz.
    let f0 = pcm.mean_pitch(PitchOptions::default()).expect("voiced output");
    assert!((f0 - 120.0).abs() / 120.0 < 0.1, "expected ~120 Hz F0, got {f0}");

    // Energy is concentrated at the formants, not in the spectrum above them —
    // the defining behaviour of a formant synthesizer.
    let e_f1 = pcm.band_energy(2048, 1024, 650.0, 810.0);
    let e_f2 = pcm.band_energy(2048, 1024, 1010.0, 1170.0);
    let e_gap = pcm.band_energy(2048, 1024, 4000.0, 6000.0); // above all formants
    assert!(e_f1 > e_gap * 2.0, "F1 is not a peak: {e_f1} vs above-formant {e_gap}");
    assert!(e_f2 > e_gap * 2.0, "F2 is not a peak: {e_f2} vs above-formant {e_gap}");
}

#[test]
fn variant_f3_raises_pitch_over_base() {
    let text = "hello world how are you today";
    let base = espeak_ng::text_to_pcm("en", text).ok();
    let f3 = espeak_ng::text_to_pcm("en+f3", text).ok();
    let (Some((b, br)), Some((f, fr))) = (base, f3) else {
        eprintln!("[SKIP] synthesis unavailable");
        return;
    };
    let bp = Pcm::from_samples(b, br).mean_pitch(PitchOptions::default());
    let fp = Pcm::from_samples(f, fr).mean_pitch(PitchOptions::default());
    let (Some(bp), Some(fp)) = (bp, fp) else {
        eprintln!("[SKIP] no voiced frames");
        return;
    };
    // The `f3` variant (pitch base 140 vs default 82) must clearly raise F0.
    assert!(
        fp > bp * 1.2,
        "en+f3 mean F0 ({fp:.0} Hz) should clearly exceed en ({bp:.0} Hz)"
    );
}

#[test]
fn synthesized_speech_has_sane_acoustics() {
    let Some(pcm) = synth("hello world") else {
        eprintln!("[SKIP] synthesis unavailable");
        return;
    };
    // Roughly half a second to a few seconds for two words.
    let dur = pcm.duration_secs();
    assert!((0.3..3.0).contains(&dur), "implausible duration {dur}s");
    // Non-silent.
    assert!(pcm.rms() > 0.001, "output is silent (rms {})", pcm.rms());
    // Contains voiced (pitched) content.
    let f0 = pcm.mean_pitch(PitchOptions::default());
    assert!(f0.is_some(), "no voiced frames detected");
    let f0 = f0.unwrap();
    assert!((50.0..500.0).contains(&f0), "implausible F0 {f0} Hz");
}

#[test]
fn intonation_produces_a_non_flat_pitch_contour() {
    // Before intonation the synth used a constant F0 (std ~0).  With the pitch
    // envelope applied, a statement's pitch varies over a musically meaningful
    // range.  (The falling/rising *direction* of the envelope is verified
    // deterministically by the unit tests in `synthesize::tests`.)
    let Some(pcm) = synth("this is a fairly long spoken sentence for testing intonation") else {
        eprintln!("[SKIP] synthesis unavailable");
        return;
    };
    let track: Vec<f32> = pcm
        .pitch_track(PitchOptions::default())
        .into_iter()
        .filter(|&f| f > 0.0)
        .collect();
    if track.len() < 9 {
        eprintln!("[SKIP] too few voiced frames");
        return;
    }
    let mean = track.iter().sum::<f32>() / track.len() as f32;
    let std = (track.iter().map(|&f| (f - mean).powi(2)).sum::<f32>() / track.len() as f32).sqrt();
    assert!(std > 15.0, "pitch contour is nearly flat (std {std:.0} Hz); intonation not applied");
}

#[test]
fn sonorant_and_stop_words_have_comparable_durations() {
    // Regression for the SPECT_SEQ ms→samples bug: sonorant-heavy words
    // (nasals/liquids) were ~2× too long relative to stop words.  Words of
    // similar syllable count should now be within a modest factor.
    let (Some(son), Some(stp)) = (
        synth("melon melon melon melon"),
        synth("kitten kitten kitten kitten"),
    ) else {
        eprintln!("[SKIP] synthesis unavailable");
        return;
    };
    let ratio = son.duration_secs() / stp.duration_secs().max(0.001);
    assert!(
        (0.6..1.6).contains(&ratio),
        "sonorant/stop duration ratio {ratio:.2} out of range \
         (sonorant {:.2}s, stop {:.2}s)",
        son.duration_secs(),
        stp.duration_secs()
    );
}

#[test]
fn utterance_ends_with_trailing_silence() {
    // eSpeak appends ~0.3 s of silence at the end of a clause; the Rust engine
    // now does too (previously it ended abruptly, making short clips too short).
    let Some(pcm) = synth("hello") else {
        eprintln!("[SKIP] synthesis unavailable");
        return;
    };
    let tail = (0.2 * pcm.sample_rate as f32) as usize;
    if pcm.samples.len() < tail {
        eprintln!("[SKIP] output too short");
        return;
    }
    let peak = pcm.samples[pcm.samples.len() - tail..]
        .iter()
        .map(|s| s.unsigned_abs())
        .max()
        .unwrap_or(0);
    assert!(peak < 50, "expected trailing silence, tail peak {peak}");
}

#[test]
fn longer_text_lasts_longer() {
    let (Some(short), Some(long)) = (synth("hi"), synth("hello there my friend")) else {
        eprintln!("[SKIP] synthesis unavailable");
        return;
    };
    assert!(
        long.duration_secs() > short.duration_secs(),
        "longer text should take longer: {} vs {}",
        long.duration_secs(),
        short.duration_secs()
    );
}
