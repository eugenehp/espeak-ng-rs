//! MBROLA interface (`synth_mbrola.c`, `mbrowrap.c`, `compilembrola.c`).
//!
//! MBROLA is a separate diphone-concatenation synthesizer.  eSpeak translates
//! text to phonemes + durations + pitch, maps each eSpeak phoneme onto the
//! diphone database's own phoneme names, writes the result in MBROLA's `.pho`
//! format, and pipes that to the external `mbrola` binary.
//!
//! What is ported here:
//!
//!   * [`MbrolaTable::compile`] — `compilembrola.c`: the `phsource/mbrola/<db>`
//!     text format → the binary `mbrola_ph/<db>_phtrans` table.
//!   * [`MbrolaTable::parse`] / [`MbrolaTable::to_bytes`] — that binary format.
//!   * [`MbrolaTable::mbr_name`] — `GetMbrName`: the context-sensitive name
//!     lookup, including all seven `control` bits.
//!   * [`write_pitch`] — `WritePitch`: pitch targets across a phoneme, taken
//!     from the intonation envelope.
//!   * [`MbrolaTable::translate`] — `MbrolaTranslate`: a phoneme list → `.pho`.
//!   * [`write_pho`], [`find_database`], [`mbrola_database`], [`synthesize`] —
//!     the handoff to the external binary.
//!
//! Neither the `mbrola` binary nor the (large, separately licensed) diphone
//! databases ship with this port, so the **audio** path can't be exercised
//! locally; [`synthesize`] returns [`Error::MbrolaNotFound`] when the binary is
//! absent.  Everything up to the `.pho` text is testable, and is checked
//! against the shipped `*_phtrans` tables byte-for-byte.  See GAPS §1.4.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{Error, Result};
use crate::phoneme::{PH_LIQUID, PH_NASAL, PH_PAUSE, PH_STOP, PH_VOWEL};
use crate::synthesize::envelopes::ENVELOPE_DATA;
use crate::synthesize::PcmBuffer;

/// Pack up to 4 ASCII characters into a word, first character in the low byte
/// (`StringToWord` in `common.c`).  This is how phoneme mnemonics are compared.
pub fn string_to_word(s: &str) -> u32 {
    let mut word = 0u32;
    for (ix, c) in s.bytes().take(4).enumerate() {
        word |= (c as u32) << (ix * 8);
    }
    word
}

/// Unpack a packed mnemonic (`WordToString`).
pub fn word_to_string(mut word: u32) -> String {
    let mut s = String::with_capacity(4);
    for _ in 0..4 {
        let c = (word & 0xff) as u8;
        if c == 0 {
            break;
        }
        s.push(c as char);
        word >>= 8;
    }
    s
}

// ── `control` bits of an MBROLA translation entry ────────────────────────────
/// Skip the phoneme after this one.
pub const MBR_SKIP_NEXT: u32 = 0x01;
/// Match `next_phoneme` against the *previous* phoneme instead of the next.
pub const MBR_MATCH_PREV: u32 = 0x02;
/// Only at the start of a word.
pub const MBR_WORD_START: u32 = 0x04;
/// Don't match two phonemes across a word boundary.
pub const MBR_NO_CROSS_WORD: u32 = 0x08;
/// Use this phoneme's name as a prefix to the *next* one (de4's `?` prefix).
pub const MBR_NAME_PREFIX: u32 = 0x10;
/// Only in a stressed syllable.
pub const MBR_STRESSED_ONLY: u32 = 0x20;
/// Only at the end of a word.
pub const MBR_WORD_END: u32 = 0x40;

/// `next_phoneme` sentinel: match any vowel (`VWL` in the source format).
const NEXT_ANY_VOWEL: u32 = 2;

/// One row of an MBROLA phoneme-name translation table (`MBROLA_TAB`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MbrolaEntry {
    /// eSpeak phoneme mnemonic, packed.
    pub name: u32,
    /// Context phoneme this row requires: 0 = none, 2 = any vowel, `':'` = the
    /// phoneme carries a length modifier, otherwise a packed mnemonic.
    pub next_phoneme: u32,
    /// MBROLA phoneme name to emit, packed.
    pub mbr_name: u32,
    /// Optional second MBROLA name, for a phoneme that splits in two.
    pub mbr_name2: u32,
    /// Percentage of the duration given to the first of the two names.
    pub percent: i32,
    /// `MBR_*` control bits.
    pub control: u32,
}

/// An MBROLA phoneme-name translation table (`<db>_phtrans`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MbrolaTable {
    /// Output volume in 1/16ths (the source file's `volume` directive).
    pub volume: u32,
    pub entries: Vec<MbrolaEntry>,
}

