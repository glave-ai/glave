//! glave-office: office files and PDFs read and written inside glave, so a person's Excel,
//! Word, PowerPoint and PDF files work with nothing else installed (#646). `glave doc` is its
//! command, which the model runs through bash; the skills built in beside it ([`SKILLS`])
//! teach it.
//!
//! - Workbooks are read with calamine (xlsx, xlsm, xlsb, xls, ods): each sheet a table with
//!   every formula beside its value, or one sheet as CSV.
//! - New workbooks are written with rust_xlsxwriter from a description in JSON: rows,
//!   formulas, number formats, a header row, column widths, frozen panes and native charts.
//! - Word documents and PowerPoint decks are read from their parts' XML (zip, quick-xml):
//!   a document's text with its headings, lists, tables and comments, a deck's slides with
//!   their notes.
//! - Any Office file is unpacked into a folder of its parts to edit in place, and packed back
//!   once each part is checked, so what the edit didn't touch stays as it was.
//! - PDFs are changed with lopdf: pages merged, kept, reordered and turned, and forms listed
//!   and filled. Reading a PDF stays with the `read` tool.

mod cells;
mod command;
mod document;
mod error;
mod layout;
mod package;
mod parts;
mod pdf;
mod presentation;
mod read;
mod table;
mod write;
mod xml;

pub use command::{Answer, USAGE, run};
pub use error::OfficeError;

/// The skills built into glave that teach the model `glave doc`, each a SKILL.md's text: for
/// spreadsheets, Word documents, PowerPoint decks and PDFs.
pub const SKILLS: [&str; 4] = [
    include_str!("../skills/spreadsheets/SKILL.md"),
    include_str!("../skills/documents/SKILL.md"),
    include_str!("../skills/presentations/SKILL.md"),
    include_str!("../skills/pdfs/SKILL.md"),
];
