// Invalid UTF-8 from stdin, -f files and argv is decoded lossily; the valid text is still spoken and the exit status is 0.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn run_stdin(args: &[&str], input: &[u8]) -> (Option<i32>, String) {
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child.stdin.take().expect("stdin").write_all(input).expect("write");
    let out = child.wait_with_output().expect("wait");
    (out.status.code(), String::from_utf8_lossy(&out.stdout).into_owned())
}

macro_rules! skip_without_en {
    () => {
        if !Path::new("espeak-ng-data/en_dict").exists() {
            eprintln!("[SKIP] no en data");
            return;
        }
    };
}

fn assert_speaks_valid_parts(code: Option<i32>, out: &str) {
    assert_eq!(code, Some(0), "exit status; stdout: {out}");
    assert!(out.contains("wˈɜːld"), "missing 'world' in {out:?}");
}

#[test]
fn stdin_flag_with_stray_bytes_and_truncated_sequence() {
    skip_without_en!();
    let (code, out) = run_stdin(&["-q", "--ipa", "-v", "en", "--stdin"], b"hello \xff\xfe world \xe2\x82");
    assert_speaks_valid_parts(code, &out);
}

#[test]
fn implicit_stdin_with_lone_surrogate_bytes() {
    skip_without_en!();
    let (code, out) = run_stdin(&["-q", "--ipa", "-v", "en"], b"hello \xed\xa0\x80 world");
    assert_speaks_valid_parts(code, &out);
}

#[test]
fn file_input_with_invalid_bytes() {
    skip_without_en!();
    let path = std::env::temp_dir().join("espeak_ng_rs_invalid_utf8_file.txt");
    std::fs::write(&path, b"hello \xff world").expect("write file");
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let out = Command::new(bin)
        .args(["-q", "--ipa", "-v", "en", "-f"])
        .arg(&path)
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&path);
    assert_speaks_valid_parts(out.status.code(), &String::from_utf8_lossy(&out.stdout));
}

#[cfg(unix)]
#[test]
fn argv_with_invalid_bytes() {
    use std::os::unix::ffi::OsStrExt;
    skip_without_en!();
    let bin = env!("CARGO_BIN_EXE_espeak_cli");
    let out = Command::new(bin)
        .args(["-q", "--ipa", "-v", "en"])
        .arg(std::ffi::OsStr::from_bytes(b"hello \xff world"))
        .output()
        .expect("run");
    assert_speaks_valid_parts(out.status.code(), &String::from_utf8_lossy(&out.stdout));
}
