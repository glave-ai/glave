//! Files read as tables to load: a CSV (or TSV) a table, a JSON array of objects a table, a
//! workbook a table for each worksheet, or for the one `--sheet` names.

use std::path::{Path, PathBuf};

use calamine::{Data, Reader};
use rusqlite::types::Value;

use super::table::{Table, table_name, value};
use super::{csv, json};
use crate::command::{bytes, named};
use crate::error::OfficeError;
use crate::read;

/// What `glave doc sql` loads: CSV, TSV, JSON, and every kind of workbook `read` reads.
pub(crate) const EXTENSIONS: [&str; 8] =
    ["csv", "tsv", "json", "xlsx", "xlsm", "xlsb", "xls", "ods"];

/// A file to load, and the name its table is given, when one is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Source {
    pub(crate) name: Option<String>,
    pub(crate) path: PathBuf,
}

impl Source {
    /// `sales.csv`, or `sales=data/2026.csv` to name its table; `None` for a word that names
    /// no file `glave doc sql` loads.
    pub(crate) fn parse(word: &str) -> Option<Self> {
        if !EXTENSIONS
            .iter()
            .any(|extension| crate::command::ends_in(Path::new(word), extension))
        {
            return None;
        }
        let named = word.split_once('=').filter(|(name, _)| {
            name.chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        });
        Some(match named {
            Some((name, path)) => Self {
                name: Some(name.to_owned()),
                path: PathBuf::from(path),
            },
            None => Self {
                name: None,
                path: PathBuf::from(word),
            },
        })
    }

    /// The table's name: the one given, or the file's name without its extension.
    fn base(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            table_name(
                &self
                    .path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )
        })
    }

    fn is(&self, extensions: &[&str]) -> bool {
        extensions
            .iter()
            .any(|extension| crate::command::ends_in(&self.path, extension))
    }

    /// Whether it is a workbook.
    pub(crate) fn workbook(&self) -> bool {
        !self.is(&["csv", "tsv", "json"])
    }

    /// Its tables: for a workbook, the sheet named `sheet`, or every worksheet that holds
    /// anything.
    pub(crate) fn tables(&self, sheet: Option<&str>) -> Result<Vec<Table>, OfficeError> {
        let bytes = bytes(&self.path)?;
        let file = named(&self.path);
        if self.workbook() {
            return self.sheets(bytes, &file, sheet);
        }
        let text = String::from_utf8(bytes).map_err(|_| {
            OfficeError::Data(format!(
                "{file} is not text in UTF-8; save it again as CSV UTF-8 from the app that made it"
            ))
        })?;
        let rows = if self.is(&["json"]) {
            json::rows(&text, &file)?
        } else {
            csv::rows(&text, &file)?
                .into_iter()
                .map(|row| row.iter().map(|field| value(field)).collect())
                .collect()
        };
        Ok(vec![Table::headed(self.base(), file, rows, 0)])
    }

    /// A workbook's worksheets as tables: one alone named as the file is, several each as
    /// `<file>_<sheet>`.
    fn sheets(
        &self,
        bytes: Vec<u8>,
        file: &str,
        sheet: Option<&str>,
    ) -> Result<Vec<Table>, OfficeError> {
        let mut book = read::open(bytes)?;
        let sheets = book.sheets_metadata().to_vec();
        let chosen = read::chosen(&sheets, sheet)?;
        let mut tables = Vec::new();
        for sheet in &chosen {
            let range = book
                .worksheet_range(&sheet.name)
                .map_err(|error| OfficeError::Sheet(sheet.name.clone(), error.to_string()))?;
            let rows: Vec<Vec<Value>> = range
                .rows()
                .map(|row| row.iter().map(cell).collect::<Vec<_>>())
                .filter(|row| row.iter().any(|value| *value != Value::Null))
                .collect();
            if rows.is_empty() && chosen.len() > 1 {
                continue;
            }
            let first = range.start().map_or(0, |(_, column)| column);
            tables.push(Table::headed(
                sheet.name.clone(),
                format!("{file}, sheet {}", sheet.name),
                rows,
                first,
            ));
        }
        let base = self.base();
        if let [only] = tables.as_mut_slice() {
            only.name = base;
        } else {
            for table in &mut tables {
                table.name = format!("{base}_{}", table_name(&table.name));
            }
        }
        Ok(tables)
    }
}

/// A workbook's cell as SQLite holds it: whole numbers as integers, true and false as 1 and
/// 0, dates as `2026-01-31` (so SQLite's date functions read them), and an error (`#DIV/0!`)
/// as empty.
fn cell(data: &Data) -> Value {
    /// The largest whole number a float holds exactly.
    const EXACT: f64 = 9_007_199_254_740_992.0;
    match data {
        Data::Empty | Data::Error(_) => Value::Null,
        Data::String(text) if text.trim().is_empty() => Value::Null,
        Data::String(text) => Value::Text(text.clone()),
        Data::Int(whole) => Value::Integer(*whole),
        #[expect(
            clippy::cast_possible_truncation,
            reason = "whole, and within i64 as checked"
        )]
        Data::Float(number) if number.fract() == 0.0 && number.abs() < EXACT => {
            Value::Integer(*number as i64)
        }
        Data::Float(number) => Value::Real(*number),
        Data::Bool(yes) => Value::Integer(i64::from(*yes)),
        Data::DateTime(_) | Data::DateTimeIso(_) | Data::DurationIso(_) => {
            Value::Text(read::shown(data))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_named_as_given_or_after_themselves() {
        assert_eq!(
            Source::parse("data/sales-2026.csv"),
            Some(Source {
                name: None,
                path: "data/sales-2026.csv".into()
            })
        );
        assert_eq!(
            Source::parse("data/sales-2026.csv").map(|s| s.base()),
            Some("sales_2026".into())
        );
        let named = Source::parse("orders=data/2026.JSON").unwrap();
        assert_eq!(named.base(), "orders");
        assert_eq!(named.path, Path::new("data/2026.JSON"));
        assert!(!named.workbook());
        assert!(Source::parse("book.xlsx").unwrap().workbook());
        // A name that isn't one stays part of the path.
        assert_eq!(
            Source::parse("a b=c.csv").unwrap().path,
            Path::new("a b=c.csv")
        );
        assert_eq!(Source::parse("SELECT * FROM sales"), None);
        assert_eq!(Source::parse("notes.txt"), None);
    }

    #[test]
    fn cells_are_held_as_the_values_they_are() {
        assert_eq!(cell(&Data::Float(120.0)), Value::Integer(120));
        assert_eq!(cell(&Data::Float(19.5)), Value::Real(19.5));
        assert_eq!(cell(&Data::Float(1e300)), Value::Real(1e300));
        assert_eq!(cell(&Data::Bool(true)), Value::Integer(1));
        assert_eq!(cell(&Data::String(" ".into())), Value::Null);
        assert_eq!(
            cell(&Data::Error(calamine::CellErrorType::Div0)),
            Value::Null
        );
        assert_eq!(
            cell(&Data::DateTimeIso("2026-01-31".into())),
            Value::Text("2026-01-31".into())
        );
    }
}