impl MbrolaTable {
    /// Compile a `phsource/mbrola/<db>` source file (`compilembrola.c`).
    ///
    /// Each data line is `<control> <phoneme> <next|NULL|VWL> <percent> <name1>
    /// [<name2>]`; `//` starts a comment and `volume <n>` sets the output
    /// volume.  Lines with fewer than 5 fields are ignored, as in C.
    pub fn compile(source: &str) -> Self {
        let mut table = MbrolaTable { volume: 20, entries: Vec::new() };

        for raw in source.lines() {
            // C reads at most 39 bytes per line (`fgets(buf, sizeof(phoneme))`),
            // so a longer line is truncated rather than wrapped.
            let line = &raw[..raw.len().min(39)];
            let line = match line.find("//") {
                Some(ix) => &line[..ix],
                None => line,
            };
            if let Some(rest) = line.strip_prefix("volume") {
                table.volume = atoi(rest) as u32;
                continue;
            }

            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 5 {
                continue;
            }
            let control = match f[0].parse::<i32>() {
                Ok(c) => c as u32,
                Err(_) => continue,
            };
            let percent = match f[3].parse::<i32>() {
                Ok(p) => p,
                Err(_) => continue,
            };
            table.entries.push(MbrolaEntry {
                name: string_to_word(f[1]),
                next_phoneme: match f[2] {
                    "NULL" => 0,
                    "VWL" => NEXT_ANY_VOWEL,
                    other => string_to_word(other),
                },
                mbr_name: if f[4] == "NULL" { 0 } else { string_to_word(f[4]) },
                mbr_name2: f.get(5).map(|s| string_to_word(s)).unwrap_or(0),
                percent,
                control,
            });
        }
        table
    }

