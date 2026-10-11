#!/usr/bin/env bash
# Each line ends in a newline, the last one too.
set -euo pipefail
perl -0pi -e 's/fs::write\(&path, lines\.join\("\\n"\)\)\?;/let text: String = lines.iter().map(|line| format!("{line}\\n")).collect();\n    fs::write(&path, text)?;/' src/lib.rs
