//! A file glave made, as a person previews it (`specs/session.md`, "Files it made"): Markdown a
//! client draws as it draws a reply, bounded. A workbook's sheets, and a CSV file, are tables of
//! what their cells show, the first rows of each; a Word document and a PowerPoint deck read as
//! `glave doc read` reads them, with headings, lists, tables and each slide under its own
//! heading. Past [`MOST_MARKDOWN`] the preview is cut at a line, and says so.

use std::io::Cursor;

use calamine::{Data, Reader, Xlsx};

use crate::command::office;
use crate::error::OfficeError;
use crate::package::Kind;
use crate::reach::within_reach;
use crate::read::{chosen, open, shown};
use crate::{data, document, presentation, table};

/// The rows shown of each sheet, or of a CSV file: a preview, not the whole file.
const ROWS: u32 = 200;

/// The columns shown of each row.
const COLUMNS: u32 = 26;

/// The sheets shown of a workbook.
const SHEETS: usize = 20;

/// The most Markdown a preview gives, in bytes.
pub const MOST_MARKDOWN: usize = 256 * 1024;

/// What a file is called in what it is read as, which the preview leaves out.
const NAME: &str = "the file";

/// A file as a person previews it.
#[derive(Debug, PartialEq, Eq)]
pub struct Preview {
    pub markdown: String,
    /// Whether more of the file than the preview shows is left out.
    pub cut: bool,
}

/// The file in `bytes`, kept as `media_type`, as Markdown to preview: CSV by its media type, an
/// Office file or a workbook by its bytes. Anything else is refused.
pub fn preview(bytes: &[u8], media_type: &str) -> Result<Preview, OfficeError> {
    let (markdown, cut) = if media_type == "text/csv" {
        let text = String::from_utf8_lossy(bytes);
        let rows = data::csv_rows(&text, NAME)?;
        let cut = rows.len() > ROWS as usize;
        (table::lines(&within(rows)).join("\n"), cut)
    } else {
        match office(bytes) {
            Some((mut package, main, Kind::Word)) => {
                (body(&document::read(&mut package, &main, NAME)?), false)
            }
            Some((mut package, main, Kind::PowerPoint)) => {
                (body(&presentation::read(&mut package, &main, NAME)?), false)
            }
            Some((_, _, Kind::Workbook)) => streamed(bytes).or_else(|_| sheets(bytes))?,
            None => sheets(bytes)?,
        }
    };
    Ok(bounded(markdown, cut))
}

/// `markdown` cut at the last line that fits [`MOST_MARKDOWN`], with whether it was, or
/// whether the file was cut before it.
fn bounded(mut markdown: String, cut: bool) -> Preview {
    if markdown.len() <= MOST_MARKDOWN {
        return Preview { markdown, cut };
    }
    let mut end = MOST_MARKDOWN;
    while !markdown.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let end = markdown
        .get(..end)
        .and_then(|kept| kept.rfind('\n'))
        .unwrap_or(end);
    markdown.truncate(end);
    Preview {
        markdown,
        cut: true,
    }
}

/// What `glave doc read` says of a document or a deck, without its first line, which names the
/// file for a model.
fn body(read: &str) -> String {
    read.split_once('\n')
        .map_or("", |(_, rest)| rest)
        .trim()
        .to_owned()
}

