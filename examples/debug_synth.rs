//! Debug synthesis to see what's happening with short words
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
    
    for word in &["the", "one", "a"] {
        let codes = translator.translate_to_codes(word).unwrap();
        println!("{}: codes={:?}", word, codes.iter().map(|c| (c.code, c.is_boundary)).collect::<Vec<_>>());
        
        let pcm = synth.synthesize_codes(&codes, &phdata).unwrap();
        let rms = if pcm.is_empty() { 0.0 } else {
            (pcm.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / pcm.len() as f64).sqrt()
        };
        println!("  pcm_len={} ({:.0}ms) peak={} rms={:.0}", 
            pcm.len(), pcm.len() as f64 / 22.05,
            pcm.iter().map(|&x| x.unsigned_abs()).max().unwrap_or(0),
            rms);
        
        // Show nonzero regions
        let mut voiced_samples = 0;
        let mut silence_samples = 0;
        for &s in &pcm {
            if s.unsigned_abs() > 100 { voiced_samples += 1; } else { silence_samples += 1; }
        }
        println!("  voiced={}ms silence={}ms", voiced_samples/22, silence_samples/22);
    }
    
    // Check phoneme types for "the"
    println!("\n=== Phoneme types for 'the' ===");
    let codes = translator.translate_to_codes("the").unwrap();
    for code in &codes {
        if !code.is_boundary && code.code != 0 {
            if let Some(ph) = phdata.get(code.code) {
                println!("  code={} start_type={} program={}", code.code, ph.start_type, ph.program);
            }
        }
    }
}
