//! Optional `espeak-ng` executable: enabling crate feature `install-espeak-ng-shim` builds this
//! binary so `cargo install` can place a `espeak-ng` command on your PATH (it may then run
//! before a system eSpeak NG if `~/.cargo/bin` is earlier in `PATH`).
//!
//! The `espeak-ng-rs` binary is always built and is safe to use when you want the Rust port
//! without claiming the `espeak-ng` name.

include!("espeak_cli.rs");
