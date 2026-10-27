//! `glave doc`: the words after it, what they ask for, and what to print. The model runs it
//! through bash, taught by the built-in skill; a person can run it as well.

use std::io::Read;
use std::path::{Path, PathBuf};

use changing::{Change, Making, changing};

use crate::data::{self, Query};
use crate::error::OfficeError;
use crate::package::{Kind, Package};
use crate::pdf;
use crate::read::{self, Reading};
use crate::write::{self, Written};
use crate::{document, parts, presentation};

pub const USAGE: &str = "\
glave doc: read, write and edit office files, and change PDFs, with nothing else installed

Usage:
  glave doc read <file> [--sheet <name>] [--rows <first>-<last>]
      a workbook (.xlsx, .xlsm, .xlsb, .xls or .ods) as tables, each formula beside the
      value it last computed to; the first 100 rows of each sheet unless --rows says which.
      A Word document (.docx) or a PowerPoint deck (.pptx) as text, with its tables,
      comments and speaker notes
  glave doc csv <file> [--sheet <name>]
      one sheet's values as CSV: the first sheet unless --sheet names another
  glave doc write <file.xlsx> [<description.json>] [--replace]
      a new workbook from a description in JSON, read from standard input unless its file
      is named: sheets of rows with formulas, number formats, a header row, column widths,
      frozen panes and charts; --replace writes over a file already there
  glave doc unpack <file> <folder>
      an Office file's parts (.docx, .xlsx, .pptx and the like) into a new folder, each XML
      part an element to a line, to edit in place
  glave doc pack <folder> <file> [--replace]
      an unpacked folder back into an Office file, each XML part checked first; --replace
      writes over a file already there
  glave doc merge <new.pdf> <file.pdf> <file.pdf>... [--replace]
      a new PDF of the PDFs' pages, each one's after the one before's
  glave doc pages <file.pdf> <new.pdf> <pages> [--replace]
      a new PDF of the pages named, in that order: 1-3,7 is pages 1 to 3 then 7, 5- runs
      from page 5 to the last, and 3-1 runs backwards
  glave doc rotate <file.pdf> <new.pdf> <degrees> [<pages>] [--replace]
      a new PDF with the pages named, or every page, turned clockwise by 90, 180 or 270
      degrees (-90 turns them anticlockwise)
  glave doc fields <file.pdf>
      a PDF form's fields: each one's name, kind, page and value
  glave doc fill <file.pdf> <new.pdf> [<values.json>] [--replace]
      a new PDF with its form filled from a JSON object of field names and values, read
      from standard input unless its file is named: text for text, true or false for a
      checkbox, one of its values for a choice
  glave doc sql <file>... [<query>] [--sheet <name>] [--csv]
      CSV, TSV, JSON (an array of objects) and workbook files loaded as tables into SQLite,
      each named after its file (name=file names it), a workbook's sheets each a table unless
      --sheet names one; the query's answer as a table, or as CSV with --csv. With no query,
      each table's columns and their types";

/// The largest file read: a workbook is read whole, and so is a description.
const MAX_FILE: u64 = 100 * 1024 * 1024;

/// What `glave doc` answers: what to print, and a warning to give apart from it, so that what
/// is printed stays only what was asked for (a CSV to pipe on).
#[derive(Debug, PartialEq, Eq)]
pub struct Answer {
    pub printed: String,
    pub warning: Option<String>,
}

impl Answer {
    fn printed(printed: String) -> Self {
        Self {
            printed,
            warning: None,
        }
    }
}

/// What `glave doc` was asked.
#[derive(Debug, PartialEq, Eq)]
enum Asked {
    Help,
    Read(PathBuf, Reading),
    Csv(PathBuf, Option<String>),
    Write(Writing),
    /// An Office file to unpack, and the folder for its parts.
    Unpack(PathBuf, PathBuf),
    Pack(Packing),
    /// A PDF whose form's fields to list.
    Fields(PathBuf),
    /// A new PDF to write, and what it is made from.
    Pdf(Making, Change),
    /// Files to load into SQLite, and the query to run.
    Sql(Query),
}

/// An unpacked folder to pack into a file.
#[derive(Debug, PartialEq, Eq)]
struct Packing {
    folder: PathBuf,
    out: PathBuf,
    /// Whether a file already at `out` may be written over.
    replace: bool,
}

/// A new workbook to write.
#[derive(Debug, PartialEq, Eq)]
struct Writing {
    out: PathBuf,
    /// The description's file; standard input when `None`.
    description: Option<PathBuf>,
    /// Whether a file already at `out` may be written over.
    replace: bool,
}

