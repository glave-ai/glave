//! `glave doc sql`: CSV, JSON and workbooks loaded as tables into an in-memory SQLite and
//! queried, the answer printed as a table or as CSV. SQLite is glave's own (its store's), built
//! with its math and percentile functions; standard deviation and correlation are added.

use std::fmt::Write as _;
use std::path::PathBuf;

use rusqlite::fallible_iterator::FallibleIterator as _;
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Batch, Connection, params_from_iter};

use crate::command::usage;
use crate::error::OfficeError;
use crate::read::{general, quoted as field};
use load::Source;
use table::{Table, quoted};

/// The rows shown of an answer as a table; `--csv` prints every one.
const ROWS_SHOWN: usize = 100;

/// What `glave doc sql` was asked.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Query {
    pub(crate) sources: Vec<Source>,
    /// The SQL to run; with none, the tables are described.
    pub(crate) sql: Option<String>,
    /// The sheet to load of each workbook; every worksheet when `None`.
    pub(crate) sheet: Option<String>,
    /// Whether to print the answer as CSV.
    pub(crate) csv: bool,
}

impl Query {
    /// The words after `glave doc sql`: files it loads, and one more word, the query.
    pub(crate) fn parse(
        words: &[PathBuf],
        sheet: Option<String>,
        csv: bool,
    ) -> Result<Self, OfficeError> {
        let mut sources = Vec::new();
        let mut sql: Option<String> = None;
        for word in words {
            let text = word.to_string_lossy();
            if let Some(source) = Source::parse(&text) {
                sources.push(source);
                continue;
            }
            if let Some(before) = &sql {
                return Err(usage(&format!(
                    "glave doc sql takes one query, and files it loads ({}): {before:?} and \
                     {text:?} can't both be the query",
                    loaded()
                )));
            }
            sql = Some(text.into_owned());
        }
        if sources.is_empty() {
            return Err(usage(&format!(
                "glave doc sql needs a file to load ({})",
                loaded()
            )));
        }
        if sql.as_deref().is_some_and(|sql| sql.trim().is_empty()) {
            return Err(usage("glave doc sql was given an empty query"));
        }
        if sheet.is_some() && !sources.iter().any(Source::workbook) {
            return Err(usage(
                "--sheet names the sheet to load of a workbook, and no workbook was given",
            ));
        }
        Ok(Self {
            sources,
            sql,
            sheet,
            csv,
        })
    }
}

/// The files `glave doc sql` loads, as its words name them.
fn loaded() -> String {
    load::EXTENSIONS
        .map(|extension| format!(".{extension}"))
        .join(", ")
}

/// Runs `query`: the files loaded, then the SQL's answer, or each table's columns and rows.
pub(crate) fn run(query: &Query) -> Result<String, OfficeError> {
    let mut tables: Vec<Table> = Vec::new();
    for source in &query.sources {
        for table in source.tables(query.sheet.as_deref())? {
            if let Some(before) = tables
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(&table.name))
            {
                return Err(OfficeError::Data(format!(
                    "{} and {} would both be the table {}: name one, like other={}",
                    before.from,
                    table.from,
                    table.name,
                    source.path.display()
                )));
            }
            tables.push(table);
        }
    }
    let connection = Connection::open_in_memory().map_err(|error| failed(&error))?;
    functions::add(&connection).map_err(|error| failed(&error))?;
    for table in &tables {
        store(&connection, table).map_err(|error| failed(&error))?;
    }
    let Some(sql) = &query.sql else {
        return Ok(described(&tables));
    };
    let answer = answer(&connection, sql).map_err(|error| {
        let said = match &error {
            rusqlite::Error::SqliteFailure(_, Some(message))
            | rusqlite::Error::SqlInputError { msg: message, .. } => message.clone(),
            error => error.to_string(),
        };
        OfficeError::Query(format!("{said}. {}", described(&tables).trim_end()))
    })?;
    Ok(match answer {
        Some(rows) if query.csv => as_csv(&rows),
        Some(rows) => shown(&rows),
        None => format!("Done: {} rows changed", connection.changes()),
    })
}

/// A failure of SQLite's own, loading the tables rather than running the query.
fn failed(error: &rusqlite::Error) -> OfficeError {
    OfficeError::Data(format!("SQLite could not load the tables: {error}"))
}

