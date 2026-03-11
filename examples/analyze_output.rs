use std::path::Path;
use espeak_ng::phoneme::PhonemeData;
use espeak_ng::translate::Translator;
use espeak_ng::{Synthesizer, VoiceParams};

fn main() {
    let data_path = Path::new("/usr/share/espeak-ng-data");
    let mut phdata = PhonemeData::load(data_path).unwrap();
    phdata.select_table_by_name("en").unwrap();
    let translator = Translator::new_default("en").unwrap();
    let synth = Synthesizer::new(VoiceParams::default());
    
    let codes = translator.translate_to_codes("hello").unwrap();
    let our_pcm = synth.synthesize_codes(&codes, &phdata).unwrap();
    
    let n = our_pcm.len();
    let peak = our_pcm.iter().map(|&x| x.unsigned_abs()).max().unwrap_or(0);
    let rms = (our_pcm.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
    
    println!("Total: {} samples ({:.0}ms), peak={}, rms={:.0}", n, n as f64/22.05, peak, rms);
    
    // Amplitude histogram
    for threshold in &[100u16, 1000, 5000, 10000, 20000, 30000] {
        let c = our_pcm.iter().filter(|&&x| x.unsigned_abs() >= *threshold).count();
        println!("  |x|>={}: {} ({:.1}%)", threshold, c, c as f64/n as f64*100.0);
    }
    
    // Show first 30 samples
    println!("First 30 samples: {:?}", &our_pcm[..30.min(n)]);
}
