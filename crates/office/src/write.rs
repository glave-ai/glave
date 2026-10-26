//! Writing a new workbook with rust_xlsxwriter from a description a model writes as JSON: its
//! sheets, each with rows of values and formulas, number formats, bold cells, a header row,
//! column widths, frozen panes and native charts; and the workbook's defined names.

mod spec;

use std::collections::BTreeMap;

use rust_xlsxwriter::{Chart, ExcelDateTime, Format, FormatBorder, Workbook, Worksheet, XlsxError};
use serde_json::Value;

use crate::cells::{Cell, Range, SheetRange, column_number};
use crate::error::OfficeError;
use spec::{ChartSpec, SheetSpec, Spec};

/// The workbook `json` describes, as the bytes of an .xlsx file.
pub(crate) fn write(json: &str) -> Result<Vec<u8>, OfficeError> {
    let spec: Spec =
        serde_json::from_str(json).map_err(|error| OfficeError::Description(error.to_string()))?;
    if spec.sheets.is_empty() {
        return Err(OfficeError::Description(
            "it names no sheets: give \"sheets\" at least one".to_owned(),
        ));
    }
    let mut book = Workbook::new();
    for sheet in &spec.sheets {
        let worksheet = book.add_worksheet();
        write_sheet(worksheet, sheet)
            .map_err(|why| OfficeError::InSheet(sheet.name.clone(), why))?;
    }
    for (name, refers) in &spec.names {
        let refers = if refers.starts_with('=') {
            refers.clone()
        } else {
            format!("={refers}")
        };
        book.define_name(name, &refers)
            .map_err(|error| OfficeError::Description(format!("the name {name}: {error}")))?;
    }
    book.save_to_buffer()
        .map_err(|error| OfficeError::Description(error.to_string()))
}

/// How one cell looks, from everything the sheet says about it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Look {
    header: bool,
    bold: bool,
    number: Option<String>,
    date: bool,
}

impl Look {
    fn format(&self) -> Option<Format> {
        if *self == Self::default() {
            return None;
        }
        let mut format = Format::new();
        if self.header || self.bold {
            format = format.set_bold();
        }
        if self.header {
            format = format
                .set_background_color("D9E1F2")
                .set_border_bottom(FormatBorder::Thin);
        }
        match (&self.number, self.date) {
            (Some(number), _) => format = format.set_num_format(number),
            (None, true) => format = format.set_num_format("yyyy-mm-dd"),
            (None, false) => {}
        }
        Some(format)
    }
}

fn write_sheet(worksheet: &mut Worksheet, sheet: &SheetSpec) -> Result<(), String> {
    worksheet
        .set_name(&sheet.name)
        .map_err(|error| error.to_string())?;
    // Results are worked out by the spreadsheet app that opens the file; an empty one makes
    // every app work them out, LibreOffice too, and reads back as not computed yet.
    worksheet.set_formula_result_default("");
    let start = match &sheet.start {
        Some(start) => {
            Cell::parse(start).ok_or_else(|| format!("start {start:?} is not a cell"))?
        }
        None => Cell { row: 0, column: 0 },
    };
    let numbers = ranges(&sheet.formats)?;
    let bold: Vec<Range> = sheet
        .bold
        .iter()
        .map(|range| Range::parse(range).ok_or_else(|| format!("bold {range:?} is not a range")))
        .collect::<Result<_, _>>()?;
    let mut widest = 0u16;
    for (r, values) in sheet.rows.iter().enumerate() {
        let row = start.row + u32::try_from(r).map_err(|_| "too many rows".to_owned())?;
        for (c, value) in values.iter().enumerate() {
            let column =
                start.column + u16::try_from(c).map_err(|_| "too many columns".to_owned())?;
            widest = widest.max(column);
            let at = Cell { row, column };
            let look = Look {
                header: sheet.header && r == 0,
                bold: bold.iter().any(|range| range.contains(at)),
                // Where ranges overlap, the smallest is the most particular, and wins.
                number: numbers
                    .iter()
                    .filter(|(range, _)| range.contains(at))
                    .min_by_key(|(range, _)| range.cells())
                    .map(|(_, format)| format.clone()),
                date: matches!(value, Value::Object(map) if map.contains_key("date")),
            };
            write_cell(worksheet, at, value, look.format().as_ref())
                .map_err(|why| format!("{at}: {why}"))?;
        }
    }
    let last_row = start.row + u32::try_from(sheet.rows.len()).unwrap_or(u32::MAX).max(1) - 1;
    if sheet.header && !sheet.rows.is_empty() {
        worksheet
            .autofilter(start.row, start.column, last_row, widest.max(start.column))
            .map_err(|error| error.to_string())?;
    }
    let freeze = match (&sheet.freeze, sheet.header) {
        (Some(cell), _) => {
            Some(Cell::parse(cell).ok_or_else(|| format!("freeze {cell:?} is not a cell"))?)
        }
        (None, true) => Some(Cell {
            row: start.row + 1,
            column: 0,
        }),
        (None, false) => None,
    };
    if let Some(freeze) = freeze {
        worksheet
            .set_freeze_panes(freeze.row, freeze.column)
            .map_err(|error| error.to_string())?;
    }
    worksheet.autofit();
    for (letters, width) in &sheet.widths {
        let column =
            column_number(letters).ok_or_else(|| format!("widths: {letters:?} is not a column"))?;
        worksheet
            .set_column_width(column, *width)
            .map_err(|error| error.to_string())?;
    }
    for chart in &sheet.charts {
        insert_chart(worksheet, &sheet.name, chart)?;
    }
    Ok(())
}