/// An xlsx workbook's worksheets, each read as a stream of its cells and stopped at
/// [`ROWS`]: a heading and a table of what its cells show, a formula with no value computed yet
/// shown as itself. Whether rows were left out.
fn streamed(bytes: &[u8]) -> Result<(String, bool), OfficeError> {
    let mut book = Xlsx::new(Cursor::new(bytes))
        .map_err(|error| OfficeError::NotAWorkbook(error.to_string()))?;
    let listed = book.sheets_metadata().to_vec();
    let mut text = Vec::new();
    let mut cut = false;
    for sheet in chosen(&listed, None)?.into_iter().take(SHEETS) {
        let sheet_error =
            |error: calamine::XlsxError| OfficeError::Sheet(sheet.name.clone(), error.to_string());
        let mut cells = book
            .worksheet_cells_reader(&sheet.name)
            .map_err(sheet_error)?;
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut first: Option<u32> = None;
        while let Some(cell) = cells.next_cell_with_formula().map_err(sheet_error)? {
            let (row, column) = cell.pos;
            let top = *first.get_or_insert(row);
            let at = row.saturating_sub(top);
            if at >= ROWS {
                cut = true;
                break;
            }
            if column >= COLUMNS {
                continue;
            }
            let shown = cell_text(Data::from(cell.value), cell.formula.as_deref());
            let (at, column) = (at as usize, column as usize);
            if rows.len() <= at {
                rows.resize_with(at + 1, Vec::new);
            }
            if let Some(line) = rows.get_mut(at) {
                if line.len() <= column {
                    line.resize(column + 1, String::new());
                }
                if let Some(slot) = line.get_mut(column) {
                    *slot = shown;
                }
            }
        }
        text.push(format!("## {}", table::escaped(sheet.name.trim())));
        text.extend(table::lines(&rows));
        text.push(String::new());
    }
    Ok((text.join("\n").trim_end().to_owned(), cut))
}

/// A workbook of another kind than xlsx (xls, xlsb, ods), read whole: its worksheets, each a
/// heading and a table of the first [`ROWS`] of what its cells show.
fn sheets(bytes: &[u8]) -> Result<(String, bool), OfficeError> {
    let mut book = open(bytes.to_vec())?;
    let listed = book.sheets_metadata().to_vec();
    let mut text = Vec::new();
    let mut cut = false;
    for sheet in chosen(&listed, None)?.into_iter().take(SHEETS) {
        within_reach(&mut book, &sheet.name)?;
        let values = book
            .worksheet_range(&sheet.name)
            .map_err(|error| OfficeError::Sheet(sheet.name.clone(), error.to_string()))?;
        let rows: Vec<Vec<String>> = values
            .rows()
            .map(|row| {
                row.iter()
                    .map(|value| cell_text(value.clone(), None))
                    .collect()
            })
            .collect();
        cut |= rows.len() > ROWS as usize;
        text.push(format!("## {}", table::escaped(sheet.name.trim())));
        text.extend(table::lines(&within(rows)));
        text.push(String::new());
    }
    Ok((text.join("\n").trim_end().to_owned(), cut))
}

/// What a cell shows: its value, or its formula while it has none.
fn cell_text(value: Data, formula: Option<&str>) -> String {
    match (value, formula) {
        (Data::Empty, Some(formula)) if !formula.is_empty() => format!("={formula}"),
        (value, _) => shown(&value),
    }
}

/// `rows`, at most [`ROWS`] of [`COLUMNS`].
fn within(rows: Vec<Vec<String>>) -> Vec<Vec<String>> {
    rows.into_iter()
        .take(ROWS as usize)
        .map(|row| row.into_iter().take(COLUMNS as usize).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_csv_file_is_a_table_and_anything_else_unknown_is_refused() {
        let table = preview(b"Item,Total\nFlights,780\n", "text/csv").unwrap();
        assert_eq!(
            table,
            Preview {
                markdown: "| Item | Total |\n|---|---|\n| Flights | 780 |".to_owned(),
                cut: false,
            }
        );
        assert!(preview(b"not a workbook", "application/octet-stream").is_err());
    }

    #[test]
    fn a_long_preview_is_cut_at_a_line() {
        let long = "a line of text\n".repeat(MOST_MARKDOWN / 10);
        let cut = bounded(long, false);
        assert!(cut.cut && cut.markdown.len() <= MOST_MARKDOWN);
        assert!(cut.markdown.ends_with("a line of text"));
        let rows: Vec<String> = (0..300).map(|n| n.to_string()).collect();
        let rows = rows.join("\n");
        let csv = preview(rows.as_bytes(), "text/csv").unwrap();
        assert!(
            csv.cut && csv.markdown.lines().count() == 201,
            "{}",
            csv.markdown.len()
        );
    }
}
