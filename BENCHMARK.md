# Benchmark Results

Generated: 2026-03-11 01:40 UTC  
Platform: `Linux 6.12.69-linuxkit aarch64`  
Rust: `rustc 1.91.1 (ed61e7d7e 2025-11-07) (Alpine Linux Rust 1.91.1-r0)`  
eSpeak NG: `eSpeak NG text-to-speech: 1.52.0  Data at: /usr/share/espeak-ng-data`

> **Reading this file**  
> Times are wall-clock per operation (lower is better).  
> Throughput is input bytes or elements processed per second (higher is better).  
> **Rust** rows show the pure-Rust implementation; rows marked **c\_cli** call
> the `espeak-ng` binary as a subprocess (includes process-spawn overhead).  
> During the stub phase, Rust `text_to_ipa` rows measure error-path overhead
> only and will be replaced by real numbers as each module is implemented.

---

## encoding/name_lookup

| Function | Input           | Mean      | ±Std      |
| -------- | --------------- | --------- | --------- |
| rust     |                 | 3.457 ns  | ±0.484 ns |
| rust     | ISO-10646-UCS-2 | 27.180 ns | ±4.133 ns |
| rust     | ISO-8859-1      | 22.813 ns | ±0.384 ns |
| rust     | ISO-8859-15     | 27.506 ns | ±3.722 ns |
| rust     | KOI8-R          | 22.302 ns | ±4.736 ns |
| rust     | US-ASCII        | 17.191 ns | ±3.143 ns |
| rust     | UTF-8           | 18.076 ns | ±0.263 ns |
| rust     | bogus           | 19.394 ns | ±2.922 ns |

![encoding/name_lookup violin plot](benches/results/encoding_name_lookup/report/violin.svg)

## encoding/utf8_decode

| Function           | Input         | Mean       | ±Std        | Throughput  |
| ------------------ | ------------- | ---------- | ----------- | ----------- |
| collect_codepoints | ascii_long    | 444.487 ns | ±8.842 ns   | 191.23 MB/s |
| collect_codepoints | ascii_short   | 145.538 ns | ±2.066 ns   | 75.58 MB/s  |
| collect_codepoints | cyrillic      | 334.121 ns | ±4.186 ns   | 275.35 MB/s |
| collect_codepoints | emoji         | 149.451 ns | ±1.995 ns   | 194.04 MB/s |
| collect_codepoints | japanese      | 182.132 ns | ±2.480 ns   | 312.96 MB/s |
| collect_codepoints | latin_accents | 276.771 ns | ±5.526 ns   | 158.98 MB/s |
| collect_codepoints | long_utf8     | 10.124 µs  | ±305.550 ns | 355.59 MB/s |
| collect_codepoints | mixed         | 178.231 ns | ±2.866 ns   | 224.43 MB/s |
| decode_to_string   | ascii_long    | 393.295 ns | ±27.359 ns  | 216.12 MB/s |
| decode_to_string   | ascii_short   | 61.057 ns  | ±1.064 ns   | 180.16 MB/s |
| decode_to_string   | cyrillic      | 298.896 ns | ±3.985 ns   | 307.80 MB/s |
| decode_to_string   | emoji         | 129.690 ns | ±1.966 ns   | 223.61 MB/s |
| decode_to_string   | japanese      | 230.797 ns | ±2.720 ns   | 246.97 MB/s |
| decode_to_string   | latin_accents | 251.349 ns | ±7.431 ns   | 175.06 MB/s |
| decode_to_string   | long_utf8     | 9.996 µs   | ±300.659 ns | 360.15 MB/s |
| decode_to_string   | mixed         | 192.041 ns | ±3.848 ns   | 208.29 MB/s |
| rust               | ascii_long    | 308.117 ns | ±4.440 ns   | 275.87 MB/s |
| rust               | ascii_short   | 69.380 ns  | ±3.395 ns   | 158.55 MB/s |
| rust               | cyrillic      | 333.824 ns | ±9.683 ns   | 275.59 MB/s |
| rust               | emoji         | 126.940 ns | ±3.792 ns   | 228.46 MB/s |
| rust               | japanese      | 215.162 ns | ±4.580 ns   | 264.92 MB/s |
| rust               | latin_accents | 202.892 ns | ±4.824 ns   | 216.86 MB/s |
| rust               | long_utf8     | 11.999 µs  | ±1.101 µs   | 300.02 MB/s |
| rust               | mixed         | 168.322 ns | ±8.471 ns   | 237.64 MB/s |
| rust_iter          | ascii_long    | 431.171 ns | ±7.369 ns   | 197.14 MB/s |
| rust_iter          | ascii_short   | 102.034 ns | ±1.641 ns   | 107.81 MB/s |
| rust_iter          | cyrillic      | 320.650 ns | ±5.459 ns   | 286.92 MB/s |
| rust_iter          | emoji         | 183.411 ns | ±46.306 ns  | 158.11 MB/s |
| rust_iter          | japanese      | 209.330 ns | ±54.539 ns  | 272.30 MB/s |
| rust_iter          | latin_accents | 248.284 ns | ±4.837 ns   | 177.22 MB/s |
| rust_iter          | long_utf8     | 12.373 µs  | ±545.641 ns | 290.95 MB/s |
| rust_iter          | mixed         | 200.809 ns | ±19.578 ns  | 199.19 MB/s |

