//! `glave doc sql` on CSV, JSON and a workbook, as the model runs it: files loaded as tables,
//! queried with SQLite's own functions, its math and percentile ones, and the statistics glave
//! adds.

use std::path::Path;

use glave_office::OfficeError;
use glave_ports::testing::TempDir;

use crate::workbooks::{answer, doc};

const WORKBOOK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/workbook.xlsx");

/// A folder holding `sales.csv` and `regions.json`.
fn files() -> TempDir {
    let dir = TempDir::new("office-data").unwrap();
    std::fs::write(
        dir.path().join("sales.csv"),
        "day,region,units,price,\"Unit note\"\r\n\
         2026-01-05,North,120,19.5,\r\n\
         2026-02-09,South,80,21.25,\"late, by a day\"\r\n\
         2026-07-14,North,95,19.5,\r\n\
         2026-08-03,West,,18,none sold\r\n\
         2026-09-21,West,140,18,\r\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("regions.json"),
        r#"[{"region": "North", "manager": "Ana", "target": 2000},
            {"region": "South", "manager": "Bo", "target": 1500.5},
            {"region": "West", "manager": "Cy", "target": null, "tags": ["new"]}]"#,
    )
    .unwrap();
    dir
}

fn at(dir: &TempDir, file: &str) -> String {
    dir.path().join(file).display().to_string()
}

fn sql(words: &[&str]) -> Result<String, OfficeError> {
    let mut all = vec!["sql"];
    all.extend(words);
    doc(&all)
}

#[test]
fn files_load_as_tables_named_after_them_with_their_columns_types() {
    let dir = files();
    let (sales, regions) = (at(&dir, "sales.csv"), at(&dir, "regions.json"));
    assert_eq!(
        sql(&[&sales, &regions]).unwrap(),
        "Table sales from sales.csv, 5 rows: day TEXT, region TEXT, units INTEGER, price REAL, \
         \"Unit note\" TEXT\n\
         Table regions from regions.json, 3 rows: region TEXT, manager TEXT, target REAL, tags \
         TEXT\n"
    );
    let named = format!("orders={sales}");
    let listed = sql(&[&named, WORKBOOK, "--sheet", "Sales"]).unwrap();
    assert!(
        listed.starts_with("Table orders from sales.csv, 5 rows"),
        "{listed}"
    );
    assert!(
        listed.contains(
            "Table workbook from workbook.xlsx, sheet Sales, 4 rows: Day TEXT, Region TEXT, \
             Units INTEGER, Price REAL, Revenue INTEGER"
        ),
        "{listed}"
    );
    // Every worksheet but the empty ones, each named after the file and its sheet.
    let every = sql(&[WORKBOOK]).unwrap();
    for table in [
        "workbook_Sales ",
        "workbook_Q1_sales ",
        "workbook_Big_data ",
        "workbook_Old ",
    ] {
        assert!(
            every.contains(&format!("Table {table}")),
            "{table} in:\n{every}"
        );
    }
}

#[test]
fn a_query_joins_groups_and_answers_as_a_table_or_as_csv() {
    let dir = files();
    let (sales, regions) = (at(&dir, "sales.csv"), at(&dir, "regions.json"));
    let query = "SELECT s.region, r.manager, sum(s.units * s.price) AS revenue, \
                 count(s.units) AS orders FROM sales s JOIN regions r USING (region) \
                 GROUP BY s.region ORDER BY revenue DESC";
    assert_eq!(
        sql(&[&sales, &regions, query]).unwrap(),
        "| region | manager | revenue | orders |\n|---|---|---|---|\n\
         | North | Ana | 4192.5 | 2 |\n| West | Cy | 2520 | 1 |\n| South | Bo | 1700 | 1 |"
    );
    assert_eq!(
        sql(&[
            &sales,
            "SELECT region, \"Unit note\" FROM sales WHERE \"Unit note\" IS NOT NULL",
            "--csv"
        ])
        .unwrap(),
        "region,Unit note\nSouth,\"late, by a day\"\nWest,none sold\n"
    );
    // Statements run in turn, and the last one's answer is printed.
    assert_eq!(
        sql(&[&sales, "CREATE VIEW big AS SELECT * FROM sales WHERE units > 100; SELECT count(*) AS n FROM big"])
            .unwrap(),
        "| n |\n|---|\n| 2 |"
    );
    assert_eq!(
        sql(&[&sales, "SELECT * FROM sales WHERE units > 1000"]).unwrap(),
        "| day | region | units | price | Unit note |\n|---|---|---|---|---|\n(no rows)"
    );
}

