fn main() {
    use std::path::Path;
    use espeak_ng::phoneme::PhonemeData;
    use espeak_ng::translate::Translator;
    use espeak_ng::{Synthesizer, VoiceParams};
    
    let data_path = Path::new("/usr/share/espeak-ng-data");
    let mut phdata = PhonemeData::load(data_path).unwrap();
    phdata.select_table_by_name("en").unwrap();
    
    let translator = Translator::new_default("en").unwrap();
    
    for word in &["ah", "hello", "the"] {
        let codes = translator.translate_to_codes(word).unwrap();
        println!("{}: codes={:?}", word, codes.iter().map(|c| (c.code, c.is_boundary)).collect::<Vec<_>>());
        
        let synth = Synthesizer::new(VoiceParams::default());
        let pcm = synth.synthesize_codes(&codes, &phdata).unwrap();
        let rms = if pcm.is_empty() { 0.0 } else {
            (pcm.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / pcm.len() as f64).sqrt()
        };
        println!("  pcm len={}, peak={}, rms={:.1}",
            pcm.len(),
            pcm.iter().map(|&x| x.unsigned_abs()).max().unwrap_or(0),
            rms);
    }
}
