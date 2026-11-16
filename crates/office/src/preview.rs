//! A file glave made, as a person previews it (`specs/session.md`, "Files it made"): Markdown a
//! client draws as it draws a reply. A workbook's sheets, and a CSV file, are tables of what
//! their cells show; a Word document and a PowerPoint deck read as `glave doc read` reads them,
//! with headings, lists, tables and each slide under its own heading.

use calamine::{Data, Reader};

use crate::command::office;
use crate::error::OfficeError;
use crate::package::Kind;
use crate::read::{chosen, open, shown};
use crate::{data, document, presentation, table};

/// The rows shown of each sheet, or of a CSV file: a preview, not the whole file.
const ROWS: usize = 200;

/// The columns shown of each row.
const COLUMNS: usize = 26;

/// The sheets shown of a workbook.
const SHEETS: usize = 20;

/// What a file is called in what it is read as, which the preview leaves out.
const NAME: &str = "the file";

/// The file in `bytes`, kept as `media_type`, as Markdown to preview: CSV or TSV by its media
/// type, an Office file or a workbook by its bytes. Anything else is refused.
pub fn preview(bytes: &[u8], media_type: &str) -> Result<String, OfficeError> {
    if matches!(media_type, "text/csv" | "text/tab-separated-values") {
        let text = String::from_utf8_lossy(bytes);
        let rows = data::csv_rows(&text, NAME)?;
        return Ok(cut(rows).join("\n"));
    }
    match office(bytes) {
        Some((mut package, main, Kind::Word)) => body(&document::read(&mut package, &main, NAME)?),
        Some((mut package, main, Kind::PowerPoint)) => {
            body(&presentation::read(&mut package, &main, NAME)?)
        }
        _ => sheets(bytes),
    }
}

/// What `glave doc read` says of a document or a deck, without its first line, which names the
/// file for a model.
fn body(read: &str) -> Result<String, OfficeError> {
    Ok(read
        .split_once('\n')
        .map_or("", |(_, rest)| rest)
        .trim()
        .to_owned())
}

/// A workbook's worksheets, each a heading and a table of what its cells show; a formula with
/// no value computed yet (a workbook no spreadsheet app has opened) shows as itself.
fn sheets(bytes: &[u8]) -> Result<String, OfficeError> {
    let mut book = open(bytes.to_vec())?;
    let listed = book.sheets_metadata().to_vec();
    let mut text = Vec::new();
    for sheet in chosen(&listed, None)?.into_iter().take(SHEETS) {
        let values = book
            .worksheet_range(&sheet.name)
            .map_err(|error| OfficeError::Sheet(sheet.name.clone(), error.to_string()))?;
        let formulas = book.worksheet_formula(&sheet.name).unwrap_or_default();
        let (height, width) = values.get_size();
        let (height, width) = (height.max(formulas.height()), width.max(formulas.width()));
        let start = values.start().unwrap_or((0, 0));
        let rows: Vec<Vec<String>> = (0..height)
            .take(ROWS)
            .map(|row| {
                (0..width)
                    .take(COLUMNS)
                    .map(|column| {
                        let at = (start.0 + as_u32(row), start.1 + as_u32(column));
                        cell(values.get_value(at), formulas.get_value(at))
                    })
                    .collect()
            })
            .collect();
        text.push(format!("## {}", table::escaped(sheet.name.trim())));
        text.extend(table::lines(&rows));
        text.push(String::new());
    }
    Ok(text.join("\n").trim_end().to_owned())
}

/// What a cell shows: its value, or its formula while it has none.
fn cell(value: Option<&Data>, formula: Option<&String>) -> String {
    match (value, formula) {
        (Some(Data::Empty) | None, Some(formula)) if !formula.is_empty() => format!("={formula}"),
        (Some(value), _) => shown(value),
        (None, _) => String::new(),
    }
}

/// `rows`, at most [`ROWS`] of [`COLUMNS`], as a table.
fn cut(rows: Vec<Vec<String>>) -> Vec<String> {
    let rows: Vec<Vec<String>> = rows
        .into_iter()
        .take(ROWS)
        .map(|row| row.into_iter().take(COLUMNS).collect())
        .collect();
    table::lines(&rows)
}

fn as_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_csv_file_is_a_table_and_anything_else_unknown_is_refused() {
        let table = preview(b"Item,Total\nFlights,780\n", "text/csv").unwrap();
        assert_eq!(table, "| Item | Total |\n|---|---|\n| Flights | 780 |");
        assert!(preview(b"not a workbook", "application/octet-stream").is_err());
    }
}
