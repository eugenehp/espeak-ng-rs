# Publishing to crates.io

This workspace contains four crates. Because the main library has optional
dependencies on the data crates, the data crates must be published first.

## Publish order

```bash
# 1. Core phoneme tables, language definitions, voice definitions (~374 KB)
cargo publish -p espeak-ng-data-phonemes

# 2. Russian language dictionary (~4.5 MB)
cargo publish -p espeak-ng-data-dict-ru

# 3. All other language dictionaries — 113 languages (~3.8 MB)
cargo publish -p espeak-ng-data-dicts

# 4. Main library (after data crates are live on crates.io, remove the
#    [patch.crates-io] section from the root Cargo.toml)
cargo publish -p espeak-ng-rs
```

## Before publishing the main crate

1. Remove the `[patch.crates-io]` section from the root `Cargo.toml`.
2. Confirm the `version` numbers in `[dependencies]` for the data crates
   match the versions just published.
3. Run `cargo package -p espeak-ng-rs` to verify the package is valid.

## Crate sizes (compressed `.crate` files)

| Crate | Files | Compressed |
|-------|-------|------------|
| `espeak-ng-data-phonemes` | 406 | 374 KB |
| `espeak-ng-data-dict-ru`  | 6 | 4.5 MB |
| `espeak-ng-data-dicts`    | 118 | 3.8 MB |

All three data crates are well under the crates.io 10 MB limit.

## Downstream usage

After publication, users add to their `Cargo.toml`:

```toml
[dependencies]
# Core library (text → IPA + synthesis)
espeak-ng-rs = "0.1"

# Optional: embed all data in the binary — no system eSpeak NG needed
espeak-ng-rs = { version = "0.1", features = ["bundled-data"] }
```

Or, for the split approach (pick only the languages you need):

```toml
[dependencies]
espeak-ng-rs           = "0.1"
espeak-ng-data-phonemes = "0.1"   # required for any language
espeak-ng-data-dicts    = "0.1"   # 113 language dicts
# espeak-ng-data-dict-ru = "0.1"  # add Russian if needed
```

Then in code:

```rust
use std::path::PathBuf;

let data_dir = PathBuf::from("/tmp/my-espeak");
std::fs::create_dir_all(&data_dir)?;

// Install the embedded data files once at startup:
espeak_ng_data_phonemes::install(&data_dir)?;
espeak_ng_data_dicts::install(&data_dir)?;

// Or use the convenience wrapper (requires bundled-data feature):
espeak_ng::install_bundled_data(&data_dir)?;

let engine = espeak_ng::EspeakNg::with_data_dir("en", &data_dir)?;
let ipa = engine.text_to_phonemes("hello world")?;
```
