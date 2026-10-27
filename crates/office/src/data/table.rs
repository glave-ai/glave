//! A table to load: its name, its columns named from the header, and its rows of values, each
//! column given the plainest type that holds all of its values.

use rusqlite::types::Value;

use crate::cells::column_name;

/// A table read from a file, ready to load.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Table {
    pub(crate) name: String,
    /// The file it came from, and the sheet for a workbook's.
    pub(crate) from: String,
    pub(crate) columns: Vec<String>,
    pub(crate) rows: Vec<Vec<Value>>,
}

impl Table {
    /// A table named `name`, from `from`, whose first row is its header: an empty header takes
    /// its column's letter (the first column being `first`, counted from 0), and a name used before gets a number after it (`Region_2`), as
    /// SQLite tells column names apart without their case.
    pub(crate) fn headed(
        name: String,
        from: String,
        mut rows: Vec<Vec<Value>>,
        first: u32,
    ) -> Self {
        let header = if rows.is_empty() {
            Vec::new()
        } else {
            rows.remove(0)
        };
        let width = rows.iter().map(Vec::len).chain([header.len()]).max();
        let mut columns: Vec<String> = Vec::new();
        for index in 0..width.unwrap_or(0) {
            let named = match header.get(index) {
                Some(Value::Text(text)) if !text.trim().is_empty() => text.trim().to_owned(),
                Some(Value::Integer(number)) => number.to_string(),
                Some(Value::Real(number)) => crate::read::general(*number),
                _ => column_name(first.saturating_add(u32::try_from(index).unwrap_or(u32::MAX))),
            };
            let taken = |name: &str| columns.iter().any(|c| c.eq_ignore_ascii_case(name));
            let mut unique = named.clone();
            let mut count = 1;
            while taken(&unique) {
                count += 1;
                unique = format!("{named}_{count}");
            }
            columns.push(unique);
        }
        for row in &mut rows {
            row.resize(columns.len(), Value::Null);
        }
        Self {
            name,
            from,
            columns,
            rows,
        }
    }

    /// Each column's type: `INTEGER` when every value is a whole number, `REAL` when every
    /// one is a number, `TEXT` otherwise; empty values don't count.
    pub(crate) fn types(&self) -> Vec<&'static str> {
        (0..self.columns.len())
            .map(|column| {
                let values = self.rows.iter().filter_map(|row| row.get(column));
                let mut kind = "INTEGER";
                for value in values {
                    match value {
                        Value::Null | Value::Integer(_) => {}
                        Value::Real(_) => kind = "REAL",
                        Value::Text(_) | Value::Blob(_) => return "TEXT",
                    }
                }
                kind
            })
            .collect()
    }
}

/// A field of text as the value it is: empty for nothing, a whole number, a number, or
/// else text. A number with a leading zero (`007`, a code) stays text, and so does a whole
/// number too long to be one exactly (an account number).
pub(crate) fn value(field: &str) -> Value {
    let trimmed = field.trim();
    if trimmed.is_empty() {
        return Value::Null;
    }
    if !numeric(trimmed) {
        return Value::Text(field.to_owned());
    }
    if let Ok(whole) = trimmed.parse::<i64>() {
        return Value::Integer(whole);
    }
    if trimmed.contains(['.', 'e', 'E'])
        && let Ok(number) = trimmed.parse::<f64>()
    {
        return Value::Real(number);
    }
    Value::Text(field.to_owned())
}

/// Whether `text` is written as a number: a sign, digits, a point and more digits, an
/// exponent; with no leading zero but before a point.
fn numeric(text: &str) -> bool {
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (unsigned, None),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    let exponent_ok = exponent.is_none_or(|exponent| {
        let exponent = exponent.strip_prefix(['-', '+']).unwrap_or(exponent);
        !exponent.is_empty() && digits(exponent)
    });
    !(whole.is_empty() && fraction.is_empty())
        && digits(whole)
        && digits(fraction)
        && !(whole.len() > 1 && whole.starts_with('0'))
        && exponent_ok
}

/// A name for a table from a file's or a sheet's: letters, digits and underscores, the rest
/// turned to underscores, and an underscore first if it would start with a digit, so it can be
/// written in a query as it is.
pub(crate) fn table_name(text: &str) -> String {
    let name: String = text
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    match name.chars().next() {
        None => "data".to_owned(),
        Some(first) if first.is_ascii_digit() => format!("_{name}"),
        Some(_) => name,
    }
}

/// `name` as SQL names it: as it is when it can be, otherwise in double quotes.
pub(crate) fn quoted(name: &str) -> String {
    let plain = name
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    if plain && !reserved(name) {
        name.to_owned()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

/// Whether `name` is one of SQL's words that a column is often named after and a query can't
/// use bare.
fn reserved(name: &str) -> bool {
    [
        "all", "and", "as", "between", "by", "case", "check", "default", "desc", "distinct",
        "else", "end", "from", "group", "having", "in", "index", "is", "join", "like", "limit",
        "not", "null", "on", "or", "order", "select", "set", "table", "to", "union", "values",
        "when", "where",
    ]
    .iter()
    .any(|word| word.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_read_as_the_values_they_are() {
        assert_eq!(value(""), Value::Null);
        assert_eq!(value("  "), Value::Null);
        assert_eq!(value("120"), Value::Integer(120));
        assert_eq!(value(" -7 "), Value::Integer(-7));
        assert_eq!(value("0"), Value::Integer(0));
        assert_eq!(value("19.5"), Value::Real(19.5));
        assert_eq!(value("-0.25"), Value::Real(-0.25));
        assert_eq!(value(".5"), Value::Real(0.5));
        assert_eq!(value("1e3"), Value::Real(1000.0));
        for text in [
            "007",
            "1,200",
            "$5",
            "12%",
            "nan",
            "inf",
            "1e",
            "-",
            ".",
            "2026-01-31",
            "12345678901234567890",
        ] {
            assert_eq!(value(text), Value::Text(text.to_owned()), "{text}");
        }
    }

    #[test]
    fn columns_take_their_header_s_names_and_the_plainest_type() {
        let row = |fields: &[&str]| fields.iter().map(|field| value(field)).collect();
        let table = Table::headed(
            "t".into(),
            "t.csv".into(),
            vec![
                row(&["Region", "", "region", "Units", "Price", "Code"]),
                row(&["North", "x", "y", "120", "19.5", "007"]),
                row(&["South", "", "", "", "20", "12"]),
                row(&["West"]),
            ],
            0,
        );
        assert_eq!(
            table.columns,
            ["Region", "B", "region_2", "Units", "Price", "Code"]
        );
        assert_eq!(
            table.types(),
            ["TEXT", "TEXT", "TEXT", "INTEGER", "REAL", "TEXT"]
        );
        assert_eq!(table.rows.get(2).map(Vec::len), Some(6));
    }

    #[test]
    fn names_are_written_so_a_query_can_use_them() {
        assert_eq!(table_name("sales-2026"), "sales_2026");
        assert_eq!(table_name("2026 sales"), "_2026_sales");
        assert_eq!(table_name(""), "data");
        assert_eq!(quoted("units"), "units");
        assert_eq!(quoted("Unit price"), "\"Unit price\"");
        assert_eq!(quoted("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(quoted("Order"), "\"Order\"");
        assert_eq!(quoted("2026"), "\"2026\"");
    }
}
