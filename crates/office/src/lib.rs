//! glave-office: office files read and written inside glave, so a person's Excel files work
//! with nothing else installed (#646). `glave doc` is its command, which the model runs through
//! bash; the skills built in beside it ([`SKILLS`]) teach it.
//!
//! - Workbooks are read with calamine (xlsx, xlsm, xlsb, xls, ods), each sheet a table with
//!   every formula beside its value, or one sheet as CSV.
//! - New workbooks are written with rust_xlsxwriter from a description in JSON: rows, formulas,
//!   number formats, a header row, widths, frozen panes and native charts.

mod cells;
mod command;
mod error;
mod read;
mod write;

pub use command::{USAGE, run};
pub use error::OfficeError;

/// The skills built into glave that teach the model `glave doc`, each a SKILL.md's text: one,
/// for spreadsheets.
pub const SKILLS: [&str; 1] = [include_str!("../skills/spreadsheets/SKILL.md")];
