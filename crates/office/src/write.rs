//! Writing a new workbook with rust_xlsxwriter from a description a model writes as JSON
//! (`description.rs`): sheets of rows, with formulas, number formats, a header row, column
//! widths, frozen panes and native charts. No formula's result is stored: an empty one makes
//! the app that opens the file work every formula out, and reads back as not computed yet.

mod description;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rust_xlsxwriter::{Chart, ExcelDateTime, Format, FormatBorder, Workbook, Worksheet, XlsxError};
use serde_json::Value;

use crate::cells::{Cell, Range, SheetRange, column_number, listed};
use crate::error::OfficeError;
use description::{ChartDescription, Description, SheetDescription};

/// A new file: its bytes, and what it holds in words.
pub(crate) struct Written {
    pub(crate) bytes: Vec<u8>,
    /// A workbook's sheets with their rows, formulas and charts, then what to know about
    /// them; a PDF's pages, or the fields filled.
    pub(crate) report: String,
}

/// The cells of text that read as numbers named in the report, of however many there are.
const TEXTS_NAMED: usize = 3;

/// What the sheets hold, counted as they are written.
#[derive(Debug, Default)]
struct Tally {
    formulas: usize,
    /// Every cell of text that reads as a number, which no formula adds up.
    texts_as_numbers: usize,
    /// The first few of those, as `Orders!E2 "1250.00"`.
    named: Vec<String>,
}

/// The workbook `json` describes.
pub(crate) fn write(json: &str) -> Result<Written, OfficeError> {
    let description: Description = serde_json::from_str(json).map_err(|error| {
        OfficeError::Description(format!("it is not the JSON glave expects: {error}"))
    })?;
    if description.sheets.is_empty() {
        return Err(OfficeError::Description(
            "it has no sheets: give \"sheets\" at least one".to_owned(),
        ));
    }
    let names: Vec<&str> = description
        .sheets
        .iter()
        .map(|sheet| sheet.name.as_str())
        .collect();
    let mut book = Workbook::new();
    let mut tally = Tally::default();
    let mut sheets = Vec::new();
    for sheet in &description.sheets {
        let before = tally.formulas;
        write_sheet(book.add_worksheet(), sheet, &names, &mut tally).map_err(|why| {
            OfficeError::Description(format!("in the sheet {:?}, {why}", sheet.name))
        })?;
        sheets.push(contents(sheet, tally.formulas - before));
    }
    let bytes = book
        .save_to_buffer()
        .map_err(|error| OfficeError::Description(error.to_string()))?;
    Ok(Written {
        bytes,
        report: report(&sheets, &tally),
    })
}

/// What the sheets hold, then what to know about it.
fn report(sheets: &[String], tally: &Tally) -> String {
    let mut report = format!("{}.", sheets.join(", "));
    if tally.formulas > 0 {
        report.push_str(
            "\nIts formulas have no results until a spreadsheet app opens the file and works \
             them out, so glave doc read shows each as not computed yet.",
        );
    }
    if tally.texts_as_numbers > 0 {
        let _ = write!(
            report,
            "\nNumbers written as text, which formulas don't add up: {}. Write numbers \
             without quotes unless they are labels.",
            listed(&tally.named, tally.texts_as_numbers)
        );
    }
    report
}

/// A sheet's name with what it holds: `Summary (4 rows, 3 formulas, 1 chart)`.
fn contents(sheet: &SheetDescription, formulas: usize) -> String {
    let mut held = vec![counted(sheet.rows.len(), "row", "rows")];
    if formulas > 0 {
        held.push(counted(formulas, "formula", "formulas"));
    }
    if !sheet.charts.is_empty() {
        held.push(counted(sheet.charts.len(), "chart", "charts"));
    }
    format!("{} ({})", sheet.name, held.join(", "))
}

