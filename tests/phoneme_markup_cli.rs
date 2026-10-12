// tests/phoneme_markup_cli.rs
//
// `-x -m` (mnemonic phoneme output in markup mode) must process SSML like
// `--ipa -m` does — previously the `-x` path fed raw text to translate_to_codes,
// so tags/entities were spoken literally.  Skips when en data is absent.

use std::path::Path;
use std::process::Command;

fn run(args: &[&str]) -> String {
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let out = Command::new(bin).args(args).output().expect("run");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn x_markup_mode_processes_ssml() {
    if !Path::new("espeak-ng-data/en_dict").exists() {
        eprintln!("[SKIP] no en data");
        return;
    }
    // Tags stripped, entities decoded, say-as applied — identical to plain text.
    assert_eq!(
        run(&["-x", "-m", "<emphasis>hello</emphasis>"]),
        run(&["-x", "hello"])
    );
    assert_eq!(
        run(&["-x", "-m", r#"<say-as interpret-as="ordinal">3</say-as>"#]),
        run(&["-x", "3rd"])
    );
    // The entity decodes to the *symbol*, which the dictionary reads as "and"
    // preceded by a short pause (`en_list`: `& _and $u+`), so compare against
    // the literal `&` rather than the bare word.
    assert_eq!(run(&["-x", "-m", "Tom &amp; Jerry"]), run(&["-x", "Tom & Jerry"]));

    // Without -m, angle-bracket text is literal (unchanged behaviour).
    assert_ne!(run(&["-x", "<emphasis>hi</emphasis>"]), run(&["-x", "hi"]));
}
