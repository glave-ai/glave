//! Reading a workbook (xlsx, xlsm, xlsb, xls or ods, told by its bytes) with calamine: each
//! sheet as a table a model reads, with the row numbers and column letters to name a cell by,
//! every formula beside the value it last computed to; or one sheet as CSV.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Cursor;

use calamine::{Data, Reader, Sheet, SheetType, SheetVisible, Sheets, open_workbook_auto_from_rs};
use rust_xlsxwriter::utility::column_number_to_name;

use crate::cells::{Cell, Range};
use crate::error::OfficeError;

/// The rows shown of each sheet when no rows are asked for: enough to see what a sheet holds,
/// little enough that a workbook of many sheets stays readable.
pub(crate) const ROWS_SHOWN: u32 = 100;

/// The columns shown of a sheet: a row wider than this is better read as CSV.
const COLUMNS_SHOWN: u16 = 50;

/// Which part of a workbook to read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Reading {
    /// Only this sheet; every sheet when `None`.
    pub(crate) sheet: Option<String>,
    /// Only these rows, numbered from 1 as a person reads them; the first [`ROWS_SHOWN`] when
    /// `None`.
    pub(crate) rows: Option<(u32, u32)>,
}

type Book = Sheets<Cursor<Vec<u8>>>;

fn open(bytes: Vec<u8>) -> Result<Book, OfficeError> {
    open_workbook_auto_from_rs(Cursor::new(bytes))
        .map_err(|error| OfficeError::NotAWorkbook(error.to_string()))
}

/// The workbook in `bytes`, named `name`, as text: its sheets and defined names, then each
/// sheet read as `reading` asks.
pub(crate) fn read(bytes: Vec<u8>, name: &str, reading: &Reading) -> Result<String, OfficeError> {
    let mut book = open(bytes)?;
    let sheets: Vec<Sheet> = book.sheets_metadata().to_vec();
    let chosen = chosen(&sheets, reading.sheet.as_deref())?;
    let mut text = String::new();
    let listed: Vec<String> = sheets.iter().map(describe).collect();
    let count = if sheets.len() == 1 {
        "1 sheet".to_owned()
    } else {
        format!("{} sheets", sheets.len())
    };
    let _ = writeln!(text, "{name}: {count}: {}", listed.join(", "));
    // Excel's own names (`_xlnm.` and after: a filter's range, the print area) are left out.
    for (defined, refers) in book.defined_names() {
        if !defined.starts_with("_xlnm.") {
            let _ = writeln!(text, "Name {defined} = {refers}");
        }
    }
    for sheet in chosen {
        let values = book
            .worksheet_range(&sheet.name)
            .map_err(|error| OfficeError::Sheet(sheet.name.clone(), error.to_string()))?;
        // A sheet's formulas are read apart from its values; a workbook kind that keeps none
        // (as CSV-like ones) reads as having none.
        let formulas = book.worksheet_formula(&sheet.name).unwrap_or_default();
        text.push('\n');
        write_sheet(
            &mut text,
            name,
            &sheet.name,
            &values,
            &formulas,
            reading.rows,
        );
    }
    Ok(text)
}

/// One sheet of the workbook in `bytes` as CSV: the first when `sheet` is `None`. Each formula
/// gives the value it last computed to; dates are written as `2026-01-31`.
pub(crate) fn csv(bytes: Vec<u8>, sheet: Option<&str>) -> Result<String, OfficeError> {
    let mut book = open(bytes)?;
    let sheets: Vec<Sheet> = book.sheets_metadata().to_vec();
    let name = match sheet {
        Some(sheet) => chosen(&sheets, Some(sheet))?
            .first()
            .map(|chosen| chosen.name.clone()),
        None => sheets
            .iter()
            .find(|sheet| sheet.typ == SheetType::WorkSheet)
            .map(|sheet| sheet.name.clone()),
    }
    .ok_or(OfficeError::NoSheets)?;
    let values = book
        .worksheet_range(&name)
        .map_err(|error| OfficeError::Sheet(name.clone(), error.to_string()))?;
    let mut text = String::new();
    for row in values.rows() {
        let fields: Vec<String> = row.iter().map(|value| quoted(&shown(value))).collect();
        text.push_str(&fields.join(","));
        text.push('\n');
    }
    Ok(text)
}

