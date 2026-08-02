//! `spectral` — deterministic WAV/PCM spectral analysis & comparison.
//!
//! A dependency-free (pure-Rust FFT) alternative to ASR for diagnosing
//! synthesis quality — especially the spectral character of consonants
//! (fricatives/stops), which whisper can only judge indirectly and noisily.
//!
//! Usage:
//!   cargo run --features wav-analysis --bin spectral -- <a.wav> [b.wav]
//!
//! One file  → prints its spectral features (centroid, flatness, roll-off,
//!             band energies, mean pitch, RMS, duration).
//! Two files → the above for both, plus a comparison (spectral-shape
//!             correlation, centroid ratio, duration/RMS/energy/pitch).
//!
//! Example (compare the Rust engine to the C oracle on a fricative word):
//!   ./target/release/espeak-ng-rs -q -w /tmp/r.wav "sea shells"
//!   espeak-ng -q -w /tmp/o.wav "sea shells"
//!   cargo run --features wav-analysis --bin spectral -- /tmp/o.wav /tmp/r.wav

use espeak_ng::analysis::{compare, Pcm, PitchOptions};

const FFT: usize = 1024;
const HOP: usize = 256;

fn features(name: &str, p: &Pcm) {
    let pitch = p
        .mean_pitch(PitchOptions::default())
        .map(|f| format!("{f:.0} Hz"))
        .unwrap_or_else(|| "unvoiced".into());
    println!("{name}");
    println!("  duration        {:.3} s ({} samples @ {} Hz)", p.duration_secs(), p.samples.len(), p.sample_rate);
    println!("  rms             {:.4}", p.rms());
    println!("  mean pitch      {pitch}");
    println!("  centroid        {:.0} Hz", p.spectral_centroid(FFT, HOP));
    println!("  flatness        {:.3}  (0 tonal … 1 noise)", p.spectral_flatness(FFT, HOP));
    println!("  rolloff(85%)    {:.0} Hz", p.spectral_rolloff(FFT, HOP, 0.85));
    println!(
        "  band energy     0–1k={:.2}  1–3k={:.2}  3–6k={:.2}  6k+={:.2}",
        p.band_energy(FFT, HOP, 0.0, 1000.0),
        p.band_energy(FFT, HOP, 1000.0, 3000.0),
        p.band_energy(FFT, HOP, 3000.0, 6000.0),
        p.band_energy(FFT, HOP, 6000.0, p.sample_rate as f32 / 2.0),
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.len() > 2 {
        eprintln!("usage: spectral <a.wav> [b.wav]");
        std::process::exit(2);
    }

    let load = |path: &str| -> Pcm {
        Pcm::read_wav(path).unwrap_or_else(|e| {
            eprintln!("error reading {path}: {e}");
            std::process::exit(1);
        })
    };

    let a = load(&args[0]);
    features(&args[0], &a);

    if args.len() == 2 {
        let b = load(&args[1]);
        println!();
        features(&args[1], &b);

        let sim = compare(&a, &b);
        println!("\ncomparison ({} = reference, {} = candidate)", args[0], args[1]);
        println!("  spectral shape corr   {:.3}  (1.0 = identical timbre)", sim.spectral_corr);
        println!("  centroid              {:.0} → {:.0} Hz", sim.centroid_a, sim.centroid_b);
        println!("  duration ratio        {:.3}", sim.dur_ratio);
        println!("  rms ratio             {:.3}", sim.rms_ratio);
        println!("  energy-envelope corr  {:.3}", sim.energy_corr);
        println!("  pitch corr            {:.3}", sim.pitch_corr);
    }
}
