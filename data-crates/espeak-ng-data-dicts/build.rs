// build.rs — generates `files.rs` in OUT_DIR.
//
// The generated file contains:
//   pub static ALL_FILES: &[(&str, &[u8])] = &[…];
//
// Every file under data/ is embedded with include_bytes! so the crate is
// fully self-contained and works without any installed data at runtime.

use std::fs;
use std::path::Path;

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let data_dir = Path::new("data");

    // Collect all file paths relative to data/
    let mut entries: Vec<String> = Vec::new();
    collect_files(data_dir, data_dir, &mut entries);
    entries.sort();

    // Generate the static array
    let mut code = String::new();
    code.push_str("/// All data files embedded in this crate.\n");
    code.push_str("///\n");
    code.push_str("/// Each entry is `(relative_path, file_bytes)`.\n");
    code.push_str("pub static ALL_FILES: &[(&str, &[u8])] = &[\n");
    for rel in &entries {
        // Use concat!(env!("CARGO_MANIFEST_DIR"), "/data/<rel>") so the
        // path is absolute and correct even when included from OUT_DIR.
        let abs_expr = format!(
            r#"concat!(env!("CARGO_MANIFEST_DIR"), "/data/{rel}")"#
        );
        code.push_str(&format!("    ({rel:?}, include_bytes!({abs_expr})),\n"));
    }
    code.push_str("];\n");

    fs::write(format!("{out_dir}/files.rs"), code)
        .expect("could not write generated files.rs");

    // Re-run if any data file changes
    println!("cargo:rerun-if-changed=data");
    println!("cargo:rerun-if-changed=build.rs");
}

fn collect_files(base: &Path, current: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<_> = fs::read_dir(current)
        .unwrap_or_else(|e| panic!("cannot read dir {}: {e}", current.display()))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort(); // deterministic order

    for path in entries {
        if path.is_dir() {
            collect_files(base, &path, out);
        } else {
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                // Use forward slashes on all platforms
                .replace('\\', "/");
            out.push(rel);
        }
    }
}