/// The number formats by range.
fn ranges(formats: &BTreeMap<String, String>) -> Result<Vec<(Range, String)>, String> {
    formats
        .iter()
        .map(|(range, format)| {
            Range::parse(range)
                .map(|parsed| (parsed, format.clone()))
                .ok_or_else(|| format!("formats: {range:?} is not a range"))
        })
        .collect()
}

/// One value: a number, text, true or false, a formula (text starting with `=`), a date
/// (`{"date": "2026-01-31"}`) or text that starts with `=` (`{"text": "=not a formula"}`).
fn write_cell(
    worksheet: &mut Worksheet,
    at: Cell,
    value: &Value,
    format: Option<&Format>,
) -> Result<(), String> {
    let Cell { row, column } = at;
    let written: Result<&mut Worksheet, XlsxError> = match (value, format) {
        (Value::Null, None) => return Ok(()),
        (Value::Null, Some(format)) => worksheet.write_blank(row, column, format),
        (Value::Bool(yes), None) => worksheet.write_boolean(row, column, *yes),
        (Value::Bool(yes), Some(format)) => {
            worksheet.write_boolean_with_format(row, column, *yes, format)
        }
        (Value::Number(number), format) => {
            let number = number.as_f64().ok_or("the number is out of range")?;
            match format {
                Some(format) => worksheet.write_number_with_format(row, column, number, format),
                None => worksheet.write_number(row, column, number),
            }
        }
        (Value::String(text), format) if text.starts_with('=') && text.len() > 1 => match format {
            Some(format) => worksheet.write_formula_with_format(row, column, text.as_str(), format),
            None => worksheet.write_formula(row, column, text.as_str()),
        },
        (Value::String(text), format) => write_text(worksheet, at, text, format),
        (Value::Object(map), format) => {
            if let Some(Value::String(date)) = map.get("date") {
                let when = ExcelDateTime::parse_from_str(date)
                    .map_err(|_| format!("{date:?} is not a date like 2026-01-31"))?;
                let date_format = Format::new().set_num_format("yyyy-mm-dd");
                worksheet.write_datetime_with_format(
                    row,
                    column,
                    when,
                    format.unwrap_or(&date_format),
                )
            } else if let Some(Value::String(text)) = map.get("text") {
                write_text(worksheet, at, text, format)
            } else {
                return Err(
                    "an object cell is {\"date\": \"2026-01-31\"} or {\"text\": \"…\"}".to_owned(),
                );
            }
        }
        (Value::Array(_), _) => return Err("a cell holds one value, not a list".to_owned()),
    };
    written.map(|_| ()).map_err(|error| error.to_string())
}

fn write_text<'a>(
    worksheet: &'a mut Worksheet,
    Cell { row, column }: Cell,
    text: &str,
    format: Option<&Format>,
) -> Result<&'a mut Worksheet, XlsxError> {
    match format {
        Some(format) => worksheet.write_string_with_format(row, column, text, format),
        None => worksheet.write_string(row, column, text),
    }
}

fn insert_chart(worksheet: &mut Worksheet, sheet: &str, spec: &ChartSpec) -> Result<(), String> {
    let at =
        Cell::parse(&spec.at).ok_or_else(|| format!("chart at {:?} is not a cell", spec.at))?;
    if spec.values.is_empty() {
        return Err("a chart needs at least one range in \"values\"".to_owned());
    }
    let mut chart = Chart::new(spec.kind.chart_type());
    if let Some(title) = &spec.title {
        chart.title().set_name(title);
    }
    if let Some(title) = &spec.x_title {
        chart.x_axis().set_name(title);
    }
    if let Some(title) = &spec.y_title {
        chart.y_axis().set_name(title);
    }
    let categories = match &spec.categories {
        Some(range) => Some(on_sheet(range, sheet, "categories")?),
        None => None,
    };
    for (i, range) in spec.values.iter().enumerate() {
        let (on, range) = on_sheet(range, sheet, "values")?;
        let series = chart.add_series();
        series.set_values((
            on.as_str(),
            range.first.row,
            range.first.column,
            range.last.row,
            range.last.column,
        ));
        if let Some((on, range)) = &categories {
            series.set_categories((
                on.as_str(),
                range.first.row,
                range.first.column,
                range.last.row,
                range.last.column,
            ));
        }
        if let Some(name) = spec.names.get(i) {
            series.set_name(name);
        }
    }
    if spec.names.is_empty() && spec.values.len() == 1 {
        chart.legend().set_hidden();
    }
    worksheet
        .insert_chart(at.row, at.column, &chart)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// A chart's range and the sheet it is on: the chart's own unless it names another.
fn on_sheet(text: &str, sheet: &str, what: &str) -> Result<(String, Range), String> {
    let parsed =
        SheetRange::parse(text).ok_or_else(|| format!("chart {what} {text:?} is not a range"))?;
    Ok((
        parsed.sheet.unwrap_or_else(|| sheet.to_owned()),
        parsed.range,
    ))
}
