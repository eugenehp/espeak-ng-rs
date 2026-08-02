//! Sound icons — external WAV clips spliced inline into synthesised speech.
//!
//! Port of eSpeak NG's `soundicon.c` plus its two trigger paths:
//!
//! 1. **SSML `<audio src="…">fallback</audio>`** (ssml.c → `LoadSoundFile2`) —
//!    plays an arbitrary WAV file; if the file can't be loaded the *fallback*
//!    text between the tags is spoken instead.
//! 2. **Punctuation announcement** (readclause.c → `LookupSoundicon`) — a
//!    punctuation character mapped to a WAV via a voice's `soundicon <ch> <file>`
//!    directive is replaced by the clip.
//!
//! Unlike the C loader (which requires the file to already be mono / 16-bit / at
//! the engine sample rate and otherwise punts to a commented-out `sox` call),
//! this decoder is robust: it accepts PCM (8/16/24/32-bit) and IEEE-float WAVs,
//! any channel count, and any sample rate, down-mixing to mono and resampling to
//! the engine rate.
//!
//! Sound icons carry **no shipped data** — they are entirely opt-in.  Register
//! punctuation icons with [`EspeakNg::add_soundicon`](crate::EspeakNg::add_soundicon)
//! and enable SSML `<audio>` with
//! [`EspeakNg::set_markup`](crate::EspeakNg::set_markup).

use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Maximum number of sound-icon slots (mirrors `N_SOUNDICON_TAB`).
pub const N_SOUNDICON_TAB: usize = 80;

/// A single sound-icon entry (mirrors `SOUND_ICON`).
#[derive(Debug, Clone, Default)]
pub struct SoundIcon {
    /// Punctuation character that triggers this icon (`None` for `<audio>` icons).
    pub name: Option<char>,
    /// Decoded mono 16-bit PCM at the engine sample rate (empty until loaded).
    pub samples: Vec<i16>,
    /// Source file path (used for lazy loading and de-duplication).
    pub filename: Option<PathBuf>,
}

/// The engine's table of registered / loaded sound icons.
#[derive(Debug, Clone)]
pub struct SoundIconTable {
    icons: Vec<SoundIcon>,
    sample_rate: u32,
}

impl SoundIconTable {
    /// Create an empty table targeting `sample_rate` Hz (icons are resampled to
    /// this rate on load).
    pub fn new(sample_rate: u32) -> Self {
        SoundIconTable { icons: Vec::new(), sample_rate: sample_rate.max(1) }
    }

    /// Number of registered icons.
    pub fn len(&self) -> usize {
        self.icons.len()
    }

    /// `true` if no icons are registered (the common case; enables the fast path).
    pub fn is_empty(&self) -> bool {
        self.icons.is_empty()
    }

    /// The icon at `idx`, if any.
    pub fn get(&self, idx: usize) -> Option<&SoundIcon> {
        self.icons.get(idx)
    }

    /// The decoded PCM of icon `idx` (empty slice if out of range / unloaded).
    pub fn samples(&self, idx: usize) -> &[i16] {
        self.icons.get(idx).map(|i| i.samples.as_slice()).unwrap_or(&[])
    }

    /// Register (and eagerly load) a punctuation → file mapping.  Mirrors the
    /// `soundicon <char> <file>` voice directive; returns the icon index.
    ///
    /// `base_dir` resolves relative paths (typically `<data>/soundicons`).
    pub fn register_punct(&mut self, ch: char, path: impl Into<PathBuf>, base_dir: &Path) -> Result<usize> {
        // If this character is already registered, reload it in place.
        if let Some(idx) = self.icons.iter().position(|i| i.name == Some(ch)) {
            self.icons[idx].filename = Some(path.into());
            self.icons[idx].samples.clear();
            self.load(idx, base_dir)?;
            return Ok(idx);
        }
        if self.icons.len() >= N_SOUNDICON_TAB {
            return Err(Error::InvalidData("soundicon table full".into()));
        }
        self.icons.push(SoundIcon { name: Some(ch), samples: Vec::new(), filename: Some(path.into()) });
        let idx = self.icons.len() - 1;
        self.load(idx, base_dir)?;
        Ok(idx)
    }

    /// Look up (lazily loading if needed) the icon for a punctuation character.
    /// Mirrors `LookupSoundicon`; returns `None` if there is no mapping or the
    /// file can't be loaded.
    pub fn lookup_punct(&mut self, ch: char, base_dir: &Path) -> Option<usize> {
        let idx = self.icons.iter().position(|i| i.name == Some(ch))?;
        if self.icons[idx].samples.is_empty() && self.load(idx, base_dir).is_err() {
            return None;
        }
        Some(idx)
    }

