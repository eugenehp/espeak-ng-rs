//! Debug wavegen output for a single vowel
use std::path::Path;
use espeak_ng::phoneme::PhonemeData;
use espeak_ng::synthesize::{bytecode, phondata::SpectSeq, wavegen::synthesize_frames, VoiceParams};

fn main() {
    let data_path = Path::new("/usr/share/espeak-ng-data");
    let phondata_bytes = std::fs::read(data_path.join("phondata")).unwrap();
    let phonindex_bytes = std::fs::read(data_path.join("phonindex")).unwrap();
    let mut phdata = PhonemeData::load(data_path).unwrap();
    phdata.select_table_by_name("en").unwrap();
    
    // oU (code 144) - stressed vowel in "hello"
    if let Some(ph) = phdata.get(144) {
        let extract = bytecode::scan_phoneme(ph.program, &phonindex_bytes);
        println!("oU: fmt_addr={:?}", extract.fmt_addr);
        
        if let Some(addr) = extract.fmt_addr {
            if let Some(seq) = SpectSeq::parse(&phondata_bytes, addr as usize) {
                println!("frames={} klatt={}", seq.frames.len(), seq.is_klatt);
                let frame = &seq.frames[1]; // pick a middle frame
                println!("fheight={:?} ffreq={:?} fwidth={:?}", 
                    &frame.fheight[..6], &frame.ffreq[..6], &frame.fwidth[..6]);
                
                let voice = VoiceParams {
                    pitch_hz: 118,
                    amplitude: 100,
                    ..Default::default()
                };
                let mut wavephase: i32 = i32::MAX;
                let samples = synthesize_frames(&seq, &voice, 1.0, &mut wavephase);
                
                let n = samples.len();
                if n == 0 { println!("No samples generated!"); return; }
                
                let peak = samples.iter().map(|&x| x.abs()).max().unwrap_or(0);
                let rms = (samples.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
                println!("Generated {} samples ({:.0}ms), peak={}, rms={:.0}", 
                    n, n as f64/22.05, peak, rms);
                
                // Distribution
                for t in &[100i32, 1000, 5000, 10000, 20000, 30000] {
                    let c = samples.iter().filter(|&&x| x.abs() >= *t).count();
                    println!("  |x|>={}: {} ({:.1}%)", t, c, c as f64/n as f64*100.0);
                }
                
                // Show first 30 samples
                println!("First 30: {:?}", &samples[..30.min(n)]);
            }
        }
    }
}
