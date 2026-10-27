//! Reading a workbook (xlsx, xlsm, xlsb, xls or ods, told by its bytes) with calamine: each
//! sheet as a table a model reads, with the row numbers and column letters to name a cell by,
//! every formula beside the value it last computed to; or one sheet as CSV.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Cursor;

use calamine::{Data, Range, Reader, Sheet, SheetType, SheetVisible, Sheets};

use crate::cells::{cell_name, column_name, listed};
use crate::command::Answer;
use crate::error::OfficeError;
use crate::table::escaped;

/// The rows shown of each sheet when no rows are asked for: enough to see what a sheet holds,
/// little enough that a workbook of many sheets stays readable.
const ROWS_SHOWN: u32 = 100;

/// The columns shown of a sheet: a row wider than this is better read as CSV.
const COLUMNS_SHOWN: u32 = 50;

/// Which part of a workbook to read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Reading {
    /// Only this sheet; every sheet when `None`.
    pub(crate) sheet: Option<String>,
    /// Only these rows, numbered from 1 as a person reads them; the first [`ROWS_SHOWN`] when
    /// `None`.
    pub(crate) rows: Option<(u32, u32)>,
}

/// A cell's row and column, counted from 0 as calamine counts them.
type At = (u32, u32);

type Book = Sheets<Cursor<Vec<u8>>>;

pub(crate) fn open(bytes: Vec<u8>) -> Result<Book, OfficeError> {
    calamine::open_workbook_auto_from_rs(Cursor::new(bytes))
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
        // A sheet's formulas are read apart from its values; a kind of workbook that keeps
        // none reads as having none.
        let formulas = book.worksheet_formula(&sheet.name).unwrap_or_default();
        text.push('\n');
        let table = Table {
            file: name,
            sheet: &sheet.name,
            values: &values,
            formulas: &formulas,
        };
        table.write(&mut text, reading.rows);
    }
    Ok(text)
}

/// One sheet of the workbook in `bytes` as CSV: the first when `sheet` is `None`. Each formula
/// gives the value it last computed to, and one no app has computed is warned of, apart from
/// the CSV; dates are written as `2026-01-31`.
pub(crate) fn csv(bytes: Vec<u8>, sheet: Option<&str>) -> Result<Answer, OfficeError> {
    let mut book = open(bytes)?;
    let sheets: Vec<Sheet> = book.sheets_metadata().to_vec();
    let name = chosen(&sheets, sheet)?
        .first()
        .map(|chosen| chosen.name.clone())
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
    let formulas = book.worksheet_formula(&name).unwrap_or_default();
    let uncomputed: Vec<String> = formulas_by_cell(&formulas)
        .into_keys()
        .filter(|at| computed(values.get_value(*at)).is_none())
        .map(|(row, column)| cell_name(row, column))
        .collect();
    Ok(Answer {
        printed: text,
        warning: not_computed(&name, &uncomputed),
    })
}

/// The cells shown by name in a warning, of however many there are.
const CELLS_NAMED: usize = 5;

/// That the formulas in `cells` of `sheet` have no values yet, so their fields are empty.
fn not_computed(sheet: &str, cells: &[String]) -> Option<String> {
    let first = cells.first()?;
    let mut named = vec![format!("{sheet}!{first}")];
    named.extend(cells.iter().skip(1).take(CELLS_NAMED - 1).cloned());
    Some(format!(
        "formulas not computed yet, so their fields are empty: {}. No spreadsheet app has \
         opened the file since a program wrote it.",
        listed(&named, cells.len())
    ))
}

/// The worksheets to read: the one named, or every worksheet (charts and macros hold no cells).
pub(crate) fn chosen<'a>(
    sheets: &'a [Sheet],
    wanted: Option<&str>,
) -> Result<Vec<&'a Sheet>, OfficeError> {
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

/// One sheet, to write as a table.
struct Table<'a> {
    file: &'a str,
    sheet: &'a str,
    values: &'a Range<Data>,
    formulas: &'a Range<String>,
}

