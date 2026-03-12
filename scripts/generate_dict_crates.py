#!/usr/bin/env python3

from __future__ import annotations

import shutil
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SOURCE_DATA_DIR = ROOT / "espeak-ng-data"
TARGET_ROOT = ROOT / "data-crates"
ROOT_CARGO_TOML = ROOT / "Cargo.toml"
ROOT_BUNDLED_DATA_RS = ROOT / "src" / "bundled_data_generated.rs"
BUILD_RS = """// build.rs — generates `files.rs` in OUT_DIR.
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
        let abs_expr = format!(
            r#"concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/data/{rel}\")"#
        );
        code.push_str(&format!("    ({rel:?}, include_bytes!({abs_expr})),\n"));
    }
    code.push_str("];\n");

    fs::write(format!("{out_dir}/files.rs"), code)
        .expect("could not write generated files.rs");

    println!("cargo:rerun-if-changed=data");
    println!("cargo:rerun-if-changed=build.rs");
}

fn collect_files(base: &Path, current: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<_> = fs::read_dir(current)
        .unwrap_or_else(|e| panic!("cannot read dir {}: {e}", current.display()))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();

    for path in entries {
        if path.is_dir() {
            collect_files(base, &path, out);
        } else {
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\\\', "/");
            out.push(rel);
        }
    }
}
"""

CARGO_FEATURES_BEGIN = "# BEGIN generated per-language features"
CARGO_FEATURES_END = "# END generated per-language features"
CARGO_DEPS_BEGIN = "# BEGIN generated per-language dependencies"
CARGO_DEPS_END = "# END generated per-language dependencies"


def cargo_toml(crate_name: str, dict_name: str, lang: str) -> str:
    return f"""[package]
name        = \"{crate_name}\"
version     = \"0.1.0\"
edition     = \"2021\"
description = \"eSpeak NG dictionary data for {dict_name}\"
license     = \"GPL-3.0-or-later\"
keywords    = [\"espeak\", \"tts\", \"{lang}\", \"dictionary\", \"data\"]
categories  = [\"multimedia::audio\", \"accessibility\"]

include = [
    \"build.rs\",
    \"src/**\",
    \"data/{dict_name}\",
]
"""


def lib_rs(crate_name: str, dict_name: str) -> str:
    return f"""//! eSpeak NG dictionary data for `{dict_name}`, embedded at compile time.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use std::path::Path;
//!
//! let data_dir = Path::new(\"/tmp/espeak-data\");
//! {crate_name.replace('-', '_')}::install(data_dir).unwrap();
//! ```

include!(concat!(env!(\"OUT_DIR\"), \"/files.rs\"));

/// Install the embedded `{dict_name}` into `dest_dir`.
pub fn install(dest_dir: &std::path::Path) -> std::io::Result<()> {{
    for (rel_path, data) in ALL_FILES {{
        let dest = dest_dir.join(rel_path);
        if let Some(parent) = dest.parent() {{
            std::fs::create_dir_all(parent)?;
        }}
        std::fs::write(dest, data)?;
    }}
    Ok(())
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn all_files_nonempty() {{
        assert!(!ALL_FILES.is_empty(), \"ALL_FILES must not be empty\");
        for (path, data) in ALL_FILES {{
            assert!(!data.is_empty(), \"embedded file {{path:?}} is empty\");
        }}
    }}
}}
"""


def replace_between(text: str, begin: str, end: str, body: str) -> str:
    start = text.index(begin) + len(begin)
    finish = text.index(end)
    return text[:start] + "\n" + body.rstrip() + "\n" + text[finish:]


def root_dependency_name(lang: str) -> str:
    if lang == "ru":
        return "espeak-ng-data-dict-ru"
    return f"espeak-ng-data-dict-{lang}"


def feature_name(lang: str) -> str:
    return f"bundled-data-{lang}"


def generate_root_feature_block(langs: list[str]) -> str:
    lines = [
        f'{feature_name(lang)} = ["dep:espeak-ng-data-phonemes", "dep:{root_dependency_name(lang)}"]'
        for lang in langs
    ]
    return "\n".join(lines)


def generate_root_dependency_block(langs: list[str]) -> str:
    lines = []
    for lang in langs:
        if lang == "ru":
            continue
        dep_name = root_dependency_name(lang)
        lines.append(
            f'{dep_name} = {{ version = "0.1.0", path = "data-crates/{dep_name}", optional = true }}'
        )
    return "\n".join(lines)


