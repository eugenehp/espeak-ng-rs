// build.rs
//
// Handles two optional features:
//
//   c-oracle        – links the installed system libespeak-ng for FFI.
//   bundled-espeak  – downloads espeak-ng 1.52.0 source from GitHub,
//                     builds it with CMake, and emits:
//                       ESPEAK_NG_BIN   path to the compiled binary
//                       ESPEAK_NG_DATA  path to the espeak-ng-data directory
//                     so benchmarks can use them without espeak-ng on PATH.
//
// The bundled build is cached in Cargo's OUT_DIR.  Running `cargo clean`
// removes the cache and forces a full rebuild on the next run.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

// ---------------------------------------------------------------------------
// Espeak-ng release to download
// ---------------------------------------------------------------------------

const VERSION: &str = "1.52.0";
const TARBALL_URL: &str =
    "https://github.com/espeak-ng/espeak-ng/archive/refs/tags/1.52.0.tar.gz";

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    // Always re-run when build.rs itself changes.
    println!("cargo:rerun-if-changed=build.rs");

    let bundled = env::var("CARGO_FEATURE_BUNDLED_ESPEAK").is_ok();
    let c_oracle = env::var("CARGO_FEATURE_C_ORACLE").is_ok();

    if bundled {
        let install_dir = build_espeak_ng();
        emit_bundled_env(&install_dir);

        // If c-oracle is also active, link against the bundled library.
        if c_oracle {
            let lib_dir = install_dir.join("lib");
            println!("cargo:rustc-link-search=native={}", lib_dir.display());
            println!("cargo:rustc-link-lib=espeak-ng");
        }
    } else if c_oracle {
        // No bundled build – link against the system espeak-ng.
        link_system_espeak();
    }
}

// ---------------------------------------------------------------------------
// System espeak-ng linking  (c-oracle without bundled-espeak)
// ---------------------------------------------------------------------------

