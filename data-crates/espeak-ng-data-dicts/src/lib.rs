//! Language dictionaries for eSpeak NG (all languages except Russian), embedded at compile time.
//!
//! Contains 113 compiled language dictionaries for eSpeak NG 1.52.0:
//! Afrikaans, Amharic, Arabic, Basque, Bengali, Bulgarian, Cantonese,
//! Catalan, Chinese (Mandarin), Croatian, Czech, Danish, Dutch, English,
//! Esperanto, Estonian, Finnish, French, Galician, German, Greek, …
//! and many more.
//!
//! The Russian dictionary is published separately as
//! [`espeak-ng-data-dict-ru`](https://crates.io/crates/espeak-ng-data-dict-ru)
//! because of its size (~8 MB).
//!
//! ## Usage
//!
//! ```rust,no_run
//! use std::path::Path;
//!
//! let data_dir = Path::new("/tmp/espeak-data");
//! espeak_ng_data_dicts::install(data_dir).unwrap();
//! ```

include!(concat!(env!("OUT_DIR"), "/files.rs"));

/// Install all embedded language dictionaries into `dest_dir`.
pub fn install(dest_dir: &std::path::Path) -> std::io::Result<()> {
    for (rel_path, data) in ALL_FILES {
        let dest = dest_dir.join(rel_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(dest, data)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_files_nonempty() {
        assert!(!ALL_FILES.is_empty(), "ALL_FILES must not be empty");
        for (path, data) in ALL_FILES {
            assert!(!data.is_empty(), "embedded file {path:?} is empty");
        }
    }
}