def generate_bundled_data_rs(langs: list[str]) -> str:
    active_cfg = ", ".join(f'feature = "{feature_name(lang)}"' for lang in langs)

    constant_entries = []
    for lang in langs:
        constant_entries.append(f'    #[cfg(feature = "{feature_name(lang)}")]')
        constant_entries.append(f'    "{lang}",')

    match_arms = []
    for lang in langs:
        crate_mod = root_dependency_name(lang).replace('-', '_')
        match_arms.append(f'        #[cfg(feature = "{feature_name(lang)}")]')
        match_arms.append(f'        "{lang}" => {crate_mod}::install(dest_dir),')

    return f'''// This file is generated by scripts/generate_dict_crates.py.
// Do not edit manually.

use std::io;
use std::path::Path;

pub const BUNDLED_LANGUAGES: &[&str] = &[
{chr(10).join(constant_entries)}
];

pub fn bundled_languages() -> &'static [&'static str] {{
    BUNDLED_LANGUAGES
}}

pub fn has_bundled_language(lang: &str) -> bool {{
    BUNDLED_LANGUAGES.contains(&lang)
}}

fn unsupported_language_error(lang: &str) -> io::Error {{
    let available = if BUNDLED_LANGUAGES.is_empty() {{
        "none".to_string()
    }} else {{
        BUNDLED_LANGUAGES.join(", ")
    }};
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("language {{lang:?}} is not bundled in this build; enabled bundled languages: {{available}}"),
    )
}}

#[allow(dead_code)]
#[cfg(any({active_cfg}))]
fn install_selected_dictionary(dest_dir: &Path, lang: &str) -> io::Result<()> {{
    match lang {{
{chr(10).join(match_arms)}
        _ => Err(unsupported_language_error(lang)),
    }}
}}

#[allow(dead_code)]
#[cfg(not(any({active_cfg})))]
fn install_selected_dictionary(_dest_dir: &Path, lang: &str) -> io::Result<()> {{
    Err(unsupported_language_error(lang))
}}

#[cfg(any({active_cfg}))]
pub fn install_bundled_language(dest_dir: &Path, lang: &str) -> io::Result<()> {{
    espeak_ng_data_phonemes::install(dest_dir)?;
    install_selected_dictionary(dest_dir, lang)
}}

#[cfg(not(any({active_cfg})))]
pub fn install_bundled_language(_dest_dir: &Path, lang: &str) -> io::Result<()> {{
    Err(unsupported_language_error(lang))
}}

#[cfg(any({active_cfg}))]
pub fn install_bundled_languages(dest_dir: &Path, languages: &[&str]) -> io::Result<()> {{
    espeak_ng_data_phonemes::install(dest_dir)?;
    for &lang in languages {{
        install_selected_dictionary(dest_dir, lang)?;
    }}
    Ok(())
}}

#[cfg(not(any({active_cfg})))]
pub fn install_bundled_languages(_dest_dir: &Path, languages: &[&str]) -> io::Result<()> {{
    if let Some(lang) = languages.first() {{
        Err(unsupported_language_error(lang))
    }} else {{
        Ok(())
    }}
}}
'''


def update_root_files(langs: list[str]) -> None:
    cargo_text = ROOT_CARGO_TOML.read_text(encoding="utf-8")
    cargo_text = replace_between(
        cargo_text,
        CARGO_FEATURES_BEGIN,
        CARGO_FEATURES_END,
        generate_root_feature_block(langs),
    )
    cargo_text = replace_between(
        cargo_text,
        CARGO_DEPS_BEGIN,
        CARGO_DEPS_END,
        generate_root_dependency_block(langs),
    )
    ROOT_CARGO_TOML.write_text(cargo_text, encoding="utf-8")

    ROOT_BUNDLED_DATA_RS.write_text(generate_bundled_data_rs(langs), encoding="utf-8")


def main() -> None:
    dict_files = sorted(SOURCE_DATA_DIR.glob("*_dict"))
    if not dict_files:
        raise SystemExit("no *_dict files found under espeak-ng-data")

    langs = [dict_path.name.removesuffix("_dict") for dict_path in dict_files]
    update_root_files(langs)

    generated = 0
    updated = 0

    for dict_path in dict_files:
        lang = dict_path.name.removesuffix("_dict")
        if lang == "ru":
            continue

        crate_name = f"espeak-ng-data-dict-{lang}"
        crate_dir = TARGET_ROOT / crate_name
        data_dir = crate_dir / "data"
        src_dir = crate_dir / "src"
        crate_dir.mkdir(parents=True, exist_ok=True)
        data_dir.mkdir(parents=True, exist_ok=True)
        src_dir.mkdir(parents=True, exist_ok=True)

        files = {
            crate_dir / "Cargo.toml": cargo_toml(crate_name, dict_path.name, lang),
            crate_dir / "build.rs": BUILD_RS,
            src_dir / "lib.rs": lib_rs(crate_name, dict_path.name),
        }

        created_here = False
        for path, content in files.items():
            encoded = content.encode("utf-8")
            if not path.exists() or path.read_bytes() != encoded:
                path.write_bytes(encoded)
                created_here = True

        target_dict_path = data_dir / dict_path.name
        source_bytes = dict_path.read_bytes()
        if not target_dict_path.exists() or target_dict_path.read_bytes() != source_bytes:
            shutil.copyfile(dict_path, target_dict_path)
            created_here = True

        if created_here:
            if (crate_dir / "Cargo.toml").exists() and (crate_dir / "src" / "lib.rs").exists():
                updated += 1
            else:
                generated += 1

    total = len(dict_files) - 1
    print(f"generated or updated {updated + generated} per-language crates ({total} non-Russian dictionaries)")


if __name__ == "__main__":
    main()