/// The worksheets to read: the one named, or every worksheet (charts and macros hold no cells).
fn chosen<'a>(sheets: &'a [Sheet], wanted: Option<&str>) -> Result<Vec<&'a Sheet>, OfficeError> {
    let worksheets = sheets
        .iter()
        .filter(|sheet| sheet.typ == SheetType::WorkSheet);
    let Some(wanted) = wanted else {
        return Ok(worksheets.collect());
    };
    let found = worksheets
        .clone()
        .find(|sheet| sheet.name == wanted)
        .or_else(|| {
            worksheets
                .clone()
                .find(|sheet| sheet.name.eq_ignore_ascii_case(wanted))
        });
    match found {
        Some(sheet) => Ok(vec![sheet]),
        None => Err(OfficeError::NoSuchSheet {
            wanted: wanted.to_owned(),
            sheets: worksheets.map(|sheet| sheet.name.clone()).collect(),
        }),
    }
}

/// A sheet's name with what kind it is when it is not a plain, visible worksheet.
fn describe(sheet: &Sheet) -> String {
    let kind = match sheet.typ {
        SheetType::WorkSheet => None,
        SheetType::ChartSheet => Some("a chart"),
        SheetType::DialogSheet | SheetType::MacroSheet | SheetType::Vba => Some("macros"),
    };
    let hidden = (sheet.visible != SheetVisible::Visible).then_some("hidden");
    let notes: Vec<&str> = kind.into_iter().chain(hidden).collect();
    if notes.is_empty() {
        sheet.name.clone()
    } else {
        format!("{} ({})", sheet.name, notes.join(", "))
    }
}

/// One sheet as a table: a header of column letters, then each row with its number.
fn write_sheet(
    text: &mut String,
    file: &str,
    sheet: &str,
    values: &calamine::Range<Data>,
    formulas: &calamine::Range<String>,
    rows: Option<(u32, u32)>,
) {
    let Some(used) = used(values, formulas) else {
        let _ = writeln!(text, "## {sheet}: empty");
        return;
    };
    let _ = writeln!(text, "## {sheet} ({used})");
    let formulas = formulas_by_cell(formulas);
    let (from, to) = rows.unwrap_or((used.first.row + 1, used.first.row + ROWS_SHOWN));
    let from = from.max(used.first.row + 1);
    let to = to.min(used.last.row + 1);
    let last_shown = used
        .last
        .column
        .min(used.first.column.saturating_add(COLUMNS_SHOWN - 1));
    let columns = used.first.column..=last_shown;
    let header: Vec<String> = columns.clone().map(column_number_to_name).collect();
    let _ = writeln!(text, "| | {} |", header.join(" | "));
    let _ = writeln!(text, "|---|{}", "---|".repeat(header.len()));
    for row in from.saturating_sub(1)..to {
        let cells: Vec<String> = columns
            .clone()
            .map(|column| {
                let at = Cell { row, column };
                let value = values.get_value((row, u32::from(column)));
                match formulas.get(&at) {
                    Some(formula) => with_formula(formula, value),
                    None => value.map(shown).unwrap_or_default(),
                }
            })
            .map(|cell| escaped(&cell))
            .collect();
        let _ = writeln!(text, "| {} | {} |", row + 1, cells.join(" | "));
    }
    if last_shown < used.last.column {
        let _ = writeln!(
            text,
            "… columns {} to {} not shown: glave doc csv {file} --sheet {}",
            column_number_to_name(last_shown + 1),
            column_number_to_name(used.last.column),
            argument(sheet)
        );
    }
    let last = used.last.row + 1;
    if to < last {
        let next_to = to.saturating_add(ROWS_SHOWN).min(last);
        let _ = writeln!(
            text,
            "… rows {} to {last} not shown: glave doc read {file} --sheet {} --rows {}-{next_to}",
            to + 1,
            argument(sheet),
            to + 1
        );
    }
}

/// The cells a sheet uses: those with values, and those with formulas, which have no value
/// until an app computes them.
fn used(values: &calamine::Range<Data>, formulas: &calamine::Range<String>) -> Option<Range> {
    let first = [values.start(), formulas.start()]
        .into_iter()
        .flatten()
        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1)))?;
    let last = [values.end(), formulas.end()]
        .into_iter()
        .flatten()
        .reduce(|a, b| (a.0.max(b.0), a.1.max(b.1)))?;
    Some(Range {
        first: cell(first),
        last: cell(last),
    })
}

fn cell((row, column): (u32, u32)) -> Cell {
    Cell {
        row,
        column: u16::try_from(column).unwrap_or(u16::MAX),
    }
}