    /// Parse a binary `<db>_phtrans`: a 4-byte volume, then 24-byte records
    /// (all little-endian), terminated by a record whose `name` is 0.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 4 {
            return Err(Error::InvalidData("mbrola phtrans: file too short".into()));
        }
        let rd = |o: usize| -> u32 {
            u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
        };
        let mut table = MbrolaTable { volume: rd(0), entries: Vec::new() };
        let mut o = 4;
        while o + 24 <= bytes.len() {
            let name = rd(o);
            if name == 0 {
                break; // list terminator
            }
            table.entries.push(MbrolaEntry {
                name,
                next_phoneme: rd(o + 4),
                mbr_name: rd(o + 8),
                mbr_name2: rd(o + 12),
                percent: rd(o + 16) as i32,
                control: rd(o + 20),
            });
            o += 24;
        }
        Ok(table)
    }

    /// Serialize to the binary `<db>_phtrans` format.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + (self.entries.len() + 1) * 24);
        out.extend_from_slice(&self.volume.to_le_bytes());
        for e in &self.entries {
            for w in [e.name, e.next_phoneme, e.mbr_name, e.mbr_name2, e.percent as u32, e.control]
            {
                out.extend_from_slice(&w.to_le_bytes());
            }
        }
        out.extend_from_slice(&[0u8; 24]); // zeroed terminator record
        out
    }

    /// Load `<data_dir>/mbrola_ph/<db>_phtrans`.
    pub fn load(data_dir: &Path, db: &str) -> Result<Self> {
        let path = data_dir.join("mbrola_ph").join(format!("{db}_phtrans"));
        let bytes = std::fs::read(&path).map_err(|_| Error::MbrolaVoiceNotFound)?;
        Self::parse(&bytes)
    }

    /// `GetMbrName` — translate one eSpeak phoneme to its MBROLA name(s) in
    /// context.  Returns `None` when the table maps the phoneme to nothing (the
    /// phoneme is then dropped from the `.pho`).
    ///
    /// `prefix` carries the pending [`MBR_NAME_PREFIX`] name across calls; it is
    /// consumed (and cleared) by the next phoneme.
    pub fn mbr_name(&self, ctx: &MbrContext, prefix: &mut u32) -> MbrName {
        let mut mnem = string_to_word(&ctx.mnemonic);
        let mut out = MbrName { name: mnem, name2: 0, split: 0, control: 0 };

        for pr in &self.entries {
            if mnem != pr.name {
                continue;
            }
            let mut found = if pr.next_phoneme == 0 {
                true
            } else if pr.next_phoneme == u32::from(b':') && ctx.lengthen {
                true
            } else {
                let other = if pr.control & MBR_MATCH_PREV != 0 {
                    (string_to_word(&ctx.prev_mnemonic), ctx.prev_type)
                } else if pr.control & MBR_NO_CROSS_WORD != 0 && ctx.next_newword {
                    // Don't match the next phoneme over a word boundary.
                    (string_to_word("_"), PH_PAUSE)
                } else {
                    (string_to_word(&ctx.next_mnemonic), ctx.next_type)
                };
                pr.next_phoneme == other.0
                    || (pr.next_phoneme == NEXT_ANY_VOWEL && other.1 == PH_VOWEL)
                    || (pr.next_phoneme == u32::from(b'_') && other.1 == PH_PAUSE)
            };

            if pr.control & MBR_WORD_START != 0 && !ctx.newword {
                found = false;
            }
            if pr.control & MBR_WORD_END != 0 && !ctx.next_newword {
                found = false;
            }
            if pr.control & MBR_STRESSED_ONLY != 0 && ctx.stress_level < ctx.word_stress {
                found = false;
            }

            if found {
                out.name2 = pr.mbr_name2;
                out.split = pr.percent;
                out.control = pr.control;

                if pr.control & MBR_NAME_PREFIX != 0 {
                    *prefix = pr.mbr_name;
                    out.name = 0;
                    return out;
                }
                mnem = pr.mbr_name;
                break;
            }
        }

        if *prefix != 0 {
            mnem = (mnem << 8) | (*prefix & 0xff);
        }
        *prefix = 0;
        out.name = mnem;
        out
    }

    /// `MbrolaTranslate` — a phoneme list to `.pho` lines.
    ///
    /// Durations come from the caller (the port computes them in its own length
    /// pipeline, where C reaches back into `DoSample3`/`DoSpect2`); this applies
    /// the MBROLA-specific adjustments on top: a vowel's `std_length` scaled by
    /// its length modifier, +50 ms before a pause for vowels/nasals/liquids, and
    /// the split of a two-name phoneme by `percent`.
    pub fn translate(&self, phonemes: &[MbrPhoneme]) -> Vec<PhoLine> {
        let mut out = Vec::new();
        let mut prefix = 0u32;
        let mut skip_next = false;

        for (ix, p) in phonemes.iter().enumerate() {
            if skip_next {
                skip_next = false;
                continue;
            }
            let next = phonemes.get(ix + 1);
            let prev = ix.checked_sub(1).and_then(|i| phonemes.get(i));
            let ctx = MbrContext {
                mnemonic: p.mnemonic.clone(),
                prev_mnemonic: prev.map(|q| q.mnemonic.clone()).unwrap_or_default(),
                next_mnemonic: next.map(|q| q.mnemonic.clone()).unwrap_or_default(),
                prev_type: prev.map(|q| q.ph_type).unwrap_or(PH_PAUSE),
                next_type: next.map(|q| q.ph_type).unwrap_or(PH_PAUSE),
                newword: p.newword,
                next_newword: next.map(|q| q.newword).unwrap_or(true),
                lengthen: p.lengthen,
                stress_level: p.stress_level,
                word_stress: p.word_stress,
            };
            let m = self.mbr_name(&ctx, &mut prefix);
            if m.control & MBR_SKIP_NEXT != 0 {
                skip_next = true;
            }
            if m.name == 0 {
                continue; // ignore this phoneme (or it was consumed as a prefix)
            }

            let mut len = p.duration_ms as i32;
            let mut name2 = m.name2;
            // A `name2` of `_` means "insert a pause after this phoneme".
            let mut pause_after = 0;
            if name2 == u32::from(b'_') {
                pause_after = m.split;
                name2 = 0;
            }

            let mut done = false;
            let mut final_pitch: Vec<(i32, i32)> = Vec::new();

            match p.ph_type {
                PH_VOWEL => {
                    let mut vlen = p.std_length as i32;
                    if p.lengthen {
                        vlen += p.lengthen_ms as i32;
                    }
                    if ctx.next_type == PH_PAUSE {
                        vlen += 50; // lengthen vowels before a pause
                    }
                    len = vlen * p.length as i32 / 256;

                    if name2 == 0 {
                        out.push(PhoLine {
                            phoneme: word_to_string(m.name),
                            duration_ms: len.max(0) as u32,
                            pitch: write_pitch(p, 0, false),
                        });
                    } else {
                        let len1 = len * m.split / 100;
                        out.push(PhoLine {
                            phoneme: word_to_string(m.name),
                            duration_ms: len1.max(0) as u32,
                            pitch: write_pitch(p, m.split, false),
                        });
                        out.push(PhoLine {
                            phoneme: word_to_string(name2),
                            duration_ms: (len - len1).max(0) as u32,
                            pitch: write_pitch(p, -m.split, false),
                        });
                    }
                    done = true;
                }
                PH_NASAL => {
                    if ctx.next_type != PH_VOWEL {
                        if ctx.next_type == PH_PAUSE {
                            len += 50;
                        }
                        final_pitch = write_pitch(p, 0, true);
                    }
                }
                PH_LIQUID => {
                    if ctx.next_type == PH_PAUSE {
                        len += 50;
                        final_pitch = write_pitch(p, 0, true);
                    }
                }
                PH_STOP if next.is_some_and(|q| q.ph_type == PH_PAUSE) => {
                    len += p.prepause_ms as i32;
                }
                _ => {}
            }

            if !done {
                if name2 != 0 {
                    let len1 = len * m.split / 100;
                    out.push(PhoLine {
                        phoneme: word_to_string(m.name),
                        duration_ms: len1.max(0) as u32,
                        pitch: Vec::new(),
                    });
                    len -= len1;
                    out.push(PhoLine {
                        phoneme: word_to_string(name2),
                        duration_ms: len.max(0) as u32,
                        pitch: final_pitch,
                    });
                } else {
                    out.push(PhoLine {
                        phoneme: word_to_string(m.name),
                        duration_ms: len.max(0) as u32,
                        pitch: final_pitch,
                    });
                }
            }

            if pause_after != 0 {
                out.push(PhoLine {
                    phoneme: "_".into(),
                    duration_ms: pause_after.max(0) as u32,
                    pitch: Vec::new(),
                });
            }
        }
        out
    }
}

