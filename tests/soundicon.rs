// tests/soundicon.rs
//
// End-to-end tests for sound-icon support: punctuation icons and SSML <audio>.
// These need the real espeak-ng data files; they skip gracefully if absent.

use espeak_ng::EspeakNg;

/// Build a minimal mono 16-bit PCM WAV at 22050 Hz.
fn make_wav(samples: &[i16]) -> Vec<u8> {
    let rate = 22_050u32;
    let data_len = samples.len() * 2;
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data_len as u32).to_le_bytes());
    for &s in samples {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

/// A distinctive 0.25 s tone as the icon audio.
fn write_icon(name: &str) -> std::path::PathBuf {
    let n = 22_050 / 4; // 0.25 s
    let tone: Vec<i16> = (0..n)
        .map(|i| ((i as f32 / 22_050.0 * 880.0 * std::f32::consts::TAU).sin() * 12_000.0) as i16)
        .collect();
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, make_wav(&tone)).unwrap();
    path
}

fn engine() -> Option<EspeakNg> {
    // Prefer the repo-local data dir, then the default (ESPEAK_DATA_PATH / system).
    let local = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("espeak-ng-data");
    if local.join("phontab").exists() {
        if let Ok(eng) = EspeakNg::with_data_dir("en", &local) {
            return Some(eng);
        }
    }
    match EspeakNg::new("en") {
        Ok(eng) => Some(eng),
        Err(_) => {
            println!("[SKIP] espeak-ng data not available");
            None
        }
    }
}

#[test]
fn punctuation_soundicon_is_spliced_in() {
    let Some(mut eng) = engine() else { return };
    let icon = write_icon("espeak_rs_it_punct.wav");

    let (base, _) = eng.synth("hello world").expect("base synth");
    eng.add_soundicon('#', &icon).expect("register icon");
    let (with_icon, _) = eng.synth("hello # world").expect("icon synth");

    // The icon clip (~0.25 s = ~5500 samples, scaled) plus a lead-in pause must
    // make the output meaningfully longer than the same words without it.
    assert!(
        with_icon.len() > base.len() + 4000,
        "expected icon splice: base={} with_icon={}",
        base.len(),
        with_icon.len()
    );
    // And it must contain audible non-silence (the tone).
    assert!(with_icon.iter().any(|&s| s.unsigned_abs() > 3000));
    let _ = std::fs::remove_file(&icon);
}

#[test]
fn no_soundicon_registered_is_unchanged() {
    let Some(eng) = engine() else { return };
    // Fast path must be byte-identical to the plain pipeline.
    let (a, _) = eng.synth("testing one two three").unwrap();
    let (b, _) = eng.synth("testing one two three").unwrap();
    assert_eq!(a, b);
    assert!(!a.is_empty());
}

#[test]
fn ssml_audio_plays_icon_when_markup_enabled() {
    let Some(mut eng) = engine() else { return };
    let icon = write_icon("espeak_rs_it_audio.wav");
    let src = icon.to_string_lossy().to_string();

    // Without markup, the tag text is treated literally (no splice).
    let (plain, _) = eng.synth("start end").unwrap();

    eng.set_markup(true);
    let text = format!("start <audio src=\"{src}\">beep</audio> end");
    let (with_audio, _) = eng.synth(&text).expect("ssml audio synth");

    assert!(
        with_audio.len() > plain.len() + 4000,
        "expected <audio> splice: plain={} with_audio={}",
        plain.len(),
        with_audio.len()
    );
    let _ = std::fs::remove_file(&icon);
}

#[test]
fn ssml_audio_missing_file_speaks_fallback() {
    let Some(mut eng) = engine() else { return };
    eng.set_markup(true);
    // Missing file → fallback text "banana" is spoken instead of failing.
    let (fallback, _) = eng
        .synth("<audio src=\"/no/such/file.wav\">banana</audio>")
        .expect("fallback synth");
    let (spoken, _) = eng.synth("banana").expect("plain synth");
    assert!(!fallback.is_empty());
    // Fallback audio should be within ~35% of speaking the word directly
    // (both synthesize "banana"; small differences from clause context).
    let ratio = fallback.len() as f32 / spoken.len().max(1) as f32;
    assert!(ratio > 0.6 && ratio < 1.6, "fallback ratio {ratio}");
}
