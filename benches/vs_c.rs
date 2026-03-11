// benches/vs_c.rs
//
// Benchmarks comparing the Rust implementation against the C espeak-ng library.
//
// Usage
// -----
//   # Pure Rust benches only (no espeak-ng needed):
//   cargo bench
//
//   # + C CLI baseline using system espeak-ng:
//   cargo bench                          # auto-detected if on PATH
//
//   # + C CLI baseline using bundled espeak-ng (downloaded + built automatically):
//   cargo bench --features bundled-espeak
//
//   # + C FFI baseline (no subprocess overhead):
//   cargo bench --features c-oracle
//
//   # + Both:
//   cargo bench --features bundled-espeak,c-oracle
//
//   # Specific group only:
//   cargo bench -- encoding
//   cargo bench -- resonator
//   cargo bench --features bundled-espeak -- "text_to_ipa/c_cli"

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Binary resolution
//
// Priority:
//   1. ESPEAK_NG_BIN  – set by build.rs when bundled-espeak feature is active
//   2. "espeak-ng"    – system binary on PATH
//
// ESPEAK_NG_DATA is used to tell the binary where to find its data files when
// running the bundled build (which isn't installed to a system prefix).
// ---------------------------------------------------------------------------

/// Path to the espeak-ng binary to use for CLI benchmarks.
///
/// Returns the bundled binary path when built with `--features bundled-espeak`,
/// otherwise falls back to `"espeak-ng"` (system PATH).
fn espeak_binary() -> &'static str {
    // option_env! is evaluated at compile time by build.rs.
    option_env!("ESPEAK_NG_BIN").unwrap_or("espeak-ng")
}

/// Optional data directory for the bundled build.
///
/// When `Some`, passed to the subprocess as the `ESPEAK_DATA_PATH` env var.
fn espeak_data() -> Option<&'static str> {
    option_env!("ESPEAK_NG_DATA")
}