fn link_system_espeak() {
    let found = Command::new("pkg-config")
        .args(["--libs", "--cflags", "espeak-ng"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if found {
        let out = Command::new("pkg-config")
            .args(["--libs", "espeak-ng"])
            .output()
            .expect("pkg-config must be on PATH when c-oracle feature is enabled");
        for token in String::from_utf8(out.stdout).unwrap().split_whitespace() {
            if let Some(lib) = token.strip_prefix("-l") {
                println!("cargo:rustc-link-lib={lib}");
            } else if let Some(path) = token.strip_prefix("-L") {
                println!("cargo:rustc-link-search=native={path}");
            }
        }
    } else {
        println!("cargo:rustc-link-lib=espeak-ng");
    }
}

// ---------------------------------------------------------------------------
// Bundled build
// ---------------------------------------------------------------------------

/// Builds espeak-ng from source and returns the install prefix.
fn build_espeak_ng() -> PathBuf {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let install_dir = out_dir.join(format!("espeak-ng-{VERSION}-install"));
    let stamp = install_dir.join(".build-complete");

    if stamp.exists() {
        // Already built – nothing to do.
        eprintln!("[bundled-espeak] cached build found at {}", install_dir.display());
        return install_dir;
    }

    eprintln!("[bundled-espeak] building espeak-ng {VERSION} – this may take a minute…");

    let src_dir = download_and_extract(&out_dir);
    cmake_build(&src_dir, &out_dir, &install_dir);

    // Write stamp so subsequent builds skip straight here.
    fs::write(&stamp, VERSION).unwrap();
    eprintln!("[bundled-espeak] build complete → {}", install_dir.display());
    install_dir
}

/// Download the tarball (if needed) and extract it.  Returns the source dir.
fn download_and_extract(out_dir: &Path) -> PathBuf {
    let tarball = out_dir.join(format!("espeak-ng-{VERSION}.tar.gz"));
    let src_dir = out_dir.join(format!("espeak-ng-{VERSION}"));

    if !tarball.exists() {
        eprintln!("[bundled-espeak] downloading {TARBALL_URL}");
        download_file(TARBALL_URL, &tarball);
    }

    if !src_dir.exists() {
        eprintln!("[bundled-espeak] extracting tarball…");
        let status = Command::new("tar")
            .args(["-xzf", tarball.to_str().unwrap(), "-C", out_dir.to_str().unwrap()])
            .status()
            .expect("[bundled-espeak] `tar` must be available");
        assert!(status.success(), "[bundled-espeak] tar extraction failed");
    }

    src_dir
}

/// Download `url` to `dest` using curl (falling back to wget).
fn download_file(url: &str, dest: &Path) {
    // Try curl first (available on macOS and most Linux distros).
    let curl_ok = Command::new("curl")
        .args(["--fail", "--location", "--silent", "--show-error",
               "--output", dest.to_str().unwrap(), url])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if curl_ok {
        return;
    }

    // Fall back to wget.
    eprintln!("[bundled-espeak] curl failed or not found, trying wget…");
    let wget_ok = Command::new("wget")
        .args(["--quiet", "-O", dest.to_str().unwrap(), url])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    assert!(
        wget_ok,
        "[bundled-espeak] failed to download {url}\n\
         Neither `curl` nor `wget` succeeded.\n\
         Set ESPEAK_NG_SOURCE_DIR to a pre-extracted source directory to skip the download."
    );
}

/// Configure, build, and install espeak-ng with CMake.
fn cmake_build(src: &Path, out_dir: &Path, install: &Path) {
    let build_dir = out_dir.join(format!("espeak-ng-{VERSION}-build"));
    fs::create_dir_all(&build_dir).unwrap();

    // Detect parallelism.
    let jobs = available_parallelism();

    // --- Configure ---
    eprintln!("[bundled-espeak] cmake configure…");
    let status = Command::new("cmake")
        .args([
            "-S", src.to_str().unwrap(),
            "-B", build_dir.to_str().unwrap(),
            &format!("-DCMAKE_INSTALL_PREFIX={}", install.display()),
            "-DCMAKE_BUILD_TYPE=Release",
            // Disable optional audio / speech backends that need extra libs.
            "-DUSE_LIBPCAUDIO=OFF",
            "-DUSE_MBROLA=OFF",
            "-DUSE_LIBSONIC=OFF",
            // Keep Klatt synthesizer (it's pure C, no extra deps).
            "-DUSE_KLATT=ON",
            // Don't build tests (saves build time).
            "-DENABLE_TESTS=OFF",
        ])
        .status()
        .expect("[bundled-espeak] `cmake` must be installed (https://cmake.org)");
    assert!(status.success(), "[bundled-espeak] cmake configure failed");

    // --- Build ---
    eprintln!("[bundled-espeak] cmake build (j={jobs})…");
    let status = Command::new("cmake")
        .args([
            "--build", build_dir.to_str().unwrap(),
            "--parallel", &jobs.to_string(),
        ])
        .status()
        .expect("[bundled-espeak] cmake build failed");
    assert!(status.success(), "[bundled-espeak] cmake build failed");

    // --- Install ---
    eprintln!("[bundled-espeak] cmake install…");
    let status = Command::new("cmake")
        .args(["--install", build_dir.to_str().unwrap()])
        .status()
        .expect("[bundled-espeak] cmake install failed");
    assert!(status.success(), "[bundled-espeak] cmake install failed");
}

/// Emit `cargo:rustc-env` variables that the bench binary reads at compile time.
fn emit_bundled_env(install: &Path) {
    // Binary
    let bin = install.join("bin").join("espeak-ng");
    assert!(
        bin.exists(),
        "[bundled-espeak] expected binary at {} but it was not found after build",
        bin.display()
    );
    println!("cargo:rustc-env=ESPEAK_NG_BIN={}", bin.display());

    // Data directory – CMake installs it at one of these locations depending
    // on the platform and cmake version.
    let candidates = [
        install.join("lib").join("espeak-ng-data"),
        install.join("share").join("espeak-ng-data"),
        // Debian multiarch layout
        install.join("lib").join("x86_64-linux-gnu").join("espeak-ng-data"),
        install.join("lib").join("aarch64-linux-gnu").join("espeak-ng-data"),
    ];
    let data_dir = candidates
        .iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| {
            panic!(
                "[bundled-espeak] could not find espeak-ng-data under {}.\n\
                 Searched:\n{}",
                install.display(),
                candidates.iter()
                    .map(|p| format!("  {}", p.display()))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        });
    println!("cargo:rustc-env=ESPEAK_NG_DATA={}", data_dir.display());

    eprintln!(
        "[bundled-espeak] binary : {}\n\
         [bundled-espeak] data   : {}",
        bin.display(),
        data_dir.display()
    );
}

/// Number of logical CPUs, capped at 8 to avoid memory pressure on CI.
fn available_parallelism() -> usize {
    // std::thread::available_parallelism is stable since 1.59.
    std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4)
}
