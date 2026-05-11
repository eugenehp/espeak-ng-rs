// Shared CLI implementation for `espeak-ng` and `espeak-ng-rs` binaries.

use std::ffi::OsString;
use std::io::{IsTerminal, Read, Write as _};
use std::path::{Path, PathBuf};

use espeak_ng::engine::{EspeakNg, Parameter};
use espeak_ng::phoneme::PhonemeData;
use espeak_ng::translate::ipa_table::{
    PHON_LENGTHEN, PHON_STRESS_2, PHON_STRESS_3, PHON_STRESS_P, PHON_STRESS_P2,
    PHON_STRESS_PREV, PHON_STRESS_TONIC, PHON_STRESS_U,
};
use espeak_ng::translate::{default_data_dir, normalize_voice_tag, PhonemeCode, Translator};

#[derive(Debug, Default)]
struct Cli {
    voice:         String,
    quiet:         bool,
    ipa:           bool,
    x_mnem:        bool,
    path_override: Option<PathBuf>,
    phonout:       Option<PathBuf>,
    sep:           Option<char>,
    stdout_wav:    bool,
    wave_out:      Option<PathBuf>,
    print_help:    bool,
    print_version: bool,
    rate:          Option<u32>,
    pitch:         Option<u32>,
    range:         Option<u32>,
    amplitude:     Option<u32>,
    file_input:    Option<PathBuf>,
    stdin_all:     bool,
    positionals:   Vec<OsString>,
    had_unknown:   Option<String>,
}

fn argv0_display() -> String {
    std::env::args_os()
        .next()
        .as_ref()
        .and_then(|p| Path::new(p).file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "espeak-ng-rs".to_string())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run() -> std::io::Result<()> {
    let cli = parse_args(std::env::args_os().skip(1).collect());
    if let Some(flag) = cli.had_unknown.clone() {
        eprintln!("{}: unrecognized option '{flag}'", argv0_display());
        return Ok(());
    }
    if cli.print_help {
        print_help();
        return Ok(());
    }
    if cli.print_version {
        let data = resolved_data_dir(cli.path_override.as_deref());
        println!(
            "{}  (Rust port {})\nData at: {}",
            argv0_display(),
            EspeakNg::version(),
            data.display()
        );
        return Ok(());
    }

    let text = gather_text(&cli)?;

    if cli.ipa || cli.x_mnem {
        phoneme_output(&cli, &text)?;
        return Ok(());
    }

    if cli.quiet && cli.wave_out.is_none() && !cli.stdout_wav {
        return Ok(());
    }

    let data_dir = resolved_data_dir(cli.path_override.as_deref());
    let mut b = EspeakNg::builder().voice(&cli.voice).data_dir(&data_dir);
    if let Some(r) = cli.rate {
        b = b.rate(r);
    }
    if let Some(p) = cli.pitch {
        b = b.pitch(p);
    }
    if let Some(v) = cli.amplitude {
        b = b.volume(v);
    }
    if let Some(pr) = cli.range {
        b = b.range(pr);
    }
    let mut engine = b.build().map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::Other, format!("{e}"))
    })?;
    apply_runtime_params(&mut engine, &cli);

    let (pcm, rate) = engine.synth(&text).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::Other, format!("{e}"))
    })?;

    if cli.stdout_wav || cli.wave_out.is_some() {
        let mut buf = Vec::new();
        write_wav_16_mono(&mut buf, &pcm, rate).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, format!("{e}"))
        })?;
        if cli.stdout_wav {
            std::io::stdout().write_all(&buf)?;
        } else if let Some(ref p) = cli.wave_out {
            std::fs::write(p, &buf)?;
        }
        return Ok(());
    }

    eprintln!(
        "espeak-ng (Rust): speech playback is not implemented; use --stdout or -w <file.wav>"
    );
    Ok(())
}

fn apply_runtime_params(engine: &mut EspeakNg, cli: &Cli) {
    if let Some(r) = cli.rate {
        engine.set_parameter(Parameter::Rate, r as i32);
    }
    if let Some(p) = cli.pitch {
        engine.set_parameter(Parameter::Pitch, p as i32);
    }
    if let Some(v) = cli.amplitude {
        engine.set_parameter(Parameter::Volume, v as i32);
    }
    if let Some(g) = cli.range {
        engine.set_parameter(Parameter::Range, g as i32);
    }
}