/// `atoi` semantics: leading spaces, optional sign, digits, stop at anything else.
fn atoi(s: &str) -> i32 {
    let t = s.trim_start();
    let (sign, t) = match t.strip_prefix('-') {
        Some(r) => (-1, r),
        None => (1, t.strip_prefix('+').unwrap_or(t)),
    };
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    sign * digits.parse::<i32>().unwrap_or(0)
}

/// The result of an [`MbrolaTable::mbr_name`] lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MbrName {
    /// Packed MBROLA name, or 0 when the phoneme is dropped.
    pub name: u32,
    /// Packed second name when the phoneme splits in two, else 0.
    pub name2: u32,
    /// Percentage of the duration given to `name` (`percent` in the table).
    pub split: i32,
    /// The matched row's `MBR_*` control bits.
    pub control: u32,
}

/// The neighbour context an [`MbrolaTable::mbr_name`] lookup needs.
#[derive(Debug, Clone, Default)]
pub struct MbrContext {
    pub mnemonic: String,
    pub prev_mnemonic: String,
    pub next_mnemonic: String,
    pub prev_type: u8,
    pub next_type: u8,
    /// This phoneme starts a word.
    pub newword: bool,
    /// The *next* phoneme starts a word (i.e. this one ends a word).
    pub next_newword: bool,
    /// The phoneme carries a `:` length modifier.
    pub lengthen: bool,
    pub stress_level: u8,
    pub word_stress: u8,
}

/// One phoneme handed to [`MbrolaTable::translate`].
#[derive(Debug, Clone, Default)]
pub struct MbrPhoneme {
    pub mnemonic: String,
    /// `PH_*` phoneme type.
    pub ph_type: u8,
    pub stress_level: u8,
    pub word_stress: u8,
    pub newword: bool,
    pub lengthen: bool,
    /// `ph->std_length` in ms (vowels).
    pub std_length: u16,
    /// Extra length contributed by a `:` modifier, in ms.
    pub lengthen_ms: u16,
    /// The length modifier as a /256 fraction (`p->length`).
    pub length: u16,
    /// Duration in ms for non-vowels, where the caller has already computed it.
    pub duration_ms: u32,
    /// Pre-pause before a stop, in ms.
    pub prepause_ms: u16,
    /// Intonation envelope index, and the syllable's pitch span.
    pub env: u8,
    pub pitch1: i32,
    pub pitch2: i32,
    /// Voice pitch parameters, `(pitch_base, pitch_range)` as `SetPitch2`
    /// computes them — Hz << 12.
    pub voice_pitch_base: i32,
    pub voice_pitch_range: i32,
}