/// Runs `glave doc` on `words`, reading a description from `input` when no file is named for
/// it, and gives what to print.
pub fn run(words: &[String], input: &mut dyn Read) -> Result<Answer, OfficeError> {
    match parse(words)? {
        Asked::Help => Ok(Answer::printed(USAGE.to_owned())),
        Asked::Read(file, reading) => {
            read_any(bytes(&file)?, &named(&file), &reading).map(Answer::printed)
        }
        Asked::Csv(file, sheet) => {
            let bytes = bytes(&file)?;
            if let Some((_, _, kind)) = office(&bytes)
                && kind != Kind::Workbook
            {
                return Err(OfficeError::NotAWorkbook(format!(
                    "{} is {}, which glave doc read reads",
                    named(&file),
                    kind.said()
                )));
            }
            read::csv(bytes, sheet.as_deref())
        }
        Asked::Write(Writing {
            out,
            description,
            replace,
        }) => make(&out, replace, OfficeError::Exists, || {
            let json = json(
                description.as_deref(),
                input,
                "it",
                OfficeError::Description,
                "there is none: give it on standard input, or name its file after the workbook's",
            )?;
            write::write(&json)
        }),
        Asked::Unpack(file, folder) => {
            parts::unpack(&bytes(&file)?, &file, &folder).map(Answer::printed)
        }
        Asked::Pack(Packing {
            folder,
            out,
            replace,
        }) => parts::pack(&folder, &out, replace).map(Answer::printed),
        Asked::Fields(file) => pdf::fields(&changing::given(&file)?).map(Answer::printed),
        Asked::Pdf(making, change) => changing::changed(&making, &change, input),
        Asked::Sql(query) => data::run(&query).map(Answer::printed),
    }
}

