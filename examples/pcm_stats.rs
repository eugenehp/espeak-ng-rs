fn main() {
    let words = ["hello", "world", "one", "two", "three", "the", "a", "good morning"];
    for word in &words {
        let (pcm, rate) = match espeak_ng::text_to_pcm("en", word) {
            Ok(x) => x,
            Err(e) => { eprintln!("{word}: error: {e}"); continue; }
        };
        let peak = pcm.iter().map(|x| x.unsigned_abs()).max().unwrap_or(0);
        let rms = { let s: f64 = pcm.iter().map(|x| (*x as f64).powi(2)).sum();
                    (s / pcm.len() as f64).sqrt() };
        let dur_ms = pcm.len() as f32 * 1000.0 / rate as f32;
        println!("{word:15}: {:.0}ms  peak={peak}  rms={rms:.0}", dur_ms);
    }
}