#[test]
fn math_percentiles_and_statistics_are_there() {
    let dir = files();
    let sales = at(&dir, "sales.csv");
    let query = "SELECT median(units) AS median, percentile(units, 25) AS p25, \
                 sqrt(16) AS root, round(ln(exp(2)), 6) AS ln, floor(19.5) AS floor, \
                 round(stddev_pop(units), 6) AS spread, corr(units, units * 2) AS corr \
                 FROM sales WHERE region = 'North' OR units IS NULL";
    // North sold 120 and 95: the median 107.5, a quarter of the way 101.25, spread 12.5.
    assert_eq!(
        sql(&[&sales, query]).unwrap(),
        "| median | p25 | root | ln | floor | spread | corr |\n|---|---|---|---|---|---|---|\n\
         | 107.5 | 101.25 | 4 | 2 | 19 | 12.5 | 1 |"
    );
}

#[test]
fn many_rows_show_the_first_hundred_and_csv_prints_them_all() {
    let dir = TempDir::new("office-data-many").unwrap();
    let rows: String = (1..=250).fold(String::new(), |rows, n| rows + &n.to_string() + "\n");
    let file = dir.path().join("many.csv");
    std::fs::write(&file, format!("n\n{rows}")).unwrap();
    let file = file.display().to_string();
    let shown = sql(&[&file, "SELECT n FROM many"]).unwrap();
    assert!(
        shown.contains("| 100 |") && !shown.contains("| 101 |"),
        "{shown}"
    );
    assert!(
        shown.ends_with("\n… 150 more rows, not shown: add --csv to print every row"),
        "{shown}"
    );
    assert_eq!(
        sql(&[&file, "SELECT n FROM many", "--csv"]).unwrap(),
        format!("n\n{rows}")
    );
}

#[test]
fn what_goes_wrong_is_said_plainly() {
    let dir = files();
    let sales = at(&dir, "sales.csv");
    let error = sql(&[&sales, "SELECT revenue FROM sales"]).unwrap_err();
    assert!(matches!(error, OfficeError::Query(_)), "{error:?}");
    assert_eq!(
        error.to_string(),
        "the query failed: no such column: revenue. Table sales from sales.csv, 5 rows: day \
         TEXT, region TEXT, units INTEGER, price REAL, \"Unit note\" TEXT"
    );
    let error = sql(&[&sales, "SELECT stddev(region) FROM sales"])
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with(
            "the query failed: stddev takes numbers, and was given the text \"North\""
        ),
        "{error}"
    );
    let missing = sql(&[&at(&dir, "nowhere.csv")]).unwrap_err().to_string();
    assert!(
        missing.starts_with("could not read ") && missing.contains("nowhere.csv"),
        "{missing}"
    );
    assert_eq!(
        sql(&[WORKBOOK, "--sheet", "Winter"])
            .unwrap_err()
            .to_string(),
        "there is no sheet named \"Winter\"; the sheets are Sales, Q1 sales, Log, Big data, Draft, Old"
    );
    let broken = dir.path().join("broken.json");
    std::fs::write(&broken, r#"{"region": "North"}"#).unwrap();
    let broken = sql(&[&broken.display().to_string()])
        .unwrap_err()
        .to_string();
    assert!(
        broken.starts_with("broken.json is not a JSON array of objects"),
        "{broken}"
    );
    let twice = format!("{}", Path::new(&sales).with_extension("json").display());
    std::fs::write(&twice, "[]").unwrap();
    assert_eq!(
        sql(&[&sales, &twice]).unwrap_err().to_string(),
        format!(
            "sales.csv and sales.json would both be the table sales: name one, like other={twice}"
        )
    );
}

#[test]
fn the_words_are_checked_before_anything_loads() {
    let usage = |words: &[&str]| {
        let error = sql(words).unwrap_err();
        assert!(matches!(error, OfficeError::Usage(_)), "{error:?}");
        error.to_string().lines().next().unwrap().to_owned()
    };
    assert_eq!(
        usage(&["SELECT 1"]),
        "glave doc sql needs a file to load (.csv, .tsv, .json, .xlsx, .xlsm, .xlsb, .xls, .ods)"
    );
    assert_eq!(
        usage(&["a.csv", "SELECT 1", "notes.txt"]),
        "glave doc sql takes one query, and files it loads (.csv, .tsv, .json, .xlsx, .xlsm, \
         .xlsb, .xls, .ods): \"SELECT 1\" and \"notes.txt\" can't both be the query"
    );
    assert_eq!(
        usage(&["a.csv", " "]),
        "glave doc sql was given an empty query"
    );
    assert_eq!(
        usage(&["a.csv", "--sheet", "Sales"]),
        "--sheet names the sheet to load of a workbook, and no workbook was given"
    );
    assert_eq!(
        usage(&["a.csv", "--rows", "1-2"]),
        "glave doc sql was not given what it takes"
    );
    let read = answer(&["read", "a.csv", "--csv"], "")
        .unwrap_err()
        .to_string();
    assert!(read.starts_with("glave doc read takes no --csv"), "{read}");
}