impl Table<'_> {
    /// A header of column letters, then each of `rows` (the first [`ROWS_SHOWN`] when `None`)
    /// with its number, then how to read what was left out.
    fn write(&self, text: &mut String, rows: Option<(u32, u32)>) {
        let sheet = self.sheet;
        let Some((first, last)) = self.used() else {
            let _ = writeln!(text, "## {sheet}: empty");
            return;
        };
        let span = if first == last {
            name(first)
        } else {
            format!("{}:{}", name(first), name(last))
        };
        let _ = writeln!(text, "## {sheet} ({span})");
        let formulas = formulas_by_cell(self.formulas);
        let (from, to) = rows.unwrap_or((first.0 + 1, first.0 + ROWS_SHOWN));
        let (from, to) = (from.max(first.0 + 1), to.min(last.0 + 1));
        let last_shown = last.1.min(first.1 + COLUMNS_SHOWN - 1);
        let columns = first.1..=last_shown;
        let header: Vec<String> = columns.clone().map(column_name).collect();
        let _ = writeln!(text, "| | {} |", header.join(" | "));
        let _ = writeln!(text, "|---|{}", "---|".repeat(header.len()));
        for row in from.saturating_sub(1)..to {
            let cells: Vec<String> = columns
                .clone()
                .map(|column| {
                    let value = self.values.get_value((row, column));
                    let shown = match formulas.get(&(row, column)) {
                        Some(formula) => with_formula(formula, value),
                        None => value.map(shown).unwrap_or_default(),
                    };
                    escaped(&shown)
                })
                .collect();
            let _ = writeln!(text, "| {} | {} |", row + 1, cells.join(" | "));
        }
        let (file, quoted) = (self.file, argument(sheet));
        if last_shown < last.1 {
            let (next, end) = (column_name(last_shown + 1), column_name(last.1));
            let _ = writeln!(
                text,
                "… columns {next} to {end} not shown: glave doc csv {file} --sheet {quoted}"
            );
        }
        let end = last.0 + 1;
        if to < end {
            let (next, until) = (to + 1, to.saturating_add(ROWS_SHOWN).min(end));
            let _ = writeln!(
                text,
                "… rows {next} to {end} not shown: \
                 glave doc read {file} --sheet {quoted} --rows {next}-{until}"
            );
        }
    }

    /// The first and last cells the sheet uses: those with values, and those with formulas,
    /// which have no value until an app computes them.
    fn used(&self) -> Option<(At, At)> {
        let first = [self.values.start(), self.formulas.start()]
            .into_iter()
            .flatten()
            .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1)))?;
        let last = [self.values.end(), self.formulas.end()]
            .into_iter()
            .flatten()
            .reduce(|a, b| (a.0.max(b.0), a.1.max(b.1)))?;
        Some((first, last))
    }
}

/// Every formula in a sheet by its cell, with its `=`, which calamine leaves off.
fn formulas_by_cell(formulas: &Range<String>) -> BTreeMap<At, String> {
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
            Some(((row, column), format!("={formula}")))
        })
        .collect()
}

/// A cell's name: `A1` for (0, 0).
fn name((row, column): At) -> String {
    cell_name(row, column)
}

/// A formula's value, unless it has none yet: a workbook written by a program rather than a
/// spreadsheet app (as `glave doc write` writes one) has no results until one opens it.
fn computed(value: Option<&Data>) -> Option<&Data> {
    match value {
        None | Some(Data::Empty) => None,
        Some(Data::String(text)) if text.is_empty() => None,
        value => value,
    }
}

/// A formula and the value it last computed to, or that it has none yet.
fn with_formula(formula: &str, value: Option<&Data>) -> String {
    match computed(value) {
        Some(value) => format!("{formula} → {}", shown(value)),
        None => format!("{formula} → not computed yet"),
    }
}

/// A value as a person reads it: numbers without binary noise, dates as `2026-01-31`.
pub(crate) fn shown(value: &Data) -> String {
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
pub(crate) fn general(number: f64) -> String {
    let rounded: f64 = format!("{number:.14e}").parse().unwrap_or(number);
    format!("{rounded}")
}

/// A field of a CSV line, quoted when it must be (RFC 4180).
pub(crate) fn quoted(field: &str) -> String {
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
    fn formulas_with_no_values_are_warned_of_by_their_cells() {
        let cells = |n: u32| (1..=n).map(|row| cell_name(row, 1)).collect::<Vec<_>>();
        assert_eq!(not_computed("Summary", &[]), None);
        assert_eq!(
            not_computed("Summary", &cells(1)).unwrap(),
            "formulas not computed yet, so their fields are empty: Summary!B2. No spreadsheet \
             app has opened the file since a program wrote it."
        );
        let many = not_computed("Summary", &cells(8)).unwrap();
        assert!(
            many.contains(": Summary!B2, B3, B4, B5, B6 and 3 more. "),
            "{many}"
        );
    }

    #[test]
    fn formulas_past_the_values_are_read_and_shown_as_excel_shows_them() {
        // As openpyxl saves a sheet: formulas with no values at all, past the last value.
        let mut values = Range::new((0, 0), (0, 0));
        values.set_value((0, 0), Data::String("Hello".into()));
        let mut formulas = Range::new((0, 1), (1, 1));
        formulas.set_value((0, 1), "_xlfn.XLOOKUP(A1,A1:A2,A1:A2)".into());
        formulas.set_value((1, 1), "1+1".into());
        let table = Table {
            file: "a.xlsx",
            sheet: "Sheet",
            values: &values,
            formulas: &formulas,
        };
        let mut text = String::new();
        table.write(&mut text, None);
        assert_eq!(
            text,
            "## Sheet (A1:B2)\n| | A | B |\n|---|---|---|\n\
             | 1 | Hello | =XLOOKUP(A1,A1:A2,A1:A2) → not computed yet |\n\
             | 2 |  | =1+1 → not computed yet |\n"
        );
    }

    #[test]
    fn cells_and_fields_are_kept_whole() {
        assert_eq!(quoted("plain"), "plain");
        assert_eq!(quoted("a, \"b\""), "\"a, \"\"b\"\"\"");
        assert_eq!(argument("Q1"), "Q1");
        assert_eq!(argument("Q1 sales"), "'Q1 sales'");
        assert_eq!(argument("it's"), "'it'\\''s'");
    }
}