fn phoneme_output(cli: &Cli, text: &str) -> std::io::Result<()> {
    let data_dir = resolved_data_dir(cli.path_override.as_deref());
    let translator = Translator::new(&cli.voice, Some(&data_dir)).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::Other, format!("{e}"))
    })?;

    let line = if cli.ipa {
        let ipa = translator.text_to_ipa(text).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, format!("{e}"))
        })?;
        apply_ipa_sep(&ipa, cli.sep)
    } else {
        let codes = translator.translate_to_codes(text).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, format!("{e}"))
        })?;
        let mut phdata = PhonemeData::load(&data_dir).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, format!("{e}"))
        })?;
        phdata.select_table_by_name(translator.options.lang.as_str()).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, format!("{e}"))
        })?;
        format_x(&codes, &phdata, cli.sep)
    };

    let line_with_nl = format!("{line}\n");
    if let Some(ref path) = cli.phonout {
        std::fs::write(path, &line_with_nl)?;
    } else {
        std::io::stdout().write_all(line_with_nl.as_bytes())?;
    }
    Ok(())
}

fn apply_ipa_sep(ipa: &str, sep: Option<char>) -> String {
    let Some(sep) = sep else {
        return ipa.to_string();
    };
    let sep_s = sep.to_string();
    let mut out = String::with_capacity(ipa.len() * 2);
    let chars: Vec<char> = ipa.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if out.ends_with(&sep_s) || out.is_empty() {
            /* continue */
        } else if !out.ends_with('\n') && !out.ends_with(' ') {
            out.push(sep);
        }
        let c = chars[i];
        if is_stress_char(c) {
            out.push(c);
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            out.push(c);
            i += 1;
            continue;
        }
        while i < chars.len() && !chars[i].is_whitespace() && !is_stress_char(chars[i]) {
            out.push(chars[i]);
            i += 1;
            if i < chars.len() && is_combining_mark(chars[i]) {
                while i < chars.len() && is_combining_mark(chars[i]) {
                    out.push(chars[i]);
                    i += 1;
                }
                break;
            }
            break;
        }
    }
    out
}

#[inline]
fn is_stress_char(c: char) -> bool {
    matches!(c, 'ˈ' | 'ˌ')
}

#[inline]
fn is_combining_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036f}' | '\u{1ab0}'..='\u{1aff}' | '\u{1dc0}'..='\u{1dff}'
    )
}

fn format_x(codes: &[PhonemeCode], phdata: &PhonemeData, sep: Option<char>) -> String {
    let mut out = String::new();
    let mut word_units: Vec<String> = Vec::new();

    let push_unit = |u: String, word_units: &mut Vec<String>| {
        if u.is_empty() {
            return;
        }
        word_units.push(u);
    };

    let flush_word = |out: &mut String, word_units: &mut Vec<String>, sep: Option<char>| {
        if word_units.is_empty() {
            return;
        }
        let piece = match sep {
            None => word_units.concat(),
            Some(s) => word_units.join(&s.to_string()),
        };
        out.push_str(&piece);
        word_units.clear();
    };

    for pc in codes {
        if pc.is_boundary && pc.code == 15 {
            flush_word(&mut out, &mut word_units, sep);
            out.push(' ');
            continue;
        }
        if pc.is_boundary && pc.code == 0 {
            flush_word(&mut out, &mut word_units, sep);
            if pc.clause_char.is_some() {
                out.push('\n');
            } else {
                out.push(' ');
            }
            continue;
        }

        if pc.code == 15 && !pc.is_boundary {
            flush_word(&mut out, &mut word_units, sep);
            out.push(' ');
            continue;
        }

        match pc.code {
            PHON_STRESS_P | PHON_STRESS_P2 | PHON_STRESS_TONIC => {
                push_unit("'".to_string(), &mut word_units);
            }
            PHON_STRESS_2 | PHON_STRESS_3 => {
                push_unit(",".to_string(), &mut word_units);
            }
            PHON_STRESS_U | PHON_STRESS_PREV => {}
            PHON_LENGTHEN => {
                push_unit(":".to_string(), &mut word_units);
            }
            0 if !pc.is_boundary => {}
            c => {
                if let Some(ph) = phdata.get(c) {
                    if ph.typ == 1 && ph.program == 0 && ph.std_length <= 4 {
                        match ph.std_length {
                            4 => push_unit("'".to_string(), &mut word_units),
                            2 | 3 => push_unit(",".to_string(), &mut word_units),
                            _ => {}
                        }
                    } else if ph.typ != 1 {
                        push_unit(ph.mnemonic_str(), &mut word_units);
                    }
                }
            }
        }
    }

    flush_word(&mut out, &mut word_units, sep);
    out.trim_end().to_string()
}