    /// Load an arbitrary WAV file (SSML `<audio src>`), de-duplicating by path.
    /// Mirrors `LoadSoundFile2`; returns the icon index.
    pub fn load_file(&mut self, path: impl Into<PathBuf>, base_dir: &Path) -> Result<usize> {
        let path = path.into();
        if let Some(idx) = self.icons.iter().position(|i| i.filename.as_deref() == Some(path.as_path())) {
            if self.icons[idx].samples.is_empty() {
                self.load(idx, base_dir)?;
            }
            return Ok(idx);
        }
        if self.icons.len() >= N_SOUNDICON_TAB {
            return Err(Error::InvalidData("soundicon table full".into()));
        }
        self.icons.push(SoundIcon { name: None, samples: Vec::new(), filename: Some(path) });
        let idx = self.icons.len() - 1;
        self.load(idx, base_dir)?;
        Ok(idx)
    }

    /// Decode the WAV file for icon `idx` into mono 16-bit PCM at the table rate.
    fn load(&mut self, idx: usize, base_dir: &Path) -> Result<()> {
        let fname = self.icons[idx]
            .filename
            .clone()
            .ok_or_else(|| Error::InvalidData("soundicon has no filename".into()))?;
        let full = resolve_path(&fname, base_dir);
        let bytes = std::fs::read(&full).map_err(Error::Io)?;
        let samples = decode_wav_mono16(&bytes, self.sample_rate)?;
        if samples.is_empty() {
            return Err(Error::InvalidData(format!(
                "soundicon file has no audio: {}",
                full.display()
            )));
        }
        self.icons[idx].samples = samples;
        Ok(())
    }
}

/// Resolve `fname` against `base_dir` for relative paths (mirrors C's
/// `path_home/soundicons/<file>`); absolute paths are used verbatim.
fn resolve_path(fname: &Path, base_dir: &Path) -> PathBuf {
    if fname.is_absolute() {
        fname.to_path_buf()
    } else {
        base_dir.join(fname)
    }
}

