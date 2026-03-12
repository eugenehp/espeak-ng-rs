//! eSpeak NG dictionary data for `ne_dict`, embedded at compile time.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use std::path::Path;
//!
//! let data_dir = Path::new("/tmp/espeak-data");
//! espeak_ng_data_dict_ne::install(data_dir).unwrap();
//! ```

include!(concat!(env!("OUT_DIR"), "/files.rs"));

/// Install the embedded `ne_dict` into `dest_dir`.
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
