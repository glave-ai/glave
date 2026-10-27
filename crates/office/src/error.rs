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
    #[error("the workbook has no worksheets")]
    NoSheets,
    #[error("there is no sheet named {wanted:?}; the sheets are {}", sheets.join(", "))]
    NoSuchSheet { wanted: String, sheets: Vec<String> },
    #[error("could not read the sheet {0:?}: {1}")]
    Sheet(String, String),
    /// A workbook to write was described wrongly: what is wrong, and where.
    #[error("the workbook's description is not right: {0}")]
    Description(String),
    #[error(
        "{} is already there, and glave doc write makes a whole new workbook: writing over it \
         loses its formatting, its charts and anything else the description doesn't hold. Name \
         a new file, or add --replace if replacing it is what was asked",
        .0.display()
    )]
    Exists(PathBuf),
}
