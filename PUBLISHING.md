# Publishing to crates.io

This workspace contains the main library, the aggregate data crates, and
optional per-language dictionary crates named `espeak-ng-data-dict-<lang>`.
Because the main library has optional dependencies on the aggregate data
crates, those aggregate crates must be published before `espeak-ng-rs`.

## Publish order

```bash
# 1. Core phoneme tables, language definitions, voice definitions (~374 KB)
cargo publish -p espeak-ng-data-phonemes

# 2. Russian language dictionary (~4.5 MB)
cargo publish -p espeak-ng-data-dict-ru

# 3. Optional: publish any language-specific dictionary crates you want to expose
#    individually, for example:
# cargo publish -p espeak-ng-data-dict-en
# cargo publish -p espeak-ng-data-dict-uk

# 4. Aggregate crate for all non-Russian language dictionaries (~3.8 MB)
cargo publish -p espeak-ng-data-dicts

# 5. Main library (after data crates are live on crates.io, remove the
#    [patch.crates-io] section from the root Cargo.toml)
cargo publish -p espeak-ng-rs
```

## Automated publish script

Use the helper script to publish all workspace crates in order:

```bash
# Preview the publish order and commands only
python3 scripts/publish_all_crates.py

# Execute publishing for all crates (fails on espeak-ng-rs if [patch.crates-io] is still present)
python3 scripts/publish_all_crates.py --execute

# Execute only data crates (skip main crate)
python3 scripts/publish_all_crates.py --execute --no-main

# Execute with cargo --dry-run for every crate
python3 scripts/publish_all_crates.py --execute --dry-run

# Local dry-run when working tree is not committed
python3 scripts/publish_all_crates.py --execute --dry-run --allow-dirty
```

## Before publishing the main crate

1. Remove the `[patch.crates-io]` section from the root `Cargo.toml`.
2. Confirm the `version` numbers in `[dependencies]` for the data crates
   match the versions just published.
3. Run `cargo package -p espeak-ng-rs` to verify the package is valid.

## Aggregate crate sizes (compressed `.crate` files)

| Crate | Files | Compressed |
|-------|-------|------------|
| `espeak-ng-data-phonemes` | 406 | 374 KB |
| `espeak-ng-data-dict-ru`  | 6 | 4.5 MB |
| `espeak-ng-data-dicts`    | 118 | 3.8 MB |

All aggregate data crates are well under the crates.io 10 MB limit.

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
espeak-ng-data-dict-en  = "0.1"
espeak-ng-data-dict-uk  = "0.1"
# espeak-ng-data-dict-ru = "0.1"  # add Russian if needed
```

Or enable the generated selective features on the main crate:

```toml
[dependencies]
espeak-ng-rs = { version = "0.1", features = ["bundled-data-en", "bundled-data-uk"] }
```

Then in code:

```rust
use std::path::PathBuf;

let data_dir = PathBuf::from("/tmp/my-espeak");
std::fs::create_dir_all(&data_dir)?;

// Install the embedded data files once at startup:
espeak_ng_data_phonemes::install(&data_dir)?;
espeak_ng_data_dict_en::install(&data_dir)?;
espeak_ng_data_dict_uk::install(&data_dir)?;

// Or use selective helpers from the main crate:
espeak_ng::install_bundled_languages(&data_dir, &["en", "uk"])?;

// Or use the convenience wrapper (requires bundled-data feature):
espeak_ng::install_bundled_data(&data_dir)?;

let engine = espeak_ng::EspeakNg::with_data_dir("en", &data_dir)?;
let ipa = engine.text_to_phonemes("hello world")?;
```

Per-language crates are generated from `espeak-ng-data/*_dict` by
`scripts/generate_dict_crates.py`.
