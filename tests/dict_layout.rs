// tests/dict_layout.rs
//
// Upstream #2261 moves the compiled dictionaries into a `dicts/` subdirectory.
// Both layouts have to work, or adopting that change in a data refresh breaks
// every voice at once.

use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn data_dir() -> Option<PathBuf> {
    let d = PathBuf::from("espeak-ng-data");
    d.join("en_dict").exists().then_some(d)
}

#[test]
fn dictionaries_load_from_a_dicts_subdirectory() {
    let Some(src) = data_dir() else {
        eprintln!("[SKIP] no data");
        return;
    };
    let dir = scratch("dict_layout");
    std::fs::create_dir_all(dir.join("dicts")).unwrap();
    // Everything but the `*_dict` files stays put; those move into `dicts/`.
    for entry in std::fs::read_dir(&src).unwrap() {
        let p = entry.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        if name.ends_with("_dict") {
            std::fs::copy(&p, dir.join("dicts").join(&name)).unwrap();
        } else if p.is_file() {
            std::fs::copy(&p, dir.join(&name)).unwrap();
        }
    }
    // `lang/` is a directory and is needed for voice lookup.
    let _ = std::process::Command::new("cp")
        .arg("-R").arg(src.join("lang")).arg(dir.join("lang"))
        .status();

    assert!(!dir.join("en_dict").exists(), "the flat dictionary must be gone");
    assert!(dir.join("dicts").join("en_dict").exists());

    let e = espeak_ng::EspeakNg::with_data_dir("en", &dir).expect("engine");
    let (pcm, _) = e.synth("hello world").expect("synth");
    assert!(!pcm.is_empty(), "no audio from the dicts/ layout");
    assert_eq!(
        e.text_to_phonemes("hello world").unwrap(),
        espeak_ng::EspeakNg::with_data_dir("en", &src)
            .unwrap()
            .text_to_phonemes("hello world")
            .unwrap(),
        "the two layouts must give the same phonemes"
    );
}

/// The flat layout still wins when a dictionary exists in both places, so a
/// half-migrated directory behaves predictably.
#[test]
fn the_flat_layout_takes_priority() {
    let Some(src) = data_dir() else { return };
    let dir = scratch("dict_layout_both");
    std::fs::create_dir_all(dir.join("dicts")).unwrap();
    std::fs::copy(src.join("en_dict"), dir.join("en_dict")).unwrap();
    // A deliberately corrupt copy in `dicts/` must be ignored.
    std::fs::write(dir.join("dicts").join("en_dict"), b"not a dictionary").unwrap();

    let path = espeak_ng::translate::dict_path(&dir, "en").expect("resolved");
    assert_eq!(path, dir.join("en_dict"));
}
