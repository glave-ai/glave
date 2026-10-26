//! `glave doc`: the words after it, what they ask for, and what to print. The model runs it
//! through bash, taught by the built-in skill; a person can run it as well.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::error::OfficeError;
use crate::read::{self, Reading};
use crate::write;

pub const USAGE: &str = "\
glave doc: read and write spreadsheets, with nothing else installed

Usage:
  glave doc read <file> [--sheet <name>] [--rows <first>-<last>]
      a workbook (.xlsx, .xlsm, .xlsb, .xls or .ods) as tables, each formula beside the
      value it last computed to; the first 100 rows of each sheet unless --rows says which
  glave doc csv <file> [--sheet <name>]
      one sheet's values as CSV: the first sheet unless --sheet names another
  glave doc write <file.xlsx> <description.json>
      a new workbook from a description in JSON (- reads it from standard input): sheets of
      rows, with formulas, number formats, bold cells, a header row, column widths, frozen
      panes and charts";

/// The largest file read: a workbook is read whole.
const MAX_FILE: u64 = 100 * 1024 * 1024;

/// What `glave doc` was asked.
#[derive(Debug, PartialEq, Eq)]
enum Asked {
    Help,
    Read(PathBuf, Reading),
    Csv(PathBuf, Option<String>),
    Write { out: PathBuf, description: PathBuf },
}

/// Runs `glave doc` on `words`, reading a description from `input` when it is given as `-`, and
/// gives what to print.
pub fn run(words: &[String], input: &mut dyn Read) -> Result<String, OfficeError> {
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
        Asked::Write { out, description } => {
            let json = if description == Path::new("-") {
                let mut json = String::new();
                input
                    .take(MAX_FILE)
                    .read_to_string(&mut json)
                    .map_err(|error| OfficeError::Io {
                        action: "read the description from",
                        path: description.clone(),
                        error,
                    })?;
                json
            } else {
                String::from_utf8_lossy(&bytes(&description)?).into_owned()
            };
            let workbook = write::write(&json)?;
            std::fs::write(&out, workbook).map_err(|error| OfficeError::Io {
                action: "write",
                path: out.clone(),
                error,
            })?;
            Ok(format!("Wrote {}.", out.display()))
        }
    }
}

/// The file at `path`, whole, when it is not too large to read.
fn bytes(path: &Path) -> Result<Vec<u8>, OfficeError> {
    let io = |action| {
        move |error| OfficeError::Io {
            action,
            path: path.to_path_buf(),
            error,
        }
    };
    let size = std::fs::metadata(path).map_err(io("read"))?.len();
    if size > MAX_FILE {
        return Err(OfficeError::TooLarge {
            path: path.to_path_buf(),
            size,
            limit: MAX_FILE,
        });
    }
    std::fs::read(path).map_err(io("read"))
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
        ("write", [out, description]) if sheet.is_none() && rows.is_none() => Ok(Asked::Write {
            out: out.clone(),
            description: description.clone(),
        }),
        ("read" | "csv" | "write", _) => Err(usage(&format!(
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
            asked(&["csv", "a.xlsx"]).unwrap(),
            Asked::Csv("a.xlsx".into(), None)
        );
        assert_eq!(
            asked(&["write", "out.xlsx", "-"]).unwrap(),
            Asked::Write {
                out: "out.xlsx".into(),
                description: "-".into()
            }
        );
        for wrong in [
            &["read"][..],
            &["read", "a.xlsx", "b.xlsx"],
            &["read", "a.xlsx", "--rows", "9-2"],
            &["read", "a.xlsx", "--rows"],
            &["csv", "a.xlsx", "--rows", "1-2"],
            &["write", "out.xlsx"],
            &["read", "a.xlsx", "--all"],
            &["open", "a.xlsx"],
        ] {
            let error = asked(wrong).unwrap_err().to_string();
            assert!(error.ends_with(USAGE), "{wrong:?}: {error}");
        }
    }
}