/// Check whether the configured espeak-ng binary is reachable.
fn espeak_available() -> bool {
    let mut cmd = std::process::Command::new(espeak_binary());
    cmd.arg("--version");
    if let Some(data) = espeak_data() {
        cmd.env("ESPEAK_DATA_PATH", data);
    }
    cmd.output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run the espeak-ng CLI and return the IPA string.
fn c_cli_text_to_ipa(lang: &str, text: &str) -> String {
    let mut cmd = std::process::Command::new(espeak_binary());
    cmd.args(["-v", lang, "-q", "--ipa", "--", text]);
    if let Some(data) = espeak_data() {
        cmd.env("ESPEAK_DATA_PATH", data);
    }
    let output = cmd.output().expect("espeak-ng must be available");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn text_bytes(s: &str) -> u64 {
    s.len() as u64
}

// ---------------------------------------------------------------------------
// Group 1: UTF-8 / text decoding  (pure Rust – always runs)
// ---------------------------------------------------------------------------

fn bench_encoding(c: &mut Criterion) {
    use espeak_ng::encoding::{decode_utf8_to_string, TextDecoder};

    let inputs: &[(&str, &str)] = &[
        ("ascii_short",  "Hello world"),
        ("ascii_long",   "The quick brown fox jumps over the lazy dog. \
                          Pack my box with five dozen liquor jugs."),
        ("latin_accents","Héllo wörld, cömplex áccentüation tëst"),
        ("cyrillic",     "Привет мир, это тест кириллического декодирования"),
        ("japanese",     "日本語のテキストデコードのベンチマーク"),
        ("mixed",        "Hello мир 日本語 مرحبا Héllo"),
        ("emoji",        "Hello 😀🌍🎵 world 🦀"),
        ("long_utf8",    &"café résumé naïve über fiancée".repeat(100)),
    ];

    let mut group = c.benchmark_group("encoding/utf8_decode");
    group.measurement_time(Duration::from_secs(5));

    for (name, input) in inputs {
        group.throughput(Throughput::Bytes(text_bytes(input)));

        group.bench_with_input(
            BenchmarkId::new("decode_to_string", name),
            input,
            |b, &text| b.iter(|| black_box(decode_utf8_to_string(black_box(text.as_bytes())))),
        );

        group.bench_with_input(
            BenchmarkId::new("collect_codepoints", name),
            input,
            |b, &text| {
                b.iter(|| {
                    let cps: Vec<u32> = TextDecoder::utf8(black_box(text.as_bytes())).collect();
                    black_box(cps)
                })
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Group 2: Encoding name lookup  (pure Rust – always runs)
// ---------------------------------------------------------------------------

fn bench_encoding_name_lookup(c: &mut Criterion) {
    use espeak_ng::encoding::Encoding;

    let names = [
        "UTF-8", "ISO-8859-1", "ISO-8859-15", "KOI8-R",
        "US-ASCII", "ISO-10646-UCS-2", "bogus", "",
    ];

    let mut group = c.benchmark_group("encoding/name_lookup");
    for name in &names {
        group.bench_with_input(
            BenchmarkId::new("rust", name),
            name,
            |b, &name| b.iter(|| black_box(Encoding::from_name(black_box(name)))),
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Group 3: Resonator DSP math  (pure Rust – always runs)
// ---------------------------------------------------------------------------

fn bench_resonator(c: &mut Criterion) {
    use espeak_ng::synthesize::Resonator;

    let mut group = c.benchmark_group("synthesize/resonator");
    group.measurement_time(Duration::from_secs(5));

    // Typical F1 formant filter at 22050 Hz
    let baseline = Resonator { a: 0.014, b: 1.940, c: -0.957, x1: 0.0, x2: 0.0 };

    group.bench_function("tick_single", |b| {
        let mut r = baseline.clone();
        b.iter(|| black_box(r.tick(black_box(1.0_f64))))
    });

    let samples64 = vec![1.0_f64; 64];
    group.bench_function("tick_64_samples", |b| {
        b.iter(|| {
            let mut r = baseline.clone();
            let out: Vec<f64> = samples64.iter().map(|&s| r.tick(black_box(s))).collect();
            black_box(out)
        })
    });

    let one_second = vec![0.5_f64; 22050];
    group.throughput(Throughput::Elements(22050));
    group.bench_function("tick_one_second_22050hz", |b| {
        b.iter(|| {
            let mut r = baseline.clone();
            let out: Vec<f64> = one_second.iter().map(|&s| r.tick(s)).collect();
            black_box(out)
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 4: Rust text_to_ipa stub  (pure Rust – always runs)
//
// During the stub phase this measures error-path overhead.
// Once the translate module is implemented these show real pipeline numbers.
// ---------------------------------------------------------------------------

fn bench_rust_text_to_ipa(c: &mut Criterion) {
    let inputs: &[(&str, &str, &str)] = &[
        ("en", "word",      "hello"),
        ("en", "sentence",  "The quick brown fox jumps over the lazy dog."),
        ("en", "paragraph", "Once upon a time, in a land far away, there lived \
                             a small dragon who could not breathe fire."),
        ("fr", "word",      "bonjour"),
        ("de", "word",      "Guten Tag"),
        ("es", "word",      "hola"),
    ];

    let mut group = c.benchmark_group("text_to_ipa/rust");
    group.measurement_time(Duration::from_secs(3));

    for (lang, size, text) in inputs {
        let id = format!("{lang}/{size}");
        group.throughput(Throughput::Bytes(text_bytes(text)));
        group.bench_with_input(
            BenchmarkId::new("rust", &id),
            &(*lang, *text),
            |b, &(lang, text)| {
                b.iter(|| black_box(espeak_ng::text_to_ipa(black_box(lang), black_box(text))))
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Group 5: C CLI throughput
//
// Uses the binary from:
//   - bundled-espeak feature → compiled from source by build.rs (ESPEAK_NG_BIN)
//   - otherwise              → system espeak-ng on PATH
//
// Skipped automatically if neither is available.
// ---------------------------------------------------------------------------

fn bench_c_cli(c: &mut Criterion) {
    if !espeak_available() {
        let bin = espeak_binary();
        eprintln!(
            "\n[SKIP] bench_c_cli: binary {bin:?} not available.\n\
             To enable C baseline benchmarks:\n\
             \x20 Install espeak-ng:            sudo apt install espeak-ng\n\
             \x20 Or use the bundled build:      cargo bench --features bundled-espeak\n"
        );
        return;
    }

    let inputs: &[(&str, &str, &str)] = &[
        ("en", "word",      "hello"),
        ("en", "sentence",  "The quick brown fox jumps over the lazy dog."),
        ("en", "paragraph", "Once upon a time, in a land far away, there lived \
                             a small dragon who could not breathe fire."),
        ("fr", "word",      "bonjour"),
        ("de", "word",      "Guten Tag"),
        ("es", "word",      "hola"),
        ("ru", "word",      "привет"),
        ("zh", "word",      "你好"),
    ];

    let mut group = c.benchmark_group("text_to_ipa/c_cli");
    group.measurement_time(Duration::from_secs(8));
    group.sample_size(15);
    group.warm_up_time(Duration::from_secs(2));

    for (lang, size, text) in inputs {
        let id = format!("{lang}/{size}");
        group.throughput(Throughput::Bytes(text_bytes(text)));
        group.bench_with_input(
            BenchmarkId::new("c", &id),
            &(*lang, *text),
            |b, &(lang, text)| {
                b.iter(|| black_box(c_cli_text_to_ipa(black_box(lang), black_box(text))))
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Group 6: First-phoneme latency
// ---------------------------------------------------------------------------

fn bench_latency(c: &mut Criterion) {
    if !espeak_available() {
        eprintln!("\n[SKIP] bench_latency: see bench_c_cli message above.\n");
        return;
    }

    let phrases: &[(&str, &str)] = &[
        ("en/hi",          "Hi"),
        ("en/hello",       "hello"),
        ("en/hello_world", "Hello world"),
        ("fr/bonjour",     "Bonjour"),
        ("de/guten",       "Guten Morgen"),
    ];

    let mut group = c.benchmark_group("latency/first_phoneme");
    group.measurement_time(Duration::from_secs(6));
    group.sample_size(15);
    group.warm_up_time(Duration::from_secs(2));

    for (id, text) in phrases {
        group.bench_with_input(
            BenchmarkId::new("c_cli", id),
            text,
            |b, &text| b.iter(|| black_box(c_cli_text_to_ipa("en", black_box(text)))),
        );
        group.bench_with_input(
            BenchmarkId::new("rust", id),
            text,
            |b, &text| b.iter(|| black_box(espeak_ng::text_to_ipa("en", black_box(text)))),
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Group 7: FFI oracle  (requires --features c-oracle)
// ---------------------------------------------------------------------------

#[cfg(feature = "c-oracle")]
fn bench_ffi(c: &mut Criterion) {
    use espeak_ng::oracle;

    let inputs: &[(&str, &str, &str)] = &[
        ("en", "word",     "hello"),
        ("en", "sentence", "The quick brown fox jumps over the lazy dog."),
        ("fr", "word",     "bonjour"),
        ("de", "word",     "Guten Tag"),
        ("es", "word",     "hola"),
    ];

    let mut group = c.benchmark_group("text_to_ipa/ffi_vs_rust");
    group.measurement_time(Duration::from_secs(8));

    for (lang, size, text) in inputs {
        let id = format!("{lang}/{size}");
        group.throughput(Throughput::Bytes(text_bytes(text)));

        group.bench_with_input(
            BenchmarkId::new("c_ffi", &id),
            &(*lang, *text),
            |b, &(lang, text)| {
                b.iter(|| black_box(oracle::text_to_ipa(black_box(lang), black_box(text))))
            },
        );
        group.bench_with_input(
            BenchmarkId::new("rust", &id),
            &(*lang, *text),
            |b, &(lang, text)| {
                b.iter(|| black_box(espeak_ng::text_to_ipa(black_box(lang), black_box(text))))
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion entry points
// ---------------------------------------------------------------------------

#[cfg(not(feature = "c-oracle"))]
criterion_group!(
    benches,
    bench_encoding,
    bench_encoding_name_lookup,
    bench_resonator,
    bench_rust_text_to_ipa,
    bench_c_cli,
    bench_latency,
);

#[cfg(feature = "c-oracle")]
criterion_group!(
    benches,
    bench_encoding,
    bench_encoding_name_lookup,
    bench_resonator,
    bench_rust_text_to_ipa,
    bench_c_cli,
    bench_latency,
    bench_ffi,
);

criterion_main!(benches);