/// `WritePitch` — the pitch targets for one phoneme, as `(position%, Hz)`.
///
/// `split` is the two-name split percentage: 0 for a whole phoneme, positive for
/// the first half of a split vowel, negative for the second.  With `final_only`
/// the phoneme gets a single target at 100% (used for a nasal or liquid that
/// runs into a pause).
pub fn write_pitch(p: &MbrPhoneme, split: i32, final_only: bool) -> Vec<(i32, i32)> {
    // Apply the pitch change only over this proportion of the phoneme.
    const ENV100: i32 = 80;

    let env = ENVELOPE_DATA[(p.env as usize).min(ENVELOPE_DATA.len() - 1)];
    let (base, range) = set_pitch2(p.voice_pitch_base, p.voice_pitch_range, p.pitch1, p.pitch2);

    let p_end = ((env[127] as i32 * range) >> 8) + base;
    let p_end = p_end / 4096;
    if final_only {
        return vec![(100, p_end)];
    }

    let mut env_split = split * 128 / 100;
    if env_split < 0 {
        env_split = -env_split;
    }

    // Find the envelope's extremes; put the mid-phoneme pitch point on one of
    // them rather than blindly at the halfway mark.
    let (mut y_max, mut y_min) = (0usize, 0usize);
    let (mut max, mut min) = (-1i32, 999i32);
    for (x, &v) in env.iter().enumerate() {
        if i32::from(v) > max {
            max = i32::from(v);
            y_max = x;
        }
        if i32::from(v) < min {
            min = i32::from(v);
            y_min = x;
        }
    }
    let mut y = [0usize; 4];
    y[2] = 64;
    if y_max > 0 && y_max < 127 {
        y[2] = y_max;
    }
    if y_min > 0 && y_min < 127 {
        y[2] = y_min;
    }
    y[1] = y[2] / 2;
    y[3] = y[2] + (127 - y[2]) / 2;

    let mut out = Vec::new();
    if split >= 0 {
        let p1 = ((env[0] as i32 * range) >> 8) + base;
        out.push((0, p1 / 4096));
    }

    // A linear rise or fall needs no intermediate points.
    if p.env > 1 {
        for &yi in &y[1..4] {
            let p2 = ((env[yi] as i32 * range) >> 8) + base;
            let y2 = if split > 0 {
                yi as i32 * ENV100 / env_split.max(1)
            } else if split < 0 {
                (yi as i32 - env_split) * ENV100 / env_split.max(1)
            } else {
                yi as i32 * ENV100 / 128
            };
            if y2 > 0 && y2 <= ENV100 {
                out.push((y2, p2 / 4096));
            }
        }
    }

    if split <= 0 {
        out.push((ENV100, p_end));
    }
    if ENV100 < 100 {
        out.push((100, p_end));
    }
    out
}

/// `SetPitch2` — combine the voice's pitch base/range with a syllable's pitch
/// span into the `(base, range)` pair the envelope is scaled by.  Hz << 12.
pub fn set_pitch2(
    voice_pitch_base: i32,
    voice_pitch_range: i32,
    pitch1: i32,
    pitch2: i32,
) -> (i32, i32) {
    let (pitch1, pitch2) = if pitch1 > pitch2 { (pitch2, pitch1) } else { (pitch1, pitch2) };
    let base = voice_pitch_base;
    let range = voice_pitch_range;
    let pitch_base = base + pitch1 * range / 2;
    let pitch_range = base + pitch2 * range / 2 - pitch_base;
    (pitch_base, pitch_range)
}

/// One line of an MBROLA `.pho` script: a phoneme name, its duration, and a
/// series of `(position%, pitch_Hz)` pitch targets across that phoneme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoLine {
    pub phoneme: String,
    pub duration_ms: u32,
    pub pitch: Vec<(i32, i32)>,
}

/// Render an MBROLA `.pho` script.
///
/// The layout matches `MbrolaTranslate`'s `sprintf`s: `<name>\t<ms>` and, when
/// there are pitch targets, a tab and then space-separated `<pos> <hz>` pairs.
pub fn write_pho(lines: &[PhoLine]) -> String {
    let mut out = String::new();
    for l in lines {
        out.push_str(&l.phoneme);
        out.push('\t');
        out.push_str(&l.duration_ms.to_string());
        if !l.pitch.is_empty() {
            out.push('\t');
            for &(pos, hz) in &l.pitch {
                out.push(' ');
                out.push_str(&pos.to_string());
                out.push(' ');
                out.push_str(&hz.to_string());
            }
        }
        out.push('\n');
    }
    out
}

/// Resolve an eSpeak MBROLA voice reference to its diphone-database name:
/// `mb-en1` / `mb/en1` → `en1`; a non-MBROLA voice → `None`.
pub fn mbrola_database(voice: &str) -> Option<&str> {
    voice.strip_prefix("mb-").or_else(|| voice.strip_prefix("mb/"))
}