/// A file's name as its text says it: the name alone, without its folder.
pub(crate) fn named(file: &Path) -> String {
    file.file_name().map_or_else(
        || file.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The Office file in `bytes`, its main part and what it is, when its parts say.
fn office(bytes: &[u8]) -> Option<(Package<'_>, String, Kind)> {
    let mut package = Package::open(bytes)?;
    let (main, kind) = package.main().ok()??;
    Some((package, main, kind))
}

/// Reads an Office file of one kind from its parts, given its main part and its name.
type ReadParts = fn(&mut Package<'_>, &str, &str) -> Result<String, OfficeError>;

/// How a kind of Office file is read from its parts; `None` for workbooks, which calamine
/// reads whole.
fn reader(kind: Kind) -> Option<ReadParts> {
    match kind {
        Kind::Word => Some(document::read),
        Kind::PowerPoint => Some(presentation::read),
        Kind::Workbook => None,
    }
}

/// The file in `bytes`, named `name`, as text: a Word document or a PowerPoint deck read from
/// its XML, anything else as a workbook.
fn read_any(bytes: Vec<u8>, name: &str, reading: &Reading) -> Result<String, OfficeError> {
    if bytes.starts_with(b"%PDF-") {
        return Err(OfficeError::NotOffice(format!(
            "{name} is a PDF, which the read tool shows as it is; glave doc merge, pages, \
             rotate, fields and fill change PDFs"
        )));
    }
    if let Some((mut package, main, kind)) = office(&bytes)
        && let Some(read) = reader(kind)
    {
        if *reading != Reading::default() {
            return Err(usage(&format!(
                "--sheet and --rows are for workbooks, and {name} is {}",
                kind.said()
            )));
        }
        return read(&mut package, &main, name);
    }
    read::read(bytes, name, reading).map_err(|error| match error {
        OfficeError::NotAWorkbook(why) => OfficeError::NotOffice(why),
        error => error,
    })
}

/// The file `made` makes, written to `out` unless a file is there and `replace` isn't given:
/// `refused` says why it wasn't written.
fn make(
    out: &Path,
    replace: bool,
    refused: fn(PathBuf) -> OfficeError,
    made: impl FnOnce() -> Result<Written, OfficeError>,
) -> Result<Answer, OfficeError> {
    if !replace && std::fs::symlink_metadata(out).is_ok() {
        return Err(refused(out.to_path_buf()));
    }
    let Written { bytes, report } = made()?;
    std::fs::write(out, bytes).map_err(|error| OfficeError::Io {
        action: "write",
        path: out.to_path_buf(),
        error,
    })?;
    Ok(Answer::printed(format!(
        "Wrote {}: {report}",
        out.display()
    )))
}

/// JSON given as the file at `path`, or else on `input`: `what` names it in what `wrong` says
/// is wrong with it, and `missing` says what to do when there is none.
fn json(
    path: Option<&Path>,
    input: &mut dyn Read,
    what: &'static str,
    wrong: fn(String) -> OfficeError,
    missing: &str,
) -> Result<String, OfficeError> {
    let json = if let Some(path) = path {
        bytes(path)?
    } else {
        let mut given = Vec::new();
        input
            .take(MAX_FILE + 1)
            .read_to_end(&mut given)
            .map_err(|error| OfficeError::Io {
                action: "read",
                path: "standard input".into(),
                error,
            })?;
        if u64::try_from(given.len()).unwrap_or(u64::MAX) > MAX_FILE {
            return Err(wrong(format!(
                "{what} is longer than the {MAX_FILE} bytes glave reads"
            )));
        }
        given
    };
    let json =
        String::from_utf8(json).map_err(|_| wrong(format!("{what} is not text in UTF-8")))?;
    if json.trim().is_empty() {
        return Err(wrong(missing.to_owned()));
    }
    Ok(json)
}

/// The file at `path`, whole, when it is not too large to read.
pub(crate) fn bytes(path: &Path) -> Result<Vec<u8>, OfficeError> {
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
    let mut replace = false;
    let mut csv = false;
    let mut rest = rest.iter();
    while let Some(word) = rest.next() {
        match word.as_str() {
            "--sheet" => sheet = Some(value(rest.next(), "--sheet", "a sheet's name")?.clone()),
            "--rows" => rows = Some(row_span(value(rest.next(), "--rows", "rows like 1-100")?)?),
            "--replace" => replace = true,
            "--csv" if verb == "sql" => csv = true,
            "-h" | "--help" => return Ok(Asked::Help),
            flag if flag.starts_with("--") => {
                return Err(usage(&format!("glave doc {verb} takes no {flag}")));
            }
            _ => files.push(PathBuf::from(word)),
        }
    }
    let reading = !replace;
    let writing = sheet.is_none() && rows.is_none();
    match (verb.as_str(), files.as_slice()) {
        ("help" | "-h" | "--help", _) => Ok(Asked::Help),
        ("read", [file]) if reading => Ok(Asked::Read(file.clone(), Reading { sheet, rows })),
        ("csv", [file]) if reading && rows.is_none() => Ok(Asked::Csv(file.clone(), sheet)),
        ("write", [out, ..]) if !ends_in(out, "xlsx") => Err(usage(&format!(
            "glave doc write makes .xlsx workbooks: name the new one first, like report.xlsx, \
             not {}",
            out.display()
        ))),
        ("write", [out, description @ ..]) if writing && description.len() <= 1 => {
            Ok(Asked::Write(Writing {
                out: out.clone(),
                description: description
                    .first()
                    .filter(|path| path.as_os_str() != "-")
                    .cloned(),
                replace,
            }))
        }
        ("unpack", [file, folder]) if reading && writing => {
            Ok(Asked::Unpack(file.clone(), folder.clone()))
        }
        ("pack", [folder, out]) if writing => Ok(Asked::Pack(Packing {
            folder: folder.clone(),
            out: out.clone(),
            replace,
        })),
        ("fields", [file]) if reading && writing => Ok(Asked::Fields(file.clone())),
        ("sql", words) if reading && rows.is_none() => {
            Query::parse(words, sheet, csv).map(Asked::Sql)
        }
        ("merge" | "pages" | "rotate" | "fill", files) if writing => {
            changing(verb, files, replace).map(|(making, change)| Asked::Pdf(making, change))
        }
        (
            "read" | "csv" | "write" | "unpack" | "pack" | "fields" | "merge" | "pages" | "rotate"
            | "fill" | "sql",
            _,
        ) => Err(usage(&format!(
            "glave doc {verb} was not given what it takes"
        ))),
        _ => Err(usage(&format!("glave doc has no command {verb:?}"))),
    }
}

/// Whether `path`'s name ends in `extension`, in any case.
pub(crate) fn ends_in(path: &Path, extension: &str) -> bool {
    path.extension()
        .is_some_and(|its| its.eq_ignore_ascii_case(extension))
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

pub(crate) fn usage(why: &str) -> OfficeError {
    OfficeError::Usage(format!("{why}\n\n{USAGE}"))
}

mod changing;

#[cfg(test)]
mod tests;