fn counted(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn write_sheet(
    worksheet: &mut Worksheet,
    sheet: &SheetDescription,
    names: &[&str],
    tally: &mut Tally,
) -> Result<(), String> {
    said(worksheet.set_name(&sheet.name))?;
    worksheet.set_formula_result_default("");
    let formats = number_formats(&sheet.formats)?;
    let mut widest = 0u16;
    for (r, cells) in sheet.rows.iter().enumerate() {
        let row = u32::try_from(r).map_err(|_| too_many("rows"))?;
        let header = sheet.header && r == 0;
        for (c, value) in cells.iter().enumerate() {
            let column = u16::try_from(c).map_err(|_| too_many("cells in a row"))?;
            widest = widest.max(column);
            let at = Cell { row, column };
            // Where ranges overlap, the smallest is the most particular, and wins.
            let number = formats
                .iter()
                .filter(|(range, _)| range.contains(at))
                .min_by_key(|(range, _)| range.cells())
                .map(|(_, format)| format.as_str());
            let format = look(header, number, value);
            let wrote = write_cell(worksheet, at, value, format.as_ref())
                .map_err(|why| format!("{at}: {why}"))?;
            match wrote {
                Wrote::Formula => tally.formulas += 1,
                Wrote::TextAsNumber(text) if !header => {
                    tally.texts_as_numbers += 1;
                    if tally.named.len() < TEXTS_NAMED {
                        tally.named.push(format!("{}!{at} {text:?}", sheet.name));
                    }
                }
                Wrote::TextAsNumber(_) | Wrote::Other => {}
            }
        }
    }
    let last_row = u32::try_from(sheet.rows.len().saturating_sub(1)).unwrap_or(u32::MAX);
    if sheet.header && !sheet.rows.is_empty() {
        said(worksheet.autofilter(0, 0, last_row, widest))?;
    }
    let freeze = match (&sheet.freeze, sheet.header) {
        (Some(cell), _) => Some(Cell::parse(cell).ok_or_else(|| {
            format!("freeze {cell:?} is not a cell like B2, the first cell that scrolls")
        })?),
        (None, true) => Some(Cell { row: 1, column: 0 }),
        (None, false) => None,
    };
    if let Some(Cell { row, column }) = freeze {
        said(worksheet.set_freeze_panes(row, column))?;
    }
    worksheet.autofit();
    for (letters, width) in &sheet.widths {
        let column = column_number(letters)
            .ok_or_else(|| format!("widths: {letters:?} is not a column like A or AB"))?;
        said(worksheet.set_column_width(column, *width))?;
    }
    for chart in &sheet.charts {
        insert_chart(worksheet, &sheet.name, names, chart)
            .map_err(|why| format!("the chart at {}: {why}", chart.at))?;
    }
    Ok(())
}

fn too_many(what: &str) -> String {
    format!("it has more {what} than a sheet holds")
}

/// What rust_xlsxwriter says went wrong, in its own words, which name what Excel allows.
fn said<T>(result: Result<T, XlsxError>) -> Result<T, String> {
    result.map_err(|error| error.to_string())
}

/// The number formats by range.
fn number_formats(formats: &BTreeMap<String, String>) -> Result<Vec<(Range, String)>, String> {
    formats
        .iter()
        .map(|(range, format)| {
            Range::parse(range)
                .map(|parsed| (parsed, format.clone()))
                .ok_or_else(|| format!("formats: {range:?} is not a range like C2:D13"))
        })
        .collect()
}

/// How a cell looks: bold over a line when it heads the sheet, and its number format, which a
/// date has of its own when none is given.
fn look(header: bool, number: Option<&str>, value: &Value) -> Option<Format> {
    let date = match value {
        Value::Object(map) => map.get("date").and_then(Value::as_str).map(date_format),
        _ => None,
    };
    let number = number.or(date);
    if !header && number.is_none() {
        return None;
    }
    let mut format = Format::new();
    if header {
        format = format.set_bold().set_border_bottom(FormatBorder::Thin);
    }
    if let Some(number) = number {
        format = format.set_num_format(number);
    }
    Some(format)
}

/// The format a date shows in unless the sheet gives one: with its time when it has one.
fn date_format(date: &str) -> &'static str {
    match (date.contains('-'), date.contains(':')) {
        (true, true) => "yyyy-mm-dd hh:mm:ss",
        (false, true) => "hh:mm:ss",
        _ => "yyyy-mm-dd",
    }
}

/// What a cell was written as, for the report.
enum Wrote {
    Formula,
    /// Text that reads as a number, kept as text as the description gives it.
    TextAsNumber(String),
    Other,
}

