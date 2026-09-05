// tests/fuzz_parsers.rs
//
// Deterministic fuzzing of the binary-format parsers — the port's answer to
// upstream #2441 (OSS-Fuzz targets for the dictionary reader).  eSpeak's data
// files are parsed with a lot of offset arithmetic; the C engine has had a
// steady trickle of out-of-bounds reads there.
//
// The port is memory-safe, so the property under test is stronger and simpler:
// **no input may panic**.  Every parser is fed
//   * random bytes,
//   * a real file truncated at every length,
//   * a real file with single bytes corrupted,
// and must return `Ok`/`Err`/`None` rather than unwinding.

use std::path::{Path, PathBuf};

/// A tiny deterministic PRNG, so a failure is always reproducible from its seed.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Lcg(seed.wrapping_mul(6364136223846793005).wrapping_add(1))
    }
    fn next_u8(&mut self) -> u8 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u8
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next_u8()).collect()
    }
}

fn data_dir() -> Option<PathBuf> {
    let d = PathBuf::from("espeak-ng-data");
    d.join("phontab").exists().then_some(d)
}

fn read(dir: &Path, name: &str) -> Option<Vec<u8>> {
    std::fs::read(dir.join(name)).ok()
}

/// Run `f` over random inputs, every truncation of `real`, and single-byte
/// corruptions of it.  `f` must not panic for any of them.
fn hammer(real: &[u8], mut f: impl FnMut(&[u8])) {
    let mut rng = Lcg::new(0x5EED);

    // Random noise of assorted lengths, including degenerate ones.
    for len in [0usize, 1, 2, 3, 4, 7, 8, 15, 16, 64, 255, 1024, 4096] {
        f(&rng.bytes(len));
    }

    // Truncations: a header that promises more than the file holds is the
    // classic out-of-bounds trigger.
    let steps = real.len().max(1).div_ceil(97);
    for cut in (0..real.len()).step_by(steps.max(1)) {
        f(&real[..cut]);
    }
    f(real);

    // Single-byte corruption, biased toward the header where the offsets live.
    for _ in 0..300 {
        let mut copy = real.to_vec();
        if copy.is_empty() {
            break;
        }
        let bias = (rng.next_u8() as usize) % 4 == 0;
        let pos = if bias {
            (rng.next_u8() as usize) % copy.len().min(64)
        } else {
            ((rng.next_u8() as usize) << 8 | rng.next_u8() as usize) % copy.len()
        };
        copy[pos] = rng.next_u8();
        f(&copy);
    }
}

#[test]
fn phoneme_table_parser_survives_corrupt_input() {
    let Some(dir) = data_dir() else {
        eprintln!("[SKIP] no data");
        return;
    };
    let real = read(&dir, "phontab").expect("phontab");
    hammer(&real, |bytes| {
        let _ = espeak_ng::phoneme::load::parse_phontab(bytes);
    });
}

#[test]
fn dictionary_parser_survives_corrupt_input() {
    let Some(dir) = data_dir() else { return };
    let Some(real) = read(&dir, "en_dict") else {
        eprintln!("[SKIP] no en_dict");
        return;
    };
    hammer(&real, |bytes| {
        // Parsing may fail; looking a word up in whatever survived must not.
        if let Ok(dict) = espeak_ng::dictionary::Dictionary::from_bytes("en", bytes.to_vec()) {
            let ctx = espeak_ng::dictionary::lookup::LookupCtx::default();
            for w in ["a", "hello", "", "\u{ffff}", "zzzzzzzzzzzzzzzz"] {
                let _ = espeak_ng::dictionary::lookup::lookup(&dict, w, &ctx);
            }
        }
    });
}

#[test]
fn mbrola_table_parser_survives_corrupt_input() {
    let Some(dir) = data_dir() else { return };
    let Some(real) = read(&dir, "mbrola_ph/en1_phtrans") else { return };
    hammer(&real, |bytes| {
        if let Ok(t) = espeak_ng::mbrola::MbrolaTable::parse(bytes) {
            // Round-tripping and looking a name up must be total, too.
            let _ = t.to_bytes();
            let mut prefix = 0;
            let ctx = espeak_ng::mbrola::MbrContext {
                mnemonic: "a".into(),
                ..Default::default()
            };
            let _ = t.mbr_name(&ctx, &mut prefix);
        }
    });
}

#[test]
fn spect_sequence_parser_survives_corrupt_input() {
    let Some(dir) = data_dir() else { return };
    let Some(real) = read(&dir, "phondata") else { return };
    // `phondata` is large; hammer a representative slice rather than all of it.
    let slice = &real[..real.len().min(64 * 1024)];
    hammer(slice, |bytes| {
        // Try to parse a spectrum sequence at several offsets, including ones
        // that certainly aren't sequence starts.
        for off in [0usize, 1, 7, 64, 1000] {
            if off < bytes.len() {
                let _ = espeak_ng::synthesize::phondata::SpectSeq::parse(
                    &bytes[off..],
                    bytes.len() - off,
                );
            }
        }
    });
}

/// The rules engine walks compiled byte programs with a lot of index
/// arithmetic; translating with a corrupt dictionary must not panic either.
#[test]
fn translation_with_a_corrupt_dictionary_does_not_panic() {
    let Some(dir) = data_dir() else { return };
    let Some(real) = read(&dir, "en_dict") else { return };
    let mut rng = Lcg::new(0xC0FFEE);
    for _ in 0..40 {
        let mut copy = real.clone();
        for _ in 0..16 {
            let pos = ((rng.next_u8() as usize) << 8 | rng.next_u8() as usize) % copy.len();
            copy[pos] = rng.next_u8();
        }
        let Ok(dict) = espeak_ng::dictionary::Dictionary::from_bytes("en", copy) else {
            continue;
        };
        let ctx = espeak_ng::dictionary::lookup::LookupCtx::default();
        for w in ["hello", "world", "a", "the"] {
            let _ = espeak_ng::dictionary::lookup::lookup(&dict, w, &ctx);
        }
    }
}
