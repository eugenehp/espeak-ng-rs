//! Detailed parity check against espeak-ng CLI
use std::path::Path;
use std::process::Command;
use espeak_ng::phoneme::PhonemeData;
use espeak_ng::translate::Translator;
use espeak_ng::{Synthesizer, VoiceParams};

fn rms(pcm: &[i16]) -> f64 {
    if pcm.is_empty() { return 0.0; }
    (pcm.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / pcm.len() as f64).sqrt()
}

fn peak(pcm: &[i16]) -> i16 {
    pcm.iter().map(|&x| x.unsigned_abs()).max().unwrap_or(0) as i16
}

fn pct_above(pcm: &[i16], threshold: u16) -> f64 {
    if pcm.is_empty() { return 0.0; }
    pcm.iter().filter(|&&x| x.unsigned_abs() >= threshold).count() as f64 / pcm.len() as f64 * 100.0
}

fn c_synthesize(text: &str) -> Vec<i16> {
    let out = Command::new("espeak-ng")
        .args(["--stdout", "-v", "en", text])
        .output()
        .expect("espeak-ng not found");
    let data = &out.stdout;
    if data.len() < 44 { return vec![]; }
    data[44..].chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn main() {
    let data_path = Path::new("/usr/share/espeak-ng-data");
    let mut phdata = PhonemeData::load(data_path).unwrap();
    phdata.select_table_by_name("en").unwrap();
    let translator = Translator::new_default("en").unwrap();
    let synth = Synthesizer::new(VoiceParams::default());

    let words = [
        "hello", "world", "one", "two", "three",
        "the", "a", "good", "walked", "happily",
        "espeak", "synthesis", "phoneme",
    ];

    const SR: f64 = 22050.0;

    println!("{:<12} {:>7} {:>7} {:>6} {:>7} {:>7} {:>7} {:>7}",
        "word", "our_ms", "c_ms", "ratio", "our_pk", "c_pk", "our_rms", "c_rms");
    println!("{}", "─".repeat(75));

    let mut dur_ratios = Vec::new();
    let mut rms_ratios = Vec::new();

    for word in &words {
        let codes = match translator.translate_to_codes(word) {
            Ok(c) => c,
            Err(_) => { println!("{:<12} ERROR", word); continue; }
        };
        let our_pcm = synth.synthesize_codes(&codes, &phdata).unwrap_or_default();
        let c_pcm   = c_synthesize(word);

        let our_ms  = our_pcm.len() as f64 / SR * 1000.0;
        let c_ms    = c_pcm.len()   as f64 / SR * 1000.0;
        let ratio   = if c_ms > 0.0 { our_ms / c_ms } else { 0.0 };
        let our_pk  = peak(&our_pcm);
        let c_pk    = peak(&c_pcm);
        let our_rms = rms(&our_pcm);
        let c_rms   = rms(&c_pcm);

        println!("{:<12} {:>7.0} {:>7.0} {:>6.2} {:>7} {:>7} {:>7.0} {:>7.0}",
            word, our_ms, c_ms, ratio, our_pk, c_pk, our_rms, c_rms);

        if c_ms > 0.0 { dur_ratios.push(ratio); }
        if c_rms > 0.0 { rms_ratios.push(our_rms / c_rms); }
    }

    println!("{}", "─".repeat(75));

    if !dur_ratios.is_empty() {
        let mean_dur = dur_ratios.iter().sum::<f64>() / dur_ratios.len() as f64;
        let mean_rms = rms_ratios.iter().sum::<f64>() / rms_ratios.len() as f64;
        let max_dur_err = dur_ratios.iter().map(|&r| (r - 1.0).abs()).fold(0.0f64, f64::max);
        println!("mean dur_ratio={:.2}  max_dur_err={:.2}  mean_rms_ratio={:.1}x",
            mean_dur, max_dur_err, mean_rms);
    }

    println!();
    println!("=== Amplitude distribution for 'hello' ===");
    let codes = translator.translate_to_codes("hello").unwrap();
    let our_pcm = synth.synthesize_codes(&codes, &phdata).unwrap_or_default();
    let c_pcm   = c_synthesize("hello");
    for thr in &[100u16, 1000, 5000, 10000, 20000] {
        println!("  |x|>={:>5}: our={:5.1}%  c={:5.1}%",
            thr, pct_above(&our_pcm, *thr), pct_above(&c_pcm, *thr));
    }
}
