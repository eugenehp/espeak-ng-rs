//! Compare two WAV files on acoustic features (duration, energy, pitch).
//!
//! ```text
//! cargo run --example wav_compare --features wav-analysis -- reference.wav candidate.wav
//! ```
//!
//! Intended for checking synthesized audio (intonation, durations, sound icons)
//! against a reference WAV without requiring bit-exact PCM.

#[cfg(feature = "wav-analysis")]
fn main() {
    use espeak_ng::analysis::{compare, Pcm};

    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: wav_compare <reference.wav> <candidate.wav>");
        std::process::exit(2);
    }
    let a = Pcm::read_wav(&args[1]).expect("read reference");
    let b = Pcm::read_wav(&args[2]).expect("read candidate");

    let sim = compare(&a, &b);
    println!("reference : {:.3}s, {} Hz, rms {:.3}, mean F0 {:?}",
        a.duration_secs(), a.sample_rate, a.rms(), sim.mean_pitch_a.map(|f| f.round()));
    println!("candidate : {:.3}s, {} Hz, rms {:.3}, mean F0 {:?}",
        b.duration_secs(), b.sample_rate, b.rms(), sim.mean_pitch_b.map(|f| f.round()));
    println!("---");
    println!("duration ratio : {:.3}", sim.dur_ratio);
    println!("rms ratio      : {:.3}", sim.rms_ratio);
    println!("energy corr    : {:.3}", sim.energy_corr);
    println!("pitch corr     : {:.3}", sim.pitch_corr);
}

#[cfg(not(feature = "wav-analysis"))]
fn main() {
    eprintln!("rebuild with `--features wav-analysis`");
    std::process::exit(2);
}
