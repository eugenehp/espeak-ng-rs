//! MBROLA interface (`synth_mbrola.c` / `mbrowrap.c`).
//!
//! MBROLA is a separate diphone-concatenation synthesizer.  eSpeak translates
//! text to phonemes + durations + pitch, writes them in MBROLA's `.pho` format,
//! and pipes that to the external `mbrola` binary together with a diphone
//! *database* (e.g. `en1`).  Neither the binary nor the (large, separately
//! licensed) diphone databases ship with this port, so the **audio** path can't
//! be exercised locally — but the `.pho` writer, voice/database resolution, and
//! the process handoff are implemented; the invocation returns
//! [`Error::MbrolaNotFound`] when the binary is absent.  See GAPS §1.4.
//!
//! The `.pho` writer is also the basis for a future `--pho` CLI output (§16).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{Error, Result};
use crate::synthesize::PcmBuffer;

/// One line of an MBROLA `.pho` script: a phoneme name, its duration, and a
/// series of `(position%, pitch_Hz)` pitch targets across that phoneme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoLine {
    pub phoneme: String,
    pub duration_ms: u32,
    pub pitch: Vec<(u8, u16)>,
}

/// Render an MBROLA `.pho` script: `<phoneme> <ms> [<pos> <hz>]…`, one per line.
pub fn write_pho(lines: &[PhoLine]) -> String {
    let mut out = String::new();
    for l in lines {
        out.push_str(&l.phoneme);
        out.push(' ');
        out.push_str(&l.duration_ms.to_string());
        for &(pos, hz) in &l.pitch {
            out.push(' ');
            out.push_str(&pos.to_string());
            out.push(' ');
            out.push_str(&hz.to_string());
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

    #[test]
    fn write_pho_format() {
        let lines = vec![
            PhoLine { phoneme: "_".into(), duration_ms: 50, pitch: vec![] },
            PhoLine { phoneme: "h".into(), duration_ms: 61, pitch: vec![(0, 120)] },
            PhoLine { phoneme: "@".into(), duration_ms: 76, pitch: vec![(0, 118), (50, 115)] },
        ];
        assert_eq!(write_pho(&lines), "_ 50\nh 61 0 120\n@ 76 0 118 50 115\n");
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
        // No `mbrola` on PATH → MbrolaNotFound; if it *is* installed, a bogus DB
        // path yields empty PCM.  Either way: no panic.
        let r = synthesize("_ 50\n", Path::new("/nonexistent/en1"));
        assert!(r.is_err() || r.map(|p| p.is_empty()).unwrap_or(true));
    }
}