/// Creates `table` in `connection` and fills it, in one transaction.
fn store(connection: &Connection, table: &Table) -> rusqlite::Result<()> {
    let columns: Vec<String> = table
        .columns
        .iter()
        .zip(table.types())
        .map(|(column, kind)| format!("{} {kind}", quoted(column)))
        .collect();
    let name = quoted(&table.name);
    connection.execute_batch(&format!(
        "BEGIN; CREATE TABLE {name} ({});",
        columns.join(", ")
    ))?;
    let places = vec!["?"; table.columns.len()].join(", ");
    {
        let mut insert = connection.prepare(&format!("INSERT INTO {name} VALUES ({places})"))?;
        for row in &table.rows {
            insert.execute(params_from_iter(row))?;
        }
    }
    connection.execute_batch("COMMIT")
}

/// Each table: its name, where it came from, its rows and its columns with their types.
fn described(tables: &[Table]) -> String {
    let mut text = String::new();
    if tables.is_empty() {
        text.push_str("No tables are loaded.");
    }
    for table in tables {
        let columns: Vec<String> = table
            .columns
            .iter()
            .zip(table.types())
            .map(|(column, kind)| format!("{} {kind}", quoted(column)))
            .collect();
        let rows = match table.rows.len() {
            1 => "1 row".to_owned(),
            count => format!("{count} rows"),
        };
        let _ = writeln!(
            text,
            "Table {} from {}, {rows}: {}",
            quoted(&table.name),
            table.from,
            columns.join(", ")
        );
    }
    text
}

/// An answer: its columns' names, then its rows.
struct Rows {
    columns: Vec<String>,
    rows: Vec<Vec<Value>>,
}

/// Runs each statement of `sql` in turn, and gives the last answer with columns; `None` when
/// no statement has any (it only changed tables).
fn answer(connection: &Connection, sql: &str) -> rusqlite::Result<Option<Rows>> {
    let mut batch = Batch::new(connection, sql);
    let mut last = None;
    while let Some(mut statement) = batch.next()? {
        if statement.column_count() == 0 {
            statement.execute([])?;
            continue;
        }
        let columns: Vec<String> = statement
            .column_names()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let width = columns.len();
        let mut rows = Vec::new();
        let mut found = statement.query([])?;
        while let Some(row) = found.next()? {
            let mut values = Vec::with_capacity(width);
            for at in 0..width {
                values.push(row.get::<_, Value>(at)?);
            }
            rows.push(values);
        }
        last = Some(Rows { columns, rows });
    }
    Ok(last)
}

/// A value as the answer shows it: a number as a spreadsheet shows it, empty for none.
fn text(value: &Value) -> String {
    match ValueRef::from(value) {
        ValueRef::Null => String::new(),
        ValueRef::Integer(whole) => whole.to_string(),
        ValueRef::Real(number) => general(number),
        ValueRef::Text(text) => String::from_utf8_lossy(text).into_owned(),
        ValueRef::Blob(bytes) => format!("({} bytes)", bytes.len()),
    }
}

/// The answer as a table, its first [`ROWS_SHOWN`] rows, and how many more there are.
fn shown(rows: &Rows) -> String {
    let mut lines = vec![rows.columns.clone()];
    lines.extend(
        rows.rows
            .iter()
            .take(ROWS_SHOWN)
            .map(|row| row.iter().map(text).collect()),
    );
    let mut shown = crate::table::lines(&lines).join("\n");
    match rows.rows.len() {
        0 => shown.push_str("\n(no rows)"),
        count if count > ROWS_SHOWN => {
            let _ = write!(
                shown,
                "\n… {} more rows, not shown: add --csv to print every row",
                count - ROWS_SHOWN
            );
        }
        _ => {}
    }
    shown
}

/// The answer as CSV, its columns' names first.
fn as_csv(rows: &Rows) -> String {
    let line = |fields: &[String]| {
        let fields: Vec<String> = fields.iter().map(|value| field(value)).collect();
        fields.join(",") + "\n"
    };
    let mut out = line(&rows.columns);
    for row in &rows.rows {
        out.push_str(&line(&row.iter().map(text).collect::<Vec<_>>()));
    }
    out
}

mod csv;
mod functions;
mod json;
pub(crate) mod load;
mod table;
