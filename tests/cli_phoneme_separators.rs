// tests/cli_phoneme_separators.rs
//
// `--sep`, `--tie` and the deprecated `--ipa=1|2|3` forms.
//
// Upstream keeps one separator *character* and a sticky tie *flag*, so the two
// options interact: `--tie --sep=.` ties with a dot, `--sep=. --tie` ties with
// U+0361, and either option on its own implies `-x`.  Every expectation here
// was taken from a locally built upstream `espeak-ng` (`ba90c8e`).

use std::path::Path;
use std::process::Command;

fn run(args: &[&str]) -> String {
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let out = Command::new(bin).args(args).output().expect("run");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

macro_rules! skip_without_en {
    () => {
        if !Path::new("espeak-ng-data/en_dict").exists() {
            eprintln!("[SKIP] no en data");
            return;
        }
    };
}

/// A tie is written *inside* a phoneme name (`tʃ` → `t͡ʃ`), never around a
/// stress mark or a length sign.
#[test]
fn tie_joins_the_parts_of_one_phoneme() {
    skip_without_en!();
    assert_eq!(run(&["-q", "-x", "--tie", "the cheese"]), "D@ t\u{361}S'i:z");
    assert_eq!(run(&["-q", "-x", "--tie=-", "the cheese"]), "D@ t-S'i:z");
    assert_eq!(run(&["-q", "--ipa", "--tie", "the cheese"]), "ðə t\u{361}ʃˈiːz");
    assert_eq!(run(&["-q", "--ipa", "--tie=-", "the cheese"]), "ðə t-ʃˈiːz");
    // `z` is upstream's shorthand for a zero-width joiner.
    assert_eq!(run(&["-q", "--ipa", "--tie=z", "the cheese"]), "ðə t\u{200d}ʃˈiːz");
}

/// A separator goes *between* phonemes; a bare `--sep` is a space, and the gap
/// between words then doubles.  `z` is a zero-width non-joiner.
#[test]
fn sep_separates_phonemes() {
    skip_without_en!();
    assert_eq!(run(&["-q", "-x", "--sep=.", "the cheese"]), "D.@ tS.'i:.z");
    assert_eq!(run(&["-q", "-x", "--sep", "the cheese"]), "D @  tS 'i: z");
    assert_eq!(run(&["-q", "-x", "--sep=z", "the cheese"]), "D\u{200c}@ tS\u{200c}'i:\u{200c}z");
    assert_eq!(run(&["-q", "--ipa", "--sep=.", "the cheese"]), "ð.ə tʃ.ˈiː.z");
}

/// Either option implies `-x`: upstream's `--sep`/`--tie` set
/// `espeakPHONEMES_SHOW` as well as the separator.
#[test]
fn sep_and_tie_imply_phoneme_output() {
    skip_without_en!();
    assert_eq!(run(&["-q", "--sep=.", "the cheese"]), "D.@ tS.'i:.z");
    assert_eq!(run(&["-q", "--tie", "the cheese"]), "D@ t\u{361}S'i:z");
}

/// The tie flag is sticky and the character is last-wins, exactly as C's
/// `phoneme_options |= …` / `phonemes_separator = …` pair behaves.
#[test]
fn tie_flag_is_sticky_and_the_character_is_last_wins() {
    skip_without_en!();
    // --tie then --sep: still a tie, but with the dot.
    assert_eq!(run(&["-q", "-x", "--tie", "--sep=.", "the cheese"]), "D@ t.S'i:z");
    // --sep then --tie: a tie with --tie's own default.
    assert_eq!(run(&["-q", "-x", "--sep=z", "--tie", "the cheese"]), "D@ t\u{361}S'i:z");
    assert_eq!(run(&["-q", "-x", "--tie", "--sep=z", "the cheese"]), "D@ t\u{200c}S'i:z");
    // A bare --sep after --tie=- ties with a space.
    assert_eq!(run(&["-q", "-x", "--tie=-", "--sep", "the cheese"]), "D@ t S'i:z");
}

/// `--ipa=1|2|3` are upstream's deprecated spellings of `--sep=_`, `--tie` and
/// `--tie=z`; any other value is plain IPA.
#[test]
fn deprecated_numeric_ipa_forms() {
    skip_without_en!();
    assert_eq!(run(&["-q", "--ipa=1", "the cheese"]), "ð_ə tʃ_ˈiː_z");
    assert_eq!(run(&["-q", "--ipa=2", "the cheese"]), "ðə t\u{361}ʃˈiːz");
    assert_eq!(run(&["-q", "--ipa=3", "the cheese"]), "ðə t\u{200d}ʃˈiːz");
    assert_eq!(run(&["-q", "--ipa=4", "the cheese"]), run(&["-q", "--ipa", "the cheese"]));
    // The numeric form sets the character, so a later `--sep` still wins.
    assert_eq!(run(&["-q", "--ipa=2", "--sep=.", "the cheese"]), "ðə t.ʃˈiːz");
}

/// An optional argument only attaches with `=`, so the word after a bare
/// `--sep` / `--tie` is text, not the separator.
#[test]
fn an_optional_argument_never_eats_the_text() {
    skip_without_en!();
    assert_eq!(run(&["-q", "-x", "--tie", "cheese"]), "t\u{361}S'i:z");
    assert_eq!(run(&["-q", "-x", "--sep", "cheese"]), "tS 'i: z");
}