/// One cell: a number, text, true or false, a formula (text starting with `=`), a date
/// (`{"date": "2026-01-31"}`), or empty (`null`), which is left out unless it has a look.
fn write_cell(
    worksheet: &mut Worksheet,
    Cell { row, column }: Cell,
    value: &Value,
    format: Option<&Format>,
) -> Result<Wrote, String> {
    let plain = Format::new();
    let look = format.unwrap_or(&plain);
    let written = match value {
        Value::Null => match format {
            Some(format) => worksheet.write_blank(row, column, format),
            None => return Ok(Wrote::Other),
        },
        Value::Bool(yes) => worksheet.write_boolean_with_format(row, column, *yes, look),
        Value::Number(number) => {
            let number = number.as_f64().ok_or("the number is too large")?;
            worksheet.write_number_with_format(row, column, number, look)
        }
        Value::String(text) if text.len() > 1 && text.starts_with('=') => {
            said(worksheet.write_formula_with_format(row, column, text.as_str(), look))?;
            return Ok(Wrote::Formula);
        }
        Value::String(text) => {
            said(worksheet.write_string_with_format(row, column, text, look))?;
            let number = reads_as_number(text).then(|| Wrote::TextAsNumber(text.clone()));
            return Ok(number.unwrap_or(Wrote::Other));
        }
        Value::Object(map) if map.len() == 1 && map.contains_key("date") => {
            let date = map.get("date").and_then(Value::as_str).unwrap_or_default();
            let when = ExcelDateTime::parse_from_str(date).map_err(|_| {
                format!("{date:?} is not a date like 2026-01-31 or 2026-01-31 14:30")
            })?;
            worksheet.write_datetime_with_format(row, column, when, look)
        }
        Value::Object(_) | Value::Array(_) => {
            return Err(
                "a cell is a number, text, true, false, null, a formula like \
                 \"=B2*C2\" or a date like {\"date\": \"2026-01-31\"}"
                    .to_owned(),
            );
        }
    };
    said(written)?;
    Ok(Wrote::Other)
}

/// Whether text reads as a number a person meant as one: `1250.00` does, while `007` and
/// `1e5` read as codes.
fn reads_as_number(text: &str) -> bool {
    let digits = text.trim_start_matches(['-', '+']);
    let leading_zero = digits.len() > 1 && digits.starts_with('0') && !digits.starts_with("0.");
    !leading_zero
        && digits.starts_with(|c: char| c.is_ascii_digit() || c == '.')
        && digits.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && text.parse::<f64>().is_ok()
}

fn insert_chart(
    worksheet: &mut Worksheet,
    sheet: &str,
    names: &[&str],
    description: &ChartDescription,
) -> Result<(), String> {
    let at = Cell::parse(&description.at)
        .ok_or_else(|| "\"at\" is not a cell like F2, where its top left corner sits".to_owned())?;
    if description.values.is_empty() {
        return Err("\"values\" needs a range for each series, like [\"B2:B13\"]".to_owned());
    }
    let mut chart = Chart::new(description.kind.chart_type());
    if let Some(title) = &description.title {
        chart.title().set_name(title);
    }
    if let Some(title) = &description.x_title {
        chart.x_axis().set_name(title);
    }
    if let Some(title) = &description.y_title {
        chart.y_axis().set_name(title);
    }
    let categories = match &description.categories {
        Some(range) => Some(on_sheet(range, sheet, names, "categories")?),
        None => None,
    };
    for (i, range) in description.values.iter().enumerate() {
        let (on, range) = on_sheet(range, sheet, names, "values")?;
        let series = chart.add_series();
        series.set_values(ranged(on, range));
        if let Some((on, range)) = categories {
            series.set_categories(ranged(on, range));
        }
        if let Some(name) = description.names.get(i) {
            series.set_name(name);
        }
    }
    // One series needs no legend, and an unnamed one would read "Series1".
    if description.values.len() == 1 && description.names.is_empty() {
        chart.legend().set_hidden();
    }
    said(worksheet.insert_chart(at.row, at.column, &chart))?;
    Ok(())
}

/// A chart's range and the sheet it is on, as the workbook names it: the chart's own unless the
/// range names another.
fn on_sheet<'a>(
    text: &str,
    sheet: &'a str,
    names: &[&'a str],
    what: &str,
) -> Result<(&'a str, Range), String> {
    let parsed = SheetRange::parse(text)
        .ok_or_else(|| format!("{what} {text:?} is not a range like B2:B13 or Sales!B2:B13"))?;
    let Some(wanted) = parsed.sheet else {
        return Ok((sheet, parsed.range));
    };
    let found = names
        .iter()
        // Excel tells sheets apart without their case.
        .find(|name| name.to_lowercase() == wanted.to_lowercase())
        .ok_or_else(|| {
            format!(
                "{what} {text:?} is on a sheet the workbook doesn't have; its sheets are {}",
                names.join(", ")
            )
        })?;
    Ok((found, parsed.range))
}

fn ranged(sheet: &str, range: Range) -> (&str, u32, u16, u32, u16) {
    (
        sheet,
        range.first.row,
        range.first.column,
        range.last.row,
        range.last.column,
    )
}

#[cfg(test)]
mod tests;
