//! What goes wrong reading or writing an office file, each said so the model can act on it.

use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum OfficeError {
    /// The words after `glave doc` are not a command it takes.
    #[error("{0}")]
    Usage(String),
    #[error("could not {action} {}: {error}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        error: std::io::Error,
    },
    #[error("{} is {size} bytes; glave reads files up to {limit} bytes", path.display())]
    TooLarge {
        path: PathBuf,
        size: u64,
        limit: u64,
    },
    #[error("this is not a workbook glave can read (.xlsx, .xlsm, .xlsb, .xls or .ods): {0}")]
    NotAWorkbook(String),
    #[error(
        "this is not a file glave doc reads: a workbook (.xlsx, .xlsm, .xlsb, .xls or .ods), a \
         Word document (.docx) or a PowerPoint deck (.pptx): {0}"
    )]
    NotOffice(String),
    /// An Office file whose zip or parts can't be read.
    #[error("the file is damaged: {0}")]
    Damaged(String),
    /// A part that is not well-formed XML: which, and where it breaks.
    #[error("{part} is not well-formed XML: {broken}")]
    Broken { part: String, broken: String },
    #[error(
        "{0} is not an Office file glave can unpack: those are zips of XML parts, as .docx, \
         .xlsx and .pptx files are"
    )]
    NotAPackage(String),
    /// What stopped an Office file being unpacked or packed, said so it can be put right.
    #[error("{0}")]
    Parts(String),
    #[error(
        "{} is already there. Add --replace to write over it, or name a new file",
        .0.display()
    )]
    AlreadyThere(PathBuf),
    #[error("the workbook has no worksheets")]
    NoSheets,
    #[error("there is no sheet named {wanted:?}; the sheets are {}", sheets.join(", "))]
    NoSuchSheet { wanted: String, sheets: Vec<String> },
    #[error("could not read the sheet {0:?}: {1}")]
    Sheet(String, String),
    /// A workbook to write was described wrongly: what is wrong, and where.
    #[error("the workbook's description is not right: {0}")]
    Description(String),
    #[error("{0} is not a PDF")]
    NotAPdf(String),
    /// A PDF that can't be opened or changed for its encryption: which, and why.
    #[error(
        "{0}: glave doesn't change encrypted PDFs or take their protection off. Ask for a copy \
         that isn't encrypted"
    )]
    Encrypted(String),
    /// What stopped a PDF being read or changed, said so it can be put right.
    #[error("{0}")]
    Pdf(String),
    /// The values to fill a form with are not right: what is wrong. Nothing was written.
    #[error("Nothing was written: {0}")]
    Values(String),
    /// A file `glave doc sql` can't load as a table: which, and why.
    #[error("{0}")]
    Data(String),
    /// A query SQLite refused: what it said, and the tables there are.
    #[error("the query failed: {0}")]
    Query(String),
    #[error(
        "{} is already there, and glave doc write makes a whole new workbook: writing over it \
         loses its formatting, its charts and anything else the description doesn't hold. Name \
         a new file, or add --replace if replacing it is what was asked",
        .0.display()
    )]
    Exists(PathBuf),
}