/// Scale a sound icon's PCM by the same factor eSpeak NG's `PlayWave` applies to
/// the `EMBED_I` (sound-icon) command: a `WCMD_WAVE` with 16-bit data and amp
/// `0x15` (21):
///
/// `out = sample * consonant_amp(26) * general_amplitude(55) >> 10 * 21 / 32`
/// (≈ 0.92×).
pub fn scale_icon_samples(samples: &[i16]) -> Vec<i16> {
    const CONSONANT_AMP: i32 = 26;
    const GENERAL_AMPLITUDE: i32 = 55;
    const AMP: i32 = 21;
    samples
        .iter()
        .map(|&s| {
            let v = (s as i32 * CONSONANT_AMP * GENERAL_AMPLITUDE) >> 10;
            (v * AMP / 32).clamp(-32768, 32767) as i16
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Text → pieces (the two trigger paths)
// ---------------------------------------------------------------------------

/// One unit of a piece-split input: a run of speakable text, or a sound-icon
/// event (index into the [`SoundIconTable`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// Text to be synthesised normally.
    Text(String),
    /// Play the sound icon at this table index.
    Icon(usize),
}

/// Split `text` into speakable runs and sound-icon events.
///
/// * When `markup` is set, `<audio src="…">fallback</audio>` (and the
///   self-closing form) is recognised: the file is loaded via
///   [`SoundIconTable::load_file`] and becomes an [`Piece::Icon`]; if it fails to
///   load, the fallback text between the tags is kept and spoken.  A bare
///   `</audio>` is dropped.
/// * Any character with a registered punctuation icon becomes an
///   [`Piece::Icon`] (the character itself is consumed).
pub fn split_pieces(
    text: &str,
    table: &mut SoundIconTable,
    base_dir: &Path,
    markup: bool,
) -> Vec<Piece> {
    let chars: Vec<char> = text.chars().collect();
    let mut pieces: Vec<Piece> = Vec::new();
    let mut cur = String::new();
    let mut i = 0;

    macro_rules! flush {
        () => {
            if !cur.is_empty() {
                pieces.push(Piece::Text(std::mem::take(&mut cur)));
            }
        };
    }

    while i < chars.len() {
        // ---- SSML <audio src="..."> ------------------------------------
        if markup && chars[i] == '<' && starts_with_ci(&chars, i + 1, "audio") {
            if let Some((src, after_open, self_closing)) = parse_audio_open(&chars, i) {
                match table.load_file(&src, base_dir) {
                    Ok(idx) => {
                        flush!();
                        pieces.push(Piece::Icon(idx));
                        // Skip the fallback content and the closing tag.
                        i = if self_closing {
                            after_open
                        } else {
                            skip_to_close(&chars, after_open, "audio")
                        };
                        continue;
                    }
                    Err(_) => {
                        // Load failed → keep the fallback text (drop the tags).
                        i = after_open;
                        continue;
                    }
                }
            }
        }
        // ---- closing </audio> (e.g. after a failed load) ---------------
        if markup && chars[i] == '<' && starts_with_ci(&chars, i + 1, "/audio") {
            if let Some(close) = find_char(&chars, i, '>') {
                i = close + 1;
                continue;
            }
        }
        // ---- punctuation sound icon ------------------------------------
        if let Some(idx) = table.lookup_punct(chars[i], base_dir) {
            flush!();
            pieces.push(Piece::Icon(idx));
            i += 1;
            continue;
        }
        cur.push(chars[i]);
        i += 1;
    }
    flush!();
    pieces
}

/// Parse an opening `<audio …>` tag starting at `start` (`<`).  Returns
/// `(src, index_after_tag, self_closing)` or `None` if malformed / no `src`.
fn parse_audio_open(chars: &[char], start: usize) -> Option<(String, usize, bool)> {
    let close = find_char(chars, start, '>')?;
    let tag: String = chars[start..=close].iter().collect();
    let inner = tag.trim_start_matches('<').trim_end_matches('>');
    let self_closing = inner.trim_end().ends_with('/');
    let src = get_attr(inner, "src")?;
    if src.is_empty() {
        return None;
    }
    Some((src, close + 1, self_closing))
}

/// Skip from `from` to just past the matching `</tag>`; returns `chars.len()` if
/// no close is found.
fn skip_to_close(chars: &[char], from: usize, tag: &str) -> usize {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '<' && starts_with_ci(chars, i + 1, &format!("/{tag}")) {
            if let Some(close) = find_char(chars, i, '>') {
                return close + 1;
            }
        }
        i += 1;
    }
    chars.len()
}

/// Case-insensitive check that `chars[at..]` begins with `pat`.
fn starts_with_ci(chars: &[char], at: usize, pat: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    if at + p.len() > chars.len() {
        return false;
    }
    chars[at..at + p.len()]
        .iter()
        .zip(&p)
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn find_char(chars: &[char], from: usize, target: char) -> Option<usize> {
    (from..chars.len()).find(|&i| chars[i] == target)
}

/// Extract `key="value"` / `key='value'` from a tag's inner text.
fn get_attr(inner: &str, key: &str) -> Option<String> {
    let bytes: Vec<char> = inner.chars().collect();
    let key_l = key.to_ascii_lowercase();
    let mut i = 0;
    while i + key_l.len() < bytes.len() {
        // Match `key` as a whole word followed by optional space then `=`.
        if starts_with_ci(&bytes, i, &key_l)
            && (i == 0 || bytes[i - 1].is_whitespace())
        {
            let mut j = i + key_l.len();
            while j < bytes.len() && bytes[j].is_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == '=' {
                j += 1;
                while j < bytes.len() && bytes[j].is_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && (bytes[j] == '"' || bytes[j] == '\'') {
                    let quote = bytes[j];
                    j += 1;
                    let mut val = String::new();
                    while j < bytes.len() && bytes[j] != quote {
                        val.push(bytes[j]);
                        j += 1;
                    }
                    return Some(val);
                }
            }
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// WAV decoding (robust: any bit depth / channels / sample rate)
// ---------------------------------------------------------------------------

/// Decode a WAV byte stream to mono 16-bit PCM at `target_rate` Hz.
///
/// Supports PCM (8/16/24/32-bit) and IEEE-float (32/64-bit), any channel count
/// (down-mixed to mono), and any source rate (linearly resampled).
pub fn decode_wav_mono16(bytes: &[u8], target_rate: u32) -> Result<Vec<i16>> {
    let err = |m: &str| Error::InvalidData(format!("soundicon WAV: {m}"));
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(err("not a RIFF/WAVE file"));
    }

    let mut fmt: Option<(u16, u16, u32, u16)> = None; // (format, channels, rate, bits)
    let mut data: Option<&[u8]> = None;

    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let bs = pos + 8;
        let be = (bs + size).min(bytes.len());
        match id {
            b"fmt " if size >= 16 => {
                let mut af = u16::from_le_bytes(bytes[bs..bs + 2].try_into().unwrap());
                let ch = u16::from_le_bytes(bytes[bs + 2..bs + 4].try_into().unwrap());
                let rate = u32::from_le_bytes(bytes[bs + 4..bs + 8].try_into().unwrap());
                let bits = u16::from_le_bytes(bytes[bs + 14..bs + 16].try_into().unwrap());
                // WAVE_FORMAT_EXTENSIBLE: real format tag is the first 2 bytes of
                // the SubFormat GUID at offset 24 of the fmt body.
                if af == 0xFFFE && size >= 26 {
                    af = u16::from_le_bytes(bytes[bs + 24..bs + 26].try_into().unwrap());
                }
                fmt = Some((af, ch, rate, bits));
            }
            b"data" => data = Some(&bytes[bs..be]),
            _ => {}
        }
        pos = bs + size + (size & 1); // chunks are word-aligned
    }

    let (af, channels, rate, bits) = fmt.ok_or_else(|| err("no fmt chunk"))?;
    let data = data.ok_or_else(|| err("no data chunk"))?;
    let channels = channels.max(1) as usize;
    let rate = rate.max(1);

    // Decode to interleaved f32 in [-1, 1].
    let interleaved: Vec<f32> = match (af, bits) {
        (1, 8) => data.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
        (1, 16) => data
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
            .collect(),
        (1, 24) => data
            .chunks_exact(3)
            .map(|c| {
                // little-endian signed 24-bit → sign-extend into i32
                let v = ((c[0] as i32) | ((c[1] as i32) << 8) | ((c[2] as i32) << 16)) << 8 >> 8;
                v as f32 / 8_388_608.0
            })
            .collect(),
        (1, 32) => data
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2_147_483_648.0)
            .collect(),
        (3, 32) => data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        (3, 64) => data
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) as f32)
            .collect(),
        _ => return Err(err(&format!("unsupported format tag {af} / {bits}-bit"))),
    };

    // Down-mix to mono.
    let mono: Vec<f32> = if channels <= 1 {
        interleaved
    } else {
        interleaved
            .chunks(channels)
            .map(|fr| fr.iter().sum::<f32>() / fr.len() as f32)
            .collect()
    };

    // Resample to the target rate.
    let resampled = resample_linear(&mono, rate, target_rate);

    Ok(resampled
        .iter()
        .map(|&v| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16)
        .collect())
}