/// Every formula in the sheet by its cell. calamine gives them without their `=`.
fn formulas_by_cell(formulas: &calamine::Range<String>) -> BTreeMap<Cell, String> {
    let Some(start) = formulas.start() else {
        return BTreeMap::new();
    };
    formulas
        .used_cells()
        .filter(|(_, _, formula)| !formula.is_empty())
        .filter_map(|(row, column, formula)| {
            let row = u32::try_from(row).ok()?.checked_add(start.0)?;
            let column = u32::try_from(column).ok()?.checked_add(start.1)?;
            // As Excel shows them: without the prefixes it stores newer functions under.
            let formula = formula.replace("_xlfn.", "").replace("_xlws.", "");
            Some((cell((row, column)), format!("={formula}")))
        })
        .collect()
}

/// A formula and the value it last computed to, or that it has none yet: a workbook written by a
/// program rather than a spreadsheet app has no results until one opens it.
fn with_formula(formula: &str, value: Option<&Data>) -> String {
    match value {
        None | Some(Data::Empty) => format!("{formula} → not computed yet"),
        Some(Data::String(text)) if text.is_empty() => format!("{formula} → not computed yet"),
        Some(value) => format!("{formula} → {}", shown(value)),
    }
}

/// A value as a person reads it: numbers without binary noise, dates as `2026-01-31`.
fn shown(value: &Data) -> String {
    match value {
        Data::Float(number) => general(*number),
        Data::DateTime(when) if when.is_duration() => {
            let seconds = (when.as_f64() * 86_400.0).round();
            let (hours, minutes) = (
                (seconds / 3600.0).trunc(),
                (seconds % 3600.0 / 60.0).trunc(),
            );
            format!("{hours:.0}:{minutes:02.0}:{:02.0}", seconds % 60.0)
        }
        Data::DateTime(when) => {
            let (year, month, day, hour, minute, second, _) = when.to_ymd_hms_milli();
            if (hour, minute, second) == (0, 0, 0) {
                format!("{year:04}-{month:02}-{day:02}")
            } else {
                format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
            }
        }
        other => other.to_string(),
    }
}

/// A number as a spreadsheet's General format shows it: at most 15 significant digits, so
/// `0.1 + 0.2` reads `0.3`.
fn general(number: f64) -> String {
    let rounded: f64 = format!("{number:.14e}").parse().unwrap_or(number);
    format!("{rounded}")
}

/// A cell's text safe inside a table row.
fn escaped(text: &str) -> String {
    text.replace('|', "\\|")
        .replace('\r', "")
        .replace('\n', "\\n")
}

/// A field of a CSV line, quoted when it must be (RFC 4180).
fn quoted(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

/// A sheet's name as one shell word, for the command that reads more of it.
fn argument(sheet: &str) -> String {
    if sheet
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    {
        sheet.to_owned()
    } else {
        format!("'{}'", sheet.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_as_a_spreadsheet_shows_them() {
        assert_eq!(general(0.1 + 0.2), "0.3");
        assert_eq!(general(2401.25), "2401.25");
        assert_eq!(general(1950.0), "1950");
        assert_eq!(general(-0.000_012_5), "-0.0000125");
        assert_eq!(general(1e20), "100000000000000000000");
    }

    #[test]
    fn formulas_past_the_values_are_read_and_shown_as_excel_shows_them() {
        // As openpyxl saves a sheet: formulas with no values at all, past the last value.
        let mut values = calamine::Range::new((0, 0), (0, 0));
        values.set_value((0, 0), Data::String("Hello".into()));
        let mut formulas = calamine::Range::new((0, 1), (1, 1));
        formulas.set_value((0, 1), "_xlfn.XLOOKUP(A1,A1:A2,A1:A2)".into());
        formulas.set_value((1, 1), "1+1".into());
        let mut text = String::new();
        write_sheet(&mut text, "a.xlsx", "Sheet", &values, &formulas, None);
        assert_eq!(
            text,
            "## Sheet (A1:B2)\n| | A | B |\n|---|---|---|\n\
             | 1 | Hello | =XLOOKUP(A1,A1:A2,A1:A2) → not computed yet |\n\
             | 2 |  | =1+1 → not computed yet |\n"
        );
    }

    #[test]
    fn cells_and_fields_are_kept_whole() {
        assert_eq!(escaped("a|b\nc"), "a\\|b\\nc");
        assert_eq!(quoted("plain"), "plain");
        assert_eq!(quoted("a, \"b\""), "\"a, \"\"b\"\"\"");
        assert_eq!(argument("Q1"), "Q1");
        assert_eq!(argument("Q1 sales"), "'Q1 sales'");
        assert_eq!(argument("it's"), "'it'\\''s'");
    }
}
