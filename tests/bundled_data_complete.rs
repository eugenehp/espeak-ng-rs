//! `bundled-data` must cover every language that has a dictionary crate.
//!
//! The dictionaries are split across `espeak-ng-data-dicts` and the
//! standalone `ru` / `fo` / `ps` crates purely to stay under the 10 MB
//! crates.io limit. That split is invisible to callers, and it is easy to
//! break: adding a language crate without adding it to the aggregate, or
//! moving one out of the aggregate without adding a dependency, leaves a
//! language that silently fails to load. These tests walk the actual crate
//! set rather than a hardcoded list, so they notice either mistake.
#![cfg(feature = "bundled-data")]

use std::path::{Path, PathBuf};

fn temp_dir(prefix: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("{prefix}_{}_{}", std::process::id(), n))
}

/// Every language for which a `espeak-ng-data-dict-<lang>` crate exists.
fn dictionary_crate_languages() -> Vec<String> {
    let crates_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("data-crates");
    let Ok(entries) = std::fs::read_dir(&crates_dir) else {
        return Vec::new(); // not available when built from a published package
    };
    let mut langs: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|name| name.strip_prefix("espeak-ng-data-dict-").map(str::to_owned))
        .collect();
    langs.sort();
    langs
}

#[test]
fn bundled_data_installs_every_dictionary_crate_language() {
    let langs = dictionary_crate_languages();
    if langs.is_empty() {
        return;
    }

    let data_dir = temp_dir("espeak-ng-bundled-complete");
    std::fs::create_dir_all(&data_dir).expect("create temp data dir");
    espeak_ng::install_bundled_data(&data_dir).expect("install bundled data");

    let missing: Vec<&str> = langs
        .iter()
        .map(String::as_str)
        .filter(|lang| !data_dir.join(format!("{lang}_dict")).exists())
        .collect();

    assert!(
        missing.is_empty(),
        "bundled-data is missing {} of {} dictionaries: {missing:?}\n\
         Add them to espeak-ng-data-dicts, or depend on their crates from the \
         bundled-data feature the way ru/fo/ps are.",
        missing.len(),
        langs.len(),
    );

    std::fs::remove_dir_all(&data_dir).ok();
}

/// A dictionary is useless without the `lang/` voice file that names it, and
/// those live in a different crate (`espeak-ng-data-phonemes`), so the two can
/// drift apart independently.
///
/// A voice file is not necessarily named after the language it serves -
/// `lang/gmq/nb` declares `dictionary no`, and `lang/iro/chr` declares the
/// tagged code `chr-US-Qaaa-x-west` - so accept the file name, the codes it
/// declares, and the prefix of each tagged code, the way eSpeak NG resolves
/// voices hierarchically.
#[test]
fn bundled_data_installs_voice_files_for_every_language() {
    let langs = dictionary_crate_languages();
    if langs.is_empty() {
        return;
    }

    let data_dir = temp_dir("espeak-ng-bundled-voices");
    std::fs::create_dir_all(&data_dir).expect("create temp data dir");
    espeak_ng::install_bundled_data(&data_dir).expect("install bundled data");

    let mut declared: Vec<String> = Vec::new();
    let mut stack = vec![data_dir.join("lang")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                declared.push(name.to_owned());
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            for line in text.lines() {
                let line = line.trim();
                for key in ["language ", "dictionary "] {
                    if let Some(rest) = line.strip_prefix(key) {
                        if let Some(code) = rest.split_whitespace().next() {
                            declared.push(code.to_owned());
                            // `chr-US-Qaaa-x-west` also serves plain `chr`.
                            if let Some((base, _)) = code.split_once('-') {
                                declared.push(base.to_owned());
                            }
                        }
                    }
                }
            }
        }
    }

    let missing: Vec<&str> = langs
        .iter()
        .map(String::as_str)
        .filter(|lang| !declared.iter().any(|d| d == lang))
        .collect();

    assert!(
        missing.is_empty(),
        "no lang/ voice file declares: {missing:?}\n\
         espeak-ng-data-phonemes is out of sync with the dictionary crates.",
    );

    std::fs::remove_dir_all(&data_dir).ok();
}

/// End-to-end proof for the languages most likely to be missed: the ones added
/// most recently, and the three published outside the aggregate.
#[test]
fn bundled_data_phonemizes_split_out_and_new_languages() {
    let data_dir = temp_dir("espeak-ng-bundled-phonemize");
    std::fs::create_dir_all(&data_dir).expect("create temp data dir");
    espeak_ng::install_bundled_data(&data_dir).expect("install bundled data");

    for lang in ["en", "ru", "fo", "ps", "ab", "crh", "kaa", "lij", "mn", "rup", "xex"] {
        let engine = espeak_ng::EspeakNg::with_data_dir(lang, &data_dir)
            .unwrap_or_else(|e| panic!("{lang}: engine init failed: {e}"));
        let ipa = engine
            .text_to_phonemes("test")
            .unwrap_or_else(|e| panic!("{lang}: phonemization failed: {e}"));
        assert!(!ipa.trim().is_empty(), "{lang}: produced no phonemes");
    }

    std::fs::remove_dir_all(&data_dir).ok();
}
