// tests/mbrola_cli.rs
//
// End-to-end tests for the MBROLA path: `--compile-mbrola` (the
// `compilembrola.c` phoneme-translation compiler) and `--pho` (the
// `MbrolaTranslate` .pho writer).
//
// The `mbrola` binary and the diphone databases are separately licensed and are
// not shipped, so the *audio* half can't run here; everything up to the `.pho`
// text can, and does.

use std::path::{Path, PathBuf};
use std::process::Command;

use espeak_ng::mbrola::{string_to_word, MbrolaTable};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn data_dir() -> PathBuf {
    std::env::var("ESPEAK_DATA_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("espeak-ng-data"))
}

/// Verbatim rows from upstream `phsource/mbrola/en1`, plus a `volume` directive
/// and a comment, to exercise every form the source format has.
const SOURCE: &str = "\
volume 20
// a comment line
0  oU NULL   0  @U
0  a# NULL   0  @
0  aI@ NULL  60 aI  @
2  l   oU    0  L
";

#[test]
fn compile_mbrola_then_parse() {
    let dir = scratch("mbrola_cli_compile");
    let src = dir.join("en1");
    std::fs::write(&src, SOURCE).unwrap();

    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let out = Command::new(bin)
        .arg(format!("--compile-mbrola={}", src.display()))
        .arg("--path")
        .arg(&dir)
        .output()
        .expect("run --compile-mbrola");
    assert!(
        out.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The output file is named after the source file, as in C.
    let phtrans = dir.join("mbrola_ph").join("en1_phtrans");
    assert!(phtrans.exists(), "en1_phtrans was not written");

    let table = MbrolaTable::parse(&std::fs::read(&phtrans).unwrap()).expect("parse");
    assert_eq!(table.volume, 20);
    assert_eq!(table.entries.len(), 4, "comment and directive are not rows");
    assert_eq!(table.entries[0].name, string_to_word("oU"));
    assert_eq!(table.entries[0].mbr_name, string_to_word("@U"));
    // The split row keeps both names and the percentage.
    assert_eq!(table.entries[2].percent, 60);
    assert_eq!(table.entries[2].mbr_name2, string_to_word("@"));
    // `control` bit 1 = match against the *previous* phoneme.
    assert_eq!(table.entries[3].control, 2);

    // The compiled rows agree with the first rows of the shipped table.
    let shipped = MbrolaTable::load(&data_dir(), "en1").expect("shipped en1_phtrans");
    assert_eq!(shipped.entries[0], table.entries[0]);
    assert_eq!(shipped.entries[1], table.entries[1]);
}

#[test]
fn compile_mbrola_missing_source_errors() {
    let dir = scratch("mbrola_cli_missing");
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let out = Command::new(bin)
        .arg("--compile-mbrola=/nonexistent/en1")
        .arg("--path")
        .arg(&dir)
        .output()
        .expect("run");
    assert!(!out.status.success(), "missing source should fail");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("not found"), "clear error expected: {err}");
    assert!(!dir.join("mbrola_ph").exists(), "nothing written on failure");
}

/// `--pho` writes one line per phoneme: `<name>\t<ms>` with, for a vowel, a tab
/// and space-separated `<position%> <Hz>` pitch targets.
#[test]
fn pho_output_is_well_formed() {
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let out = Command::new(bin)
        .args(["-v", "mb-en1", "--pho", "hello world"])
        .env("ESPEAK_DATA_PATH", data_dir())
        .output()
        .expect("run --pho");
    assert!(
        out.status.success(),
        "--pho failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let pho = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = pho.lines().collect();
    assert!(lines.len() >= 8, "too few phonemes: {pho}");

    for line in &lines {
        let mut fields = line.split('\t');
        let name = fields.next().unwrap();
        assert!(!name.is_empty(), "empty phoneme name in {line:?}");
        let ms: u32 = fields
            .next()
            .unwrap_or_else(|| panic!("no duration in {line:?}"))
            .parse()
            .unwrap_or_else(|_| panic!("duration is not a number in {line:?}"));
        assert!(ms > 0, "zero-length phoneme in {line:?}");

        if let Some(pitch) = fields.next() {
            let nums: Vec<i32> = pitch
                .split_whitespace()
                .map(|t| t.parse().unwrap_or_else(|_| panic!("bad pitch in {line:?}")))
                .collect();
            assert_eq!(nums.len() % 2, 0, "unpaired pitch target in {line:?}");
            for pair in nums.chunks(2) {
                assert!((0..=100).contains(&pair[0]), "position out of range in {line:?}");
                assert!((50..400).contains(&pair[1]), "implausible F0 in {line:?}");
            }
            // MBROLA requires the positions to be non-decreasing.
            let pos: Vec<i32> = nums.iter().step_by(2).copied().collect();
            assert!(pos.windows(2).all(|w| w[0] <= w[1]), "positions not sorted in {line:?}");
        }
    }

    // The clause ends with silence, so MBROLA doesn't clip the last phoneme.
    assert!(lines.last().unwrap().starts_with('_'), "no trailing pause: {pho}");
}

/// An `mb-*` voice maps eSpeak's phoneme names through that database's
/// translation table; the plain voice keeps eSpeak's own names.
#[test]
fn mbrola_voice_translates_phoneme_names() {
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let run = |voice: &str| -> String {
        let out = Command::new(bin)
            .args(["-v", voice, "--pho", "hello"])
            .env("ESPEAK_DATA_PATH", data_dir())
            .output()
            .expect("run --pho");
        assert!(out.status.success(), "{voice}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    let names = |pho: &str| -> Vec<String> {
        pho.lines().map(|l| l.split('\t').next().unwrap().to_string()).collect()
    };

    let plain = names(&run("en"));
    let mapped = names(&run("mb-en1"));
    assert_eq!(plain.len(), mapped.len(), "same phoneme count either way");
    // en1's table renames the /oʊ/ of "hello" from eSpeak's `oU` to MBROLA `@U`.
    assert!(plain.contains(&"oU".to_string()), "expected eSpeak's own name: {plain:?}");
    assert!(mapped.contains(&"@U".to_string()), "expected the mapped name: {mapped:?}");
}

/// An `mb-*` voice still speaks when the (unshipped) MBROLA binary and diphone
/// database are absent: the CLI warns and falls back to the built-in
/// synthesizer on the voice file's own language.
#[test]
fn mbrola_voice_falls_back_to_the_builtin_synthesizer() {
    let dir = scratch("mbrola_cli_speak");
    let wav = dir.join("out.wav");
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let out = Command::new(bin)
        .args(["-v", "mb-en1", "-w"])
        .arg(&wav)
        .arg("hello")
        .env("ESPEAK_DATA_PATH", data_dir())
        .output()
        .expect("run -v mb-en1");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("MBROLA"), "expected a clear warning, got: {err}");
    assert!(wav.exists(), "no audio written: {err}");
    assert!(std::fs::metadata(&wav).unwrap().len() > 1000, "audio is empty");
}

/// The voice file lists several `language` lines with priorities; the lowest
/// number wins (`mb-en1`: `en-uk 3`, `en-gb 3`, `en 2` → `en`).
#[test]
fn mbrola_voice_file_is_parsed() {
    let v = espeak_ng::voices::load_mbrola_voice(&data_dir(), "mb-en1").expect("mb-en1");
    assert_eq!(v.language, "en");
    assert_eq!(v.database, "en1");
    assert_eq!(v.phtrans, "en1_phtrans");

    // The bare database name and the `mb/` spelling resolve to the same voice.
    assert_eq!(espeak_ng::voices::load_mbrola_voice(&data_dir(), "en1"), Some(v));

    let de = espeak_ng::voices::load_mbrola_voice(&data_dir(), "mb-de4").expect("mb-de4");
    assert_eq!(de.language, "de");
    assert_eq!(de.database, "de4");

    assert_eq!(espeak_ng::voices::load_mbrola_voice(&data_dir(), "nope"), None);
}

/// The `.pho` pitch targets follow the *voice's* own `pitch <base> <range>` —
/// upstream #2501/#2516 are about MBROLA output quality, and handing every
/// database the default male pitch is the first thing that makes it wrong.
#[test]
fn pho_pitch_follows_the_voice() {
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let targets = |voice: &str, text: &str| -> Vec<i32> {
        let out = Command::new(bin)
            .args(["-v", voice, "--pho", text])
            .env("ESPEAK_DATA_PATH", data_dir())
            .output()
            .expect("run --pho");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.split('\t').nth(2))
            .flat_map(|p| {
                p.split_whitespace()
                    .skip(1)
                    .step_by(2)
                    .filter_map(|h| h.parse::<i32>().ok())
                    .collect::<Vec<_>>()
            })
            .collect()
    };

    // `mb-la1` declares `pitch 82 117`; `mb-de1` declares `pitch 140 220`.
    let male = targets("mb-la1", "salve munde");
    let female = targets("mb-de1", "guten Tag");
    assert!(!male.is_empty() && !female.is_empty(), "no pitch targets emitted");

    let mean = |v: &[i32]| v.iter().sum::<i32>() as f64 / v.len() as f64;
    assert!(
        mean(&female) > mean(&male) * 1.4,
        "a voice pitched at 140 should read far higher than one at 82: {:.0} vs {:.0}",
        mean(&female),
        mean(&male)
    );
    for &hz in male.iter().chain(&female) {
        assert!((50..400).contains(&hz), "{hz} Hz is not a plausible F0");
    }
}