/// Locate an MBROLA diphone database directory under `data_dir`, returning
/// [`Error::MbrolaVoiceNotFound`] if none is present.
pub fn find_database(data_dir: &Path, db: &str) -> Result<PathBuf> {
    for cand in [
        data_dir.join("mbrola").join(db),
        data_dir.join("mbrola").join(db).join(db),
        data_dir.join("mbrola").join("voices").join(db),
        data_dir.join("voices").join("mb").join(db),
    ] {
        if cand.exists() {
            return Ok(cand);
        }
    }
    Err(Error::MbrolaVoiceNotFound)
}

/// Synthesize a `.pho` script through the external `mbrola` binary against the
/// diphone `database`, returning 16-bit little-endian PCM.
///
/// Returns [`Error::MbrolaNotFound`] if the binary cannot be launched.
pub fn synthesize(pho: &str, database: &Path) -> Result<PcmBuffer> {
    use std::io::Write;
    // `mbrola <database> - -.raw` → reads .pho from stdin, writes raw PCM to stdout.
    let mut child = Command::new("mbrola")
        .arg(database)
        .arg("-")
        .arg("-.raw")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::MbrolaNotFound)?;

    child
        .stdin
        .take()
        .ok_or(Error::MbrolaNotFound)?
        .write_all(pho.as_bytes())
        .map_err(Error::Io)?;

    let out = child.wait_with_output().map_err(Error::Io)?;
    let pcm = out
        .stdout
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    Ok(pcm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phoneme::PH_FRICATIVE;

    fn data_dir() -> PathBuf {
        std::env::var("ESPEAK_DATA_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("espeak-ng-data"))
    }

    #[test]
    fn write_pho_format() {
        // C writes `"%s\t"` then `"%d\t%s"` where the pitch string starts with
        // a space, so a line with pitch reads `name<TAB>ms<TAB> 0 120`.
        let lines = vec![
            PhoLine { phoneme: "_".into(), duration_ms: 50, pitch: vec![] },
            PhoLine { phoneme: "h".into(), duration_ms: 61, pitch: vec![(0, 120)] },
            PhoLine { phoneme: "@".into(), duration_ms: 76, pitch: vec![(0, 118), (50, 115)] },
        ];
        assert_eq!(write_pho(&lines), "_\t50\nh\t61\t 0 120\n@\t76\t 0 118 50 115\n");
    }

    #[test]
    fn database_resolution() {
        assert_eq!(mbrola_database("mb-en1"), Some("en1"));
        assert_eq!(mbrola_database("mb/de2"), Some("de2"));
        assert_eq!(mbrola_database("en"), None);
        // Absent database → typed error, never a panic.
        assert!(matches!(
            find_database(Path::new("/nonexistent"), "en1"),
            Err(Error::MbrolaVoiceNotFound)
        ));
    }

    #[test]
    fn synthesize_without_binary_is_clean() {
        // The `mbrola` binary is not shipped; the call must fail with a typed
        // error rather than hanging or panicking.
        match synthesize("_\t50\n", Path::new("/nonexistent/en1")) {
            Err(Error::MbrolaNotFound) => {}
            Ok(pcm) => assert!(pcm.is_empty(), "unexpected audio from a missing database"),
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    }

    /// `StringToWord` packs the first character into the low byte.
    #[test]
    fn mnemonics_pack_little_endian() {
        assert_eq!(string_to_word("oU"), 0x556f);
        assert_eq!(string_to_word("@"), 0x40);
        assert_eq!(string_to_word(""), 0);
        // More than 4 characters are truncated, as in C.
        assert_eq!(string_to_word("abcde"), string_to_word("abcd"));
        for m in ["oU", "@", "a#", "l/2", "aI@"] {
            assert_eq!(word_to_string(string_to_word(m)), m);
        }
    }

    /// Every shipped `<db>_phtrans` must parse and re-serialize byte-for-byte —
    /// the binary layout is 4-byte volume + 24-byte records + a zero terminator.
    #[test]
    fn every_shipped_phtrans_round_trips() {
        let dir = data_dir().join("mbrola_ph");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).expect("mbrola_ph directory") {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            let table = MbrolaTable::parse(&bytes)
                .unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
            assert_eq!(
                table.to_bytes(),
                bytes,
                "{} did not round-trip",
                path.display()
            );
            checked += 1;
        }
        assert!(checked >= 40, "expected the full set of phtrans tables, saw {checked}");
    }

    /// The `compilembrola.c` source format compiles to exactly the bytes the
    /// shipped table holds — checked against the first rows of `en1_phtrans`.
    #[test]
    fn compile_matches_the_shipped_table() {
        // Verbatim from upstream `phsource/mbrola/en1`.
        let src = "\n0  oU NULL   0  @U\n0  a# NULL   0  @\n0  @2 NULL   0  @\n";
        let compiled = MbrolaTable::compile(src);
        assert_eq!(compiled.volume, 20, "default volume is 20/16ths");
        assert_eq!(compiled.entries.len(), 3);

        let shipped = MbrolaTable::load(&data_dir(), "en1").expect("en1_phtrans");
        assert_eq!(shipped.volume, compiled.volume);
        assert_eq!(&shipped.entries[..3], &compiled.entries[..]);
        assert_eq!(shipped.entries[0].name, string_to_word("oU"));
        assert_eq!(shipped.entries[0].mbr_name, string_to_word("@U"));
    }

    /// `volume`, `//` comments, `NULL`/`VWL` sentinels, and the optional second
    /// name all follow `compilembrola.c`.
    #[test]
    fn compile_handles_every_source_form() {
        let t = MbrolaTable::compile(
            "volume 24\n\
             // a comment line\n\
             0  aI@ NULL  60 aI  @   // splits in two\n\
             2  l   oU    0  L\n\
             16 ?   VWL   0  ?\n\
             not enough fields\n",
        );
        assert_eq!(t.volume, 24);
        assert_eq!(t.entries.len(), 3);

        let split = t.entries[0];
        assert_eq!(split.name, string_to_word("aI@"));
        assert_eq!(split.next_phoneme, 0, "NULL means no context requirement");
        assert_eq!(split.percent, 60);
        assert_eq!(split.mbr_name2, string_to_word("@"));

        assert_eq!(t.entries[1].control & MBR_MATCH_PREV, MBR_MATCH_PREV);
        assert_eq!(t.entries[1].next_phoneme, string_to_word("oU"));

        assert_eq!(t.entries[2].next_phoneme, 2, "VWL means any vowel");
        assert_eq!(t.entries[2].control & MBR_NAME_PREFIX, MBR_NAME_PREFIX);
    }

    fn ctx(mnem: &str, next: &str, next_type: u8) -> MbrContext {
        MbrContext {
            mnemonic: mnem.into(),
            next_mnemonic: next.into(),
            next_type,
            newword: true,
            next_newword: false,
            ..Default::default()
        }
    }

    /// `GetMbrName`: an unconditional row, a context-dependent row, and a
    /// phoneme absent from the table (which keeps its own name).
    #[test]
    fn name_lookup_follows_context() {
        let t = MbrolaTable::compile("0  oU NULL 0 @U\n0  l VWL 0 l\n0 l NULL 0 5\n");
        let mut prefix = 0;

        let m = t.mbr_name(&ctx("oU", "t", PH_STOP), &mut prefix);
        assert_eq!(word_to_string(m.name), "@U");

        // `l` before a vowel takes the first row; elsewhere the fallback row.
        let m = t.mbr_name(&ctx("l", "a", PH_VOWEL), &mut prefix);
        assert_eq!(word_to_string(m.name), "l");
        let m = t.mbr_name(&ctx("l", "t", PH_STOP), &mut prefix);
        assert_eq!(word_to_string(m.name), "5");

        // Not in the table → unchanged.
        let m = t.mbr_name(&ctx("z", "a", PH_VOWEL), &mut prefix);
        assert_eq!(word_to_string(m.name), "z");
    }

    /// A row with `control` bit 4 contributes a *prefix* to the next phoneme's
    /// name and emits nothing itself — de4's glottal-stop marker.
    #[test]
    fn name_prefix_bit_defers_to_the_next_phoneme() {
        let t = MbrolaTable::compile("16 ? VWL 0 ?\n");
        let mut prefix = 0;

        let m = t.mbr_name(&ctx("?", "a", PH_VOWEL), &mut prefix);
        assert_eq!(m.name, 0, "the prefix phoneme itself is dropped");
        assert_eq!(prefix, string_to_word("?"));

        let m = t.mbr_name(&ctx("a", "t", PH_STOP), &mut prefix);
        assert_eq!(word_to_string(m.name), "?a", "prefix moved onto the next name");
        assert_eq!(prefix, 0, "the prefix is consumed");
    }

    /// Bits 2 (start of word), 5 (stressed only) and 6 (end of word) gate a row.
    #[test]
    fn positional_control_bits_gate_a_row() {
        let t = MbrolaTable::compile("4 h NULL 0 H\n32 a NULL 0 A\n64 n NULL 0 N\n");
        let mut prefix = 0;

        let mut c = ctx("h", "a", PH_VOWEL);
        assert_eq!(word_to_string(t.mbr_name(&c, &mut prefix).name), "H");
        c.newword = false;
        assert_eq!(word_to_string(t.mbr_name(&c, &mut prefix).name), "h", "not word-initial");

        let mut c = ctx("a", "t", PH_STOP);
        c.stress_level = 4;
        c.word_stress = 4;
        assert_eq!(word_to_string(t.mbr_name(&c, &mut prefix).name), "A");
        c.stress_level = 1;
        assert_eq!(word_to_string(t.mbr_name(&c, &mut prefix).name), "a", "unstressed");

        let mut c = ctx("n", "t", PH_STOP);
        c.next_newword = true;
        assert_eq!(word_to_string(t.mbr_name(&c, &mut prefix).name), "N");
        c.next_newword = false;
        assert_eq!(word_to_string(t.mbr_name(&c, &mut prefix).name), "n", "not word-final");
    }

    fn vowel(mnem: &str, env: u8) -> MbrPhoneme {
        MbrPhoneme {
            mnemonic: mnem.into(),
            ph_type: PH_VOWEL,
            std_length: 160,
            length: 256,
            env,
            pitch1: 20,
            pitch2: 40,
            // `pitch 82 118` → base (82-9)<<12, range (118-82)*108.
            voice_pitch_base: (82 - 9) << 12,
            voice_pitch_range: (118 - 82) * 108,
            newword: true,
            ..Default::default()
        }
    }

    /// `WritePitch` emits a start target, up to three envelope-driven targets,
    /// and an end target — all inside the plausible F0 range for the voice.
    #[test]
    fn pitch_targets_span_the_phoneme() {
        let p = vowel("a", 3);
        let pts = write_pitch(&p, 0, false);
        assert!(pts.len() >= 3, "expected several pitch targets, got {pts:?}");
        assert_eq!(pts[0].0, 0, "first target is at the phoneme start");
        assert_eq!(pts.last().unwrap().0, 100, "last target is at the end");
        for &(pos, hz) in &pts {
            assert!((0..=100).contains(&pos), "position {pos} out of range");
            assert!((50..400).contains(&hz), "{hz} Hz is not a plausible F0");
        }
        // Positions must be non-decreasing, or MBROLA rejects the line.
        assert!(pts.windows(2).all(|w| w[0].0 <= w[1].0), "{pts:?} is not monotonic");

        // `final_only` collapses to a single end-of-phoneme target.
        let f = write_pitch(&p, 0, true);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].0, 100);
    }

    /// A linear rise/fall envelope (index 0 or 1) gets no intermediate targets.
    #[test]
    fn linear_envelopes_have_no_intermediate_targets() {
        let pts = write_pitch(&vowel("a", 1), 0, false);
        assert_eq!(pts.iter().map(|p| p.0).collect::<Vec<_>>(), vec![0, 80, 100]);
    }

    /// End to end: a phoneme list becomes a `.pho` script, with the vowel's
    /// duration taken from `std_length` and the pause carried through.
    #[test]
    fn translate_renders_a_pho_script() {
        let t = MbrolaTable::compile("0  oU NULL 0 @U\n");
        let phonemes = vec![
            MbrPhoneme {
                mnemonic: "h".into(),
                ph_type: PH_FRICATIVE,
                duration_ms: 61,
                newword: true,
                ..Default::default()
            },
            MbrPhoneme { mnemonic: "oU".into(), ..vowel("oU", 3) },
            MbrPhoneme {
                mnemonic: "_".into(),
                ph_type: PH_PAUSE,
                duration_ms: 100,
                ..Default::default()
            },
        ];
        let pho = write_pho(&t.translate(&phonemes));
        let lines: Vec<&str> = pho.lines().collect();
        assert_eq!(lines.len(), 3, "{pho}");
        assert!(lines[0].starts_with("h\t61"), "{}", lines[0]);
        // The vowel is renamed by the table and lengthened before the pause
        // (std_length 160 + 50).
        assert!(lines[1].starts_with("@U\t210\t"), "{}", lines[1]);
        assert!(lines[2].starts_with("_\t100"), "{}", lines[2]);
    }

    /// A table row with a second name splits the phoneme into two `.pho` lines
    /// whose durations follow `percent`.
    #[test]
    fn a_split_vowel_becomes_two_lines() {
        let t = MbrolaTable::compile("0  aI@ NULL 60 aI @\n");
        let phonemes = vec![MbrPhoneme { mnemonic: "aI@".into(), ..vowel("aI@", 3) }];
        let lines = t.translate(&phonemes);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].phoneme, "aI");
        assert_eq!(lines[1].phoneme, "@");
        // 60% / 40% of the vowel's 210 ms (160 std_length + 50 before the pause).
        assert_eq!(lines[0].duration_ms + lines[1].duration_ms, 210);
        assert_eq!(lines[0].duration_ms, 126);
    }
}