fn gather_text(cli: &Cli) -> std::io::Result<String> {
    if let Some(ref path) = cli.file_input {
        return std::fs::read_to_string(path);
    }
    if cli.stdin_all || (cli.positionals.is_empty() && !std::io::stdin().is_terminal()) {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        return Ok(buf);
    }
    if !cli.positionals.is_empty() {
        let mut s = String::new();
        for (i, os) in cli.positionals.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            s.push_str(&os.to_string_lossy());
        }
        return Ok(s);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "espeak-ng: no text provided (pass text as arguments, use -f, or pipe stdin)",
    ))
}

fn resolved_data_dir(path_arg: Option<&Path>) -> PathBuf {
    if let Some(p) = path_arg {
        if p.join("espeak-ng-data").join("en_dict").exists() {
            return p.join("espeak-ng-data");
        }
        if p.join("en_dict").exists() {
            return p.to_path_buf();
        }
        return p.to_path_buf();
    }
    PathBuf::from(default_data_dir())
}

fn write_wav_16_mono(w: &mut Vec<u8>, samples: &[i16], sample_rate: u32) -> std::io::Result<()> {
    let data_bytes = samples.len().saturating_mul(2);
    let riff_len = 36u32 + data_bytes as u32;

    w.clear();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&riff_len.to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * 2;
    w.extend_from_slice(&byte_rate.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    for &s in samples {
        w.extend_from_slice(&s.to_le_bytes());
    }
    Ok(())
}

fn parse_args(args: Vec<OsString>) -> Cli {
    let mut cli = Cli {
        voice: "en".to_string(),
        ..Default::default()
    };
    let mut i = 0;
    let mut posix_end = false;
    while i < args.len() {
        let arg = &args[i];
        if posix_end {
            cli.positionals.push(arg.clone());
            i += 1;
            continue;
        }
        let s = arg.to_string_lossy();
        if s == "--" {
            posix_end = true;
            i += 1;
            continue;
        }
        if s == "-" {
            cli.positionals.push(arg.clone());
            i += 1;
            continue;
        }
        if s.starts_with("--") {
            if let Some(rest) = s.strip_prefix("--path=") {
                cli.path_override = Some(PathBuf::from(rest));
            } else if s == "--path" {
                if let Some(p) = args.get(i + 1) {
                    cli.path_override = Some(PathBuf::from(p));
                    i += 1;
                }
            } else if let Some(rest) = s.strip_prefix("--phonout=") {
                cli.phonout = Some(PathBuf::from(rest));
            } else if s == "--phonout" {
                if let Some(p) = args.get(i + 1) {
                    cli.phonout = Some(PathBuf::from(p));
                    i += 1;
                }
            } else if let Some(rest) = s.strip_prefix("--sep=") {
                cli.sep = parse_sep_value(rest);
            } else if s == "--sep" {
                if let Some(p) = args.get(i + 1) {
                    cli.sep = parse_sep_value(&p.to_string_lossy());
                    i += 1;
                }
            } else if s == "--split" {
                if args.get(i + 1).is_some() {
                    i += 1;
                }
            } else {
                match s.as_ref() {
                    "--help" => cli.print_help = true,
                    "--version" => cli.print_version = true,
                    "--ipa" => cli.ipa = true,
                    "--stdout" => cli.stdout_wav = true,
                    "--stdin" => cli.stdin_all = true,
                    _ if s.starts_with("--compile") => {}
                    _ if s == "--voices" || s.starts_with("--voices=") => {}
                    _ if s == "--ssml-break" || s.starts_with("--ssml-break=") => {}
                    _ if s == "--tie" || s.starts_with("--tie=") => {}
                    _ if s == "--punct" || s.starts_with("--punct=") => {}
                    _ => {
                        cli.had_unknown.get_or_insert(s.to_string());
                    }
                }
            }
            i += 1;
            continue;
        }
        if s.starts_with('-') && s != "-" {
            let mut idx = 1usize;
            let bytes = s.as_bytes();
            while idx < bytes.len() {
                let c = bytes[idx] as char;
                match c {
                    'h' => cli.print_help = true,
                    'q' => cli.quiet = true,
                    'x' => cli.x_mnem = true,
                    'm' => { /* SSML: not supported */ }
                    'v' => {
                        if idx + 1 < bytes.len() {
                            let voice = &s[idx + 1..];
                            cli.voice = normalize_voice_tag(voice);
                            idx = bytes.len();
                        } else if let Some(n) = args.get(i + 1) {
                            cli.voice = normalize_voice_tag(&n.to_string_lossy());
                            i += 1;
                        }
                    }
                    's' => {
                        let (n, skip) = parse_trailing_u32(&s[idx + 1..]);
                        if let Some(v) = n {
                            cli.rate = Some(v);
                            idx = bytes.len();
                        } else if let Some(a) = args.get(i + 1) {
                            if let Ok(v) = a.to_string_lossy().parse::<u32>() {
                                cli.rate = Some(v);
                                i += 1;
                            }
                        }
                        let _ = skip;
                    }
                    'p' => {
                        let (n, _) = parse_trailing_u32(&s[idx + 1..]);
                        if let Some(v) = n {
                            cli.pitch = Some(v);
                            idx = bytes.len();
                        } else if let Some(a) = args.get(i + 1) {
                            if let Ok(v) = a.to_string_lossy().parse::<u32>() {
                                cli.pitch = Some(v);
                                i += 1;
                            }
                        }
                    }
                    'P' => {
                        let (n, _) = parse_trailing_u32(&s[idx + 1..]);
                        if let Some(v) = n {
                            cli.range = Some(v);
                            idx = bytes.len();
                        } else if let Some(a) = args.get(i + 1) {
                            if let Ok(v) = a.to_string_lossy().parse::<u32>() {
                                cli.range = Some(v);
                                i += 1;
                            }
                        }
                    }
                    'a' => {
                        let (n, _) = parse_trailing_u32(&s[idx + 1..]);
                        if let Some(v) = n {
                            cli.amplitude = Some(v);
                            idx = bytes.len();
                        } else if let Some(a) = args.get(i + 1) {
                            if let Ok(v) = a.to_string_lossy().parse::<u32>() {
                                cli.amplitude = Some(v);
                                i += 1;
                            }
                        }
                    }
                    'w' => {
                        if idx + 1 < bytes.len() {
                            cli.wave_out = Some(PathBuf::from(&s[idx + 1..]));
                            idx = bytes.len();
                        } else if let Some(a) = args.get(i + 1) {
                            cli.wave_out = Some(PathBuf::from(a));
                            i += 1;
                        }
                    }
                    'f' => {
                        if idx + 1 < bytes.len() {
                            cli.file_input = Some(PathBuf::from(&s[idx + 1..]));
                            idx = bytes.len();
                        } else if let Some(a) = args.get(i + 1) {
                            cli.file_input = Some(PathBuf::from(a));
                            i += 1;
                        }
                    }
                    _ => {
                        cli.had_unknown.get_or_insert(format!("-{c}"));
                        idx = bytes.len();
                    }
                }
                idx += 1;
            }
            i += 1;
            continue;
        }
        cli.positionals.push(arg.clone());
        i += 1;
    }
    cli
}

fn parse_trailing_u32(rest: &str) -> (Option<u32>, ()) {
    if rest.is_empty() {
        return (None, ());
    }
    if let Ok(v) = rest.parse::<u32>() {
        return (Some(v), ());
    }
    (None, ())
}

fn parse_sep_value(s: &str) -> Option<char> {
    if s == "z" {
        return Some('\u{200c}');
    }
    let mut it = s.chars();
    let c = it.next()?;
    Some(c)
}

fn print_help() {
    let prog = argv0_display();
    println!(
        "\
{prog} — Rust port of eSpeak NG (subset of upstream CLI).

{prog} [options] [\"text …\"]

Phonemes / IPA
  --ipa            IPA to stdout (or --phonout)
  -x               Phoneme mnemonics (Kirshenbaum-style)
  -q               No audio (useful with --ipa / -x)
  --sep=<c>        Separator between phoneme units (`z` = U+200C as in upstream)
  --phonout=<f>    Write phoneme output to file

Voice / data
  -v <voice>       Voice / language tag (e.g. en-us, de, fr)
  --path=<dir>     Parent of espeak-ng-data/ or data dir itself

Speech (WAV only; playback not implemented)
  -w <wav>         Write WAV
  --stdout         WAV to stdout
  -s -p -P -a      Speed, pitch, pitch range, amplitude (same ranges as upstream)

Other
  -f <file>        Input text file
  --stdin          Read all stdin as one string
  --help           This help
  --version        Version and data directory

Environment
  ESPEAK_DATA_PATH   Directory containing en_dict, phontab, …

Installation (cargo)
  cargo install espeak-ng              # installs espeak-ng-rs + espeak-ng (default)
  cargo install espeak-ng --no-default-features   # only espeak-ng-rs (keeps C espeak-ng on PATH)
  cargo install espeak-ng --bin espeak-ng-rs     # install a single binary
",
    );
}
