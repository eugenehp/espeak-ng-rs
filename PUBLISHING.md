# Publishing to crates.io

This workspace contains the main library, the aggregate data crates, and
optional per-language dictionary crates named `espeak-ng-data-dict-<lang>`.
Because the main library has optional dependencies on the aggregate data
crates, those aggregate crates must be published before `espeak-ng`.

## Publish order

The workspace is ~126 crates, which is far past what crates.io will accept in
one sitting, so publishing is driven by `scripts/publish.sh` rather than by
hand. It resolves the order from `cargo metadata`:

1. `espeak-ng-data-phonemes` — phoneme tables, language and voice definitions
2. `espeak-ng-data-dict-<lang>` — the per-language dictionaries
3. `espeak-ng-data-dicts` — the aggregate of all non-Russian dictionaries
4. `espeak-ng` — the library, last, once its dependencies resolve

```bash
./scripts/publish.sh                  # plan: preflight, order, ETA. No writes.
./scripts/publish.sh package          # cargo package every crate, offline
./scripts/publish.sh publish          # the real thing
```

### Rate limits

crates.io meters publishing in two separate buckets:

| Action | Burst | Sustained rate |
|--------|-------|----------------|
| Publishing a **new** crate | 10 | 1 per 10 minutes |
| Publishing a **new version** of an existing crate | 30 | 1 per minute |

The script paces itself with a GCRA token bucket, one per bucket, so it spends
the burst immediately and then settles into the sustained rate. Time spent
building and uploading counts against the interval, so the verification step is
effectively free. A full release of ~100 crates takes a little over an hour;
`plan` mode prints the estimate up front.

If crates.io returns 429 anyway — most often because an earlier run already
drained the bucket — the script parses the `try again after <timestamp>` out of
the error and sleeps until exactly that instant rather than guessing at a
backoff curve. `--max-wait` caps how long a single sleep may be.

Rate limits are per-user and outlive the process, so if a run is interrupted
part-way, restart it with `--assume-drained` to pace from an empty bucket
instead of eating a 429 first.

### Resuming

Every step is idempotent. Before publishing anything the script asks the sparse
index whether that exact `name@version` already exists, and skips it if so, so
re-running after a failure costs nothing but the index lookups. After each
upload it polls the index until the new version is visible, because the CDN in
front of it lags by a few seconds and the next crate may depend on what was
just published.

A failed crate stops the run and prints the exact resume command:

```bash
./scripts/publish.sh publish --from espeak-ng-data-dict-fr
```

Progress is appended to `target/publish-state.tsv` and the full cargo output to
`target/publish-logs/<timestamp>.log`.

### Preflight

`publish` mode refuses to start unless these pass (`plan` reports them but does
not abort):

- the working tree is clean (or `--allow-dirty`)
- `[patch.crates-io]` is absent from the root `Cargo.toml`
- crates.io credentials are available
- **every data crate matches `espeak-ng-data/`**

That last check exists because of a failure mode specific to this workspace.
The data crates are snapshots of `espeak-ng-data/`. If a snapshot drifts from
its source it usually still carries its old version number — so it is already
on crates.io, so the publish step skips it as "already published", and the
drift reaches users silently while every other crate is refreshed. The
freshness check turns that silent skip into a hard failure.

Add `--test` to run the test suite as part of preflight.

### Selecting a subset

```bash
./scripts/publish.sh publish --only 'dict-(en|de|fr)$'   # a few crates
./scripts/publish.sh publish --exclude 'dict-ru$'        # all but one
./scripts/publish.sh publish --no-main                   # data crates only
./scripts/publish.sh publish --limit 10                  # stop after 10
```

`--limit` is useful for spreading a release across sessions: it stops cleanly
and prints the `--from` command to pick up where it left off.

### Legacy script

`scripts/publish_all_crates.py` predates `publish.sh` and does the same job
with a fixed 10-second inter-crate delay and no freshness or index checks. It
is kept for reference; prefer the shell script.

## Before publishing the main crate

1. Remove the `[patch.crates-io]` section from the root `Cargo.toml`.
2. Confirm the `version` numbers in `[dependencies]` for the data crates
   match the versions just published.
3. Run `cargo package -p espeak-ng` to verify the package is valid.

## Versioning

Every crate in the workspace - the library and all 125 data crates - carries
the same version and is released together. Publishing a data crate whose bytes
changed under a version that is already on crates.io is impossible, and letting
the versions drift apart made it easy to miss that a crate needed a bump at
all. One number for the whole workspace removes the question.

## Crate sizes (compressed `.crate` files)

crates.io rejects anything over 10 MB, which is what decides how the
dictionaries are split up.

| Crate | Files | Compressed |
|-------|-------|------------|
| `espeak-ng-data-phonemes` | 409 | 0.39 MB |
| `espeak-ng-data-dicts` (120 languages) | 126 | 4.80 MB |
| `espeak-ng-data-dict-ru` | 7 | 4.82 MB |
| `espeak-ng-data-dict-fo` | 7 | 2.86 MB |
| `espeak-ng-data-dict-ps` | 7 | 1.15 MB |

Russian, Faroese and Pashto stay out of the aggregate for size alone — folding
all three back in takes it to 8.9 MB, close enough to the limit that the next
dictionary refresh would break publishing. The `bundled-data` feature depends
on all four crates, so the split is invisible to callers, and
`tests/bundled_data_complete.rs` walks the actual crate set to make sure no
language is lost on either side of it.

If a future refresh pushes the aggregate near the limit again, move the next
largest dictionary out the same way: give it a `dep:` line in `bundled-data`,
an `install` call in `install_bundled_data`, and drop it from the aggregate's
`include` list.

## Downstream usage

After publication, users add to their `Cargo.toml`:

```toml
[dependencies]
# Core library (text → IPA + synthesis)
espeak-ng = "0.2"

# Optional: embed all data in the binary — no system eSpeak NG needed
espeak-ng = { version = "0.2", features = ["bundled-data"] }
```

Or, for the split approach (pick only the languages you need):

```toml
[dependencies]
espeak-ng               = "0.2"
espeak-ng-data-phonemes = "0.2"   # required for any language
espeak-ng-data-dict-en  = "0.2"
espeak-ng-data-dict-uk  = "0.2"
# espeak-ng-data-dict-ru = "0.2"  # add Russian if needed
```

Or enable the generated selective features on the main crate:

```toml
[dependencies]
espeak-ng = { version = "0.2", features = ["bundled-data-en", "bundled-data-uk"] }
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
