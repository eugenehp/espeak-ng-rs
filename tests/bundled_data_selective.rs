#![cfg(any(feature = "bundled-data-en", feature = "bundled-data-uk", feature = "bundled-data-de"))]

use std::path::{Path, PathBuf};

fn temp_dir(prefix: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("{prefix}_{}_{}", std::process::id(), n))
}

fn assert_core_phoneme_files(dir: &Path) {
    for name in ["phontab", "phonindex", "phondata", "intonations"] {
        assert!(dir.join(name).exists(), "missing core phoneme data file: {name}");
    }
}

#[cfg(feature = "bundled-data-en")]
#[test]
fn install_bundled_language_en_writes_expected_files() {
    let data_dir = temp_dir("espeak-ng-selective-en");
    std::fs::create_dir_all(&data_dir).expect("create temp data dir");

    espeak_ng::install_bundled_language(&data_dir, "en").expect("install selective en bundled data");

    assert_core_phoneme_files(&data_dir);
    assert!(data_dir.join("en_dict").exists(), "expected en_dict to be installed");

    if !espeak_ng::has_bundled_language("ru") {
        assert!(!data_dir.join("ru_dict").exists(), "ru_dict should not be installed in this feature set");
    }

    std::fs::remove_dir_all(&data_dir).expect("cleanup temp data dir");
}

#[cfg(all(feature = "bundled-data-en", feature = "bundled-data-de"))]
#[test]
fn install_bundled_languages_multiple_writes_each_selected_dict() {
    let data_dir = temp_dir("espeak-ng-selective-en-de");
    std::fs::create_dir_all(&data_dir).expect("create temp data dir");

    espeak_ng::install_bundled_languages(&data_dir, &["en", "de"]).expect("install selected bundled languages");

    assert_core_phoneme_files(&data_dir);
    assert!(data_dir.join("en_dict").exists(), "expected en_dict to be installed");
    assert!(data_dir.join("de_dict").exists(), "expected de_dict to be installed");

    std::fs::remove_dir_all(&data_dir).expect("cleanup temp data dir");
}