/// Linear resampler from `src_rate` to `dst_rate`.
fn resample_linear(input: &[f32], src_rate: u32, dst_rate: u32) -> Vec<f32> {
    if src_rate == dst_rate || input.is_empty() {
        return input.to_vec();
    }
    let ratio = dst_rate as f64 / src_rate as f64;
    let out_len = ((input.len() as f64) * ratio).round().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 / ratio;
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - idx as f64) as f32;
        let a = input.get(idx).copied().unwrap_or(0.0);
        let b = input.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal PCM WAV from mono i16 samples at `rate`.
    fn make_wav(samples: &[i16], rate: u32, channels: u16) -> Vec<u8> {
        let bits = 16u16;
        let block_align = channels * bits / 8;
        let byte_rate = rate * block_align as u32;
        let data_len = samples.len() * 2;
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        w.extend_from_slice(b"WAVE");
        w.extend_from_slice(b"fmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes()); // PCM
        w.extend_from_slice(&channels.to_le_bytes());
        w.extend_from_slice(&rate.to_le_bytes());
        w.extend_from_slice(&byte_rate.to_le_bytes());
        w.extend_from_slice(&block_align.to_le_bytes());
        w.extend_from_slice(&bits.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data_len as u32).to_le_bytes());
        for &s in samples {
            w.extend_from_slice(&s.to_le_bytes());
        }
        w
    }

    #[test]
    fn decode_16bit_mono_same_rate() {
        let samples: Vec<i16> = (0..100).map(|i| (i * 100) as i16).collect();
        let wav = make_wav(&samples, 22_050, 1);
        let out = decode_wav_mono16(&wav, 22_050).unwrap();
        assert_eq!(out, samples);
    }

    #[test]
    fn decode_downmixes_stereo() {
        // L=1000, R=3000 → mono 2000.
        let stereo: Vec<i16> = vec![1000, 3000, 1000, 3000];
        let wav = make_wav(&stereo, 22_050, 2);
        let out = decode_wav_mono16(&wav, 22_050).unwrap();
        assert_eq!(out.len(), 2);
        assert!((out[0] - 2000).abs() <= 1, "downmix {}", out[0]);
    }

    #[test]
    fn decode_resamples_up() {
        let samples: Vec<i16> = vec![0; 100];
        let wav = make_wav(&samples, 11_025, 1);
        let out = decode_wav_mono16(&wav, 22_050).unwrap();
        // 11025 → 22050 doubles the sample count (± rounding).
        assert!((out.len() as i32 - 200).abs() <= 2, "len {}", out.len());
    }

    #[test]
    fn table_register_and_lookup() {
        let dir = std::env::temp_dir();
        let path = dir.join("espeak_rs_test_icon.wav");
        std::fs::write(&path, make_wav(&vec![500i16; 50], 22_050, 1)).unwrap();

        let mut table = SoundIconTable::new(22_050);
        let idx = table.register_punct('!', &path, &dir).unwrap();
        assert_eq!(table.lookup_punct('!', &dir), Some(idx));
        assert_eq!(table.lookup_punct('?', &dir), None);
        assert_eq!(table.samples(idx).len(), 50);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn split_punctuation_icon() {
        let dir = std::env::temp_dir();
        let path = dir.join("espeak_rs_test_bell.wav");
        std::fs::write(&path, make_wav(&vec![300i16; 20], 22_050, 1)).unwrap();

        let mut table = SoundIconTable::new(22_050);
        let idx = table.register_punct('#', &path, &dir).unwrap();

        let pieces = split_pieces("a#b", &mut table, &dir, false);
        assert_eq!(
            pieces,
            vec![Piece::Text("a".into()), Piece::Icon(idx), Piece::Text("b".into())]
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn split_ssml_audio() {
        let dir = std::env::temp_dir();
        let path = dir.join("espeak_rs_test_audio.wav");
        std::fs::write(&path, make_wav(&vec![100i16; 30], 22_050, 1)).unwrap();

        let mut table = SoundIconTable::new(22_050);
        let src = path.to_string_lossy().to_string();
        let text = format!("before <audio src=\"{src}\">fallback</audio> after");
        let pieces = split_pieces(&text, &mut table, &dir, true);

        assert_eq!(pieces.len(), 3);
        assert!(matches!(&pieces[0], Piece::Text(t) if t.contains("before")));
        assert!(matches!(pieces[1], Piece::Icon(_)));
        assert!(matches!(&pieces[2], Piece::Text(t) if t.contains("after")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn split_ssml_audio_missing_keeps_fallback() {
        let dir = std::env::temp_dir();
        let mut table = SoundIconTable::new(22_050);
        let text = "x <audio src=\"/nonexistent/nope.wav\">fallback text</audio> y";
        let pieces = split_pieces(text, &mut table, &dir, true);
        // No icon; the fallback text survives.
        assert!(pieces.iter().all(|p| matches!(p, Piece::Text(_))));
        let joined: String = pieces
            .iter()
            .map(|p| match p {
                Piece::Text(t) => t.clone(),
                Piece::Icon(_) => String::new(),
            })
            .collect();
        assert!(joined.contains("fallback text"), "joined={joined:?}");
        assert!(!joined.contains("<audio"), "tags leaked: {joined:?}");
    }

    #[test]
    fn get_attr_variants() {
        assert_eq!(get_attr("audio src=\"a.wav\"", "src"), Some("a.wav".into()));
        assert_eq!(get_attr("audio src='b.wav' foo='1'", "src"), Some("b.wav".into()));
        assert_eq!(get_attr("audio  src = \"c.wav\"", "src"), Some("c.wav".into()));
        assert_eq!(get_attr("audio data-src='x'", "src"), None);
    }

    #[test]
    fn scale_icon_reduces_amplitude() {
        let out = scale_icon_samples(&[10000, -10000]);
        // ≈ 0.916× (arithmetic shift floors negatives, so ±1 asymmetry is OK).
        assert!((out[0] - 9160).abs() < 200, "scaled {}", out[0]);
        assert!((out[1] + out[0]).abs() <= 1, "asymmetric: {} vs {}", out[0], out[1]);
    }
}
