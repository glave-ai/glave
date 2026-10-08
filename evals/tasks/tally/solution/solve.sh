#!/usr/bin/env bash
set -euo pipefail
cat > src/main.rs <<'EOF'
use std::io::Read;
use std::process::ExitCode;

#[derive(Default)]
struct Counts {
    lines: u64,
    words: u64,
    bytes: u64,
}

fn count(data: &[u8]) -> Counts {
    let mut counts = Counts { bytes: data.len() as u64, ..Counts::default() };
    let mut in_word = false;
    for &byte in data {
        if byte == b'\n' {
            counts.lines += 1;
        }
        if byte.is_ascii_whitespace() {
            in_word = false;
        } else if !in_word {
            in_word = true;
            counts.words += 1;
        }
    }
    counts
}

fn line(counts: &Counts, show: (bool, bool, bool), name: Option<&str>) -> String {
    let mut parts = Vec::new();
    if show.0 {
        parts.push(counts.lines.to_string());
    }
    if show.1 {
        parts.push(counts.words.to_string());
    }
    if show.2 {
        parts.push(counts.bytes.to_string());
    }
    if let Some(name) = name {
        parts.push(name.to_string());
    }
    parts.join(" ")
}

fn main() -> ExitCode {
    let (mut l, mut w, mut c) = (false, false, false);
    let mut files = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.strip_prefix('-') {
            Some(flags) if !flags.is_empty() => {
                for flag in flags.chars() {
                    match flag {
                        'l' => l = true,
                        'w' => w = true,
                        'c' => c = true,
                        other => {
                            eprintln!("tally: unknown flag -{other}");
                            return ExitCode::from(2);
                        }
                    }
                }
            }
            _ => files.push(arg),
        }
    }
    let show = if l || w || c { (l, w, c) } else { (true, true, true) };
    if files.is_empty() {
        let mut data = Vec::new();
        if std::io::stdin().read_to_end(&mut data).is_err() {
            eprintln!("tally: -: cannot read");
            return ExitCode::from(1);
        }
        println!("{}", line(&count(&data), show, None));
        return ExitCode::SUCCESS;
    }
    let mut total = Counts::default();
    let mut status = ExitCode::SUCCESS;
    for file in &files {
        match std::fs::read(file) {
            Ok(data) => {
                let counts = count(&data);
                println!("{}", line(&counts, show, Some(file)));
                total.lines += counts.lines;
                total.words += counts.words;
                total.bytes += counts.bytes;
            }
            Err(_) => {
                eprintln!("tally: {file}: cannot read");
                status = ExitCode::from(1);
            }
        }
    }
    if files.len() > 1 {
        println!("{}", line(&total, show, Some("total")));
    }
    status
}
EOF