![encoding/utf8_decode violin plot](benches/results/encoding_utf8_decode/report/violin.svg)
![encoding/utf8_decode throughput](benches/results/encoding_utf8_decode/report/lines.svg)

## latency/first_phoneme

| Function | Input          | Mean      | ±Std        |
| -------- | -------------- | --------- | ----------- |
| c_cli    | de/guten       | 6.183 ms  | ±440.782 µs |
| c_cli    | en/hello       | 5.687 ms  | ±272.014 µs |
| c_cli    | en/hello_world | 6.115 ms  | ±420.056 µs |
| c_cli    | en/hi          | 6.414 ms  | ±1.362 ms   |
| c_cli    | fr/bonjour     | 5.656 ms  | ±338.558 µs |
| rust     | de/guten       | 14.945 ns | ±0.170 ns   |
| rust     | en/hello       | 15.100 ns | ±0.144 ns   |
| rust     | en/hello_world | 14.986 ns | ±0.340 ns   |
| rust     | en/hi          | 14.909 ns | ±0.272 ns   |
| rust     | fr/bonjour     | 14.848 ns | ±0.274 ns   |

![latency/first_phoneme violin plot](benches/results/latency_first_phoneme/report/violin.svg)

## synthesize/resonator

| Function                | Mean       | ±Std       | Throughput       |
| ----------------------- | ---------- | ---------- | ---------------- |
| tick_64_samples         | 174.663 ns | ±20.744 ns | -                |
| tick_one_second_22050hz | 61.763 µs  | ±4.568 µs  | 357011694 elem/s |
| tick_single             | 3.654 ns   | ±0.160 ns  | -                |

![synthesize/resonator violin plot](benches/results/synthesize_resonator/report/violin.svg)

## text_to_ipa/c_cli

| Function | Input        | Mean     | ±Std        | Throughput |
| -------- | ------------ | -------- | ----------- | ---------- |
| c        | de/word      | 6.715 ms | ±966.471 µs | 1.34 KB/s  |
| c        | en/paragraph | 9.304 ms | ±1.159 ms   | 9.89 KB/s  |
| c        | en/sentence  | 8.171 ms | ±1.629 ms   | 5.39 KB/s  |
| c        | en/word      | 6.917 ms | ±1.463 ms   | 0.72 KB/s  |
| c        | es/word      | 6.457 ms | ±1.576 ms   | 0.62 KB/s  |
| c        | fr/word      | 6.456 ms | ±966.300 µs | 1.08 KB/s  |
| c        | ru/word      | 9.324 ms | ±1.247 ms   | 1.29 KB/s  |
| c        | zh/word      | 6.919 ms | ±1.164 ms   | 0.87 KB/s  |

![text_to_ipa/c_cli violin plot](benches/results/text_to_ipa_c_cli/report/violin.svg)
![text_to_ipa/c_cli throughput](benches/results/text_to_ipa_c_cli/report/lines.svg)

## text_to_ipa/rust

| Function | Input        | Mean      | ±Std      | Throughput  |
| -------- | ------------ | --------- | --------- | ----------- |
| rust     | de/word      | 15.039 ns | ±0.730 ns | 598.46 MB/s |
| rust     | en/paragraph | 16.136 ns | ±0.788 ns | 5.70 GB/s   |
| rust     | en/sentence  | 15.414 ns | ±0.508 ns | 2.85 GB/s   |
| rust     | en/word      | 14.992 ns | ±0.535 ns | 333.52 MB/s |
| rust     | es/word      | 15.028 ns | ±0.228 ns | 266.17 MB/s |
| rust     | fr/word      | 14.731 ns | ±0.371 ns | 475.18 MB/s |

![text_to_ipa/rust violin plot](benches/results/text_to_ipa_rust/report/violin.svg)
![text_to_ipa/rust throughput](benches/results/text_to_ipa_rust/report/lines.svg)

---

## Notes

- Times are [criterion](https://github.com/bheisler/criterion.rs) means over
  100 samples (15 for CLI subprocess groups).
- **c\_cli** benchmarks include subprocess spawn + espeak-ng initialisation +
  data file loading on every call — this is the real-world latency a caller
  would see when shelling out to `espeak-ng`.
- The **bundled-espeak** feature (`cargo bench --features bundled-espeak`)
  downloads and compiles espeak-ng from source so the C baseline runs even
  without a system installation.
- Once the Rust `translate` module is implemented, the **rust** rows in the
  `text_to_ipa` groups will reflect actual pipeline performance.
- Charts are Criterion's SVG output copied into `benches/results/` so they
  render directly in GitHub without needing `target/` to be checked in.

## Re-running

```bash
# Using system espeak-ng (must be on PATH)
./benches/bench.sh

# Building espeak-ng from source automatically
./benches/bench.sh --bundled

# Only encoding benchmarks
./benches/bench.sh --filter encoding

# Parse existing results without re-running
./benches/bench.sh --no-run
```
