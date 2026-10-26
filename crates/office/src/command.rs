//! `glave doc`: the words after it, what they ask for, and what to print. The model runs it
//! through bash, taught by the built-in skill; a person can run it as well.

use std::path::{Path, PathBuf};

use crate::error::OfficeError;
use crate::read::{self, Reading};

pub const USAGE: &str = "\
glave doc: read spreadsheets, with nothing else installed

Usage:
  glave doc read <file> [--sheet <name>] [--rows <first>-<last>]
      a workbook (.xlsx, .xlsm, .xlsb, .xls or .ods) as tables, each formula beside the
      value it last computed to; the first 100 rows of each sheet unless --rows says which
  glave doc csv <file> [--sheet <name>]
      one sheet's values as CSV: the first sheet unless --sheet names another";

/// The largest file read: a workbook is read whole.
const MAX_FILE: u64 = 100 * 1024 * 1024;

/// What `glave doc` was asked.
#[derive(Debug, PartialEq, Eq)]
enum Asked {
    Help,
    Read(PathBuf, Reading),
    Csv(PathBuf, Option<String>),
}

/// Runs `glave doc` on `words`, and gives what to print.
pub fn run(words: &[String]) -> Result<String, OfficeError> {
    match parse(words)? {
        Asked::Help => Ok(USAGE.to_owned()),
        Asked::Read(file, reading) => {
            let name = file.file_name().map_or_else(
                || file.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            read::read(bytes(&file)?, &name, &reading)
        }
        Asked::Csv(file, sheet) => read::csv(bytes(&file)?, sheet.as_deref()),
    }
}

/// The file at `path`, whole, when it is not too large to read.
fn bytes(path: &Path) -> Result<Vec<u8>, OfficeError> {
    let io = |error| OfficeError::Io {
        action: "read",
        path: path.to_path_buf(),
        error,
    };
    let size = std::fs::metadata(path).map_err(io)?.len();
    if size > MAX_FILE {
        return Err(OfficeError::TooLarge {
            path: path.to_path_buf(),
            size,
            limit: MAX_FILE,
        });
    }
    std::fs::read(path).map_err(io)
}

fn parse(words: &[String]) -> Result<Asked, OfficeError> {
    let Some((verb, rest)) = words.split_first() else {
        return Ok(Asked::Help);
    };
    let mut files = Vec::new();
    let mut sheet = None;
    let mut rows = None;
    let mut rest = rest.iter();
    while let Some(word) = rest.next() {
        match word.as_str() {
            "--sheet" => sheet = Some(value(rest.next(), "--sheet", "a sheet's name")?.clone()),
            "--rows" => rows = Some(row_span(value(rest.next(), "--rows", "rows like 1-100")?)?),
            "-h" | "--help" => return Ok(Asked::Help),
            flag if flag.starts_with("--") => {
                return Err(usage(&format!("glave doc {verb} takes no {flag}")));
            }
            _ => files.push(PathBuf::from(word)),
        }
    }
    match (verb.as_str(), files.as_slice()) {
        ("help" | "-h" | "--help", _) => Ok(Asked::Help),
        ("read", [file]) => Ok(Asked::Read(file.clone(), Reading { sheet, rows })),
        ("csv", [file]) if rows.is_none() => Ok(Asked::Csv(file.clone(), sheet)),
        ("read" | "csv", _) => Err(usage(&format!(
            "glave doc {verb} was not given what it takes"
        ))),
        _ => Err(usage(&format!("glave doc has no command {verb:?}"))),
    }
}

fn value<'a>(word: Option<&'a String>, flag: &str, what: &str) -> Result<&'a String, OfficeError> {
    word.ok_or_else(|| usage(&format!("{flag} needs {what}")))
}

/// `1-100`, or one row, `7`; numbered from 1.
fn row_span(text: &str) -> Result<(u32, u32), OfficeError> {
    let (first, last) = text.split_once('-').unwrap_or((text, text));
    let number = |text: &str| text.trim().parse::<u32>().ok().filter(|n| *n > 0);
    match (number(first), number(last)) {
        (Some(first), Some(last)) if first <= last => Ok((first, last)),
        _ => Err(usage(&format!(
            "--rows takes rows like 1-100, not {text:?}"
        ))),
    }
}

fn usage(why: &str) -> OfficeError {
    OfficeError::Usage(format!("{why}\n\n{USAGE}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(words: &[&str]) -> Result<Asked, OfficeError> {
        parse(&words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn the_words_say_what_is_asked() {
        assert_eq!(asked(&[]).unwrap(), Asked::Help);
        assert_eq!(asked(&["read", "--help"]).unwrap(), Asked::Help);
        assert_eq!(
            asked(&["read", "--sheet", "Q1 sales", "a.xlsx", "--rows", "101-200"]).unwrap(),
            Asked::Read(
                "a.xlsx".into(),
                Reading {
                    sheet: Some("Q1 sales".into()),
                    rows: Some((101, 200))
                }
            )
        );
        assert_eq!(
            asked(&["read", "a.xlsx", "--rows", "7"]).unwrap(),
            Asked::Read(
                "a.xlsx".into(),
                Reading {
                    sheet: None,
                    rows: Some((7, 7))
                }
            )
        );
        assert_eq!(
            asked(&["csv", "a.xlsx"]).unwrap(),
            Asked::Csv("a.xlsx".into(), None)
        );
        for wrong in [
            &["read"][..],
            &["read", "a.xlsx", "b.xlsx"],
            &["read", "a.xlsx", "--rows", "9-2"],
            &["read", "a.xlsx", "--rows", "0-2"],
            &["read", "a.xlsx", "--rows"],
            &["csv", "a.xlsx", "--rows", "1-2"],
            &["read", "a.xlsx", "--all"],
            &["write", "a.xlsx"],
        ] {
            let error = asked(wrong).unwrap_err().to_string();
            assert!(error.ends_with(USAGE), "{wrong:?}: {error}");
        }
    }
}
