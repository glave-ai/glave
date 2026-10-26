//! `glave doc` on a workbook as Excel saves one, as the model runs it. `workbook.xlsx` was
//! written by rust_xlsxwriter for #646's spike: a header, dates and formulas with their computed
//! results (Sales), a sheet name to quote and a sum with binary noise (Q1 sales), text a table
//! or a CSV must keep whole from B2 (Log), 250 rows (Big data), formulas no app has computed
//! (Draft), a hidden sheet (Old) and a chart sheet (Chart).

use std::path::Path;

use glave_office::{OfficeError, USAGE, run};
use glave_ports::testing::TempDir;

const WORKBOOK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/workbook.xlsx");

fn doc(words: &[&str]) -> Result<String, OfficeError> {
    let words: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
    run(&words)
}

fn has(text: &str, lines: &[&str]) {
    for line in lines {
        assert!(
            text.lines().any(|seen| seen == *line),
            "no {line:?} in:\n{text}"
        );
    }
}

#[test]
fn a_workbook_reads_as_tables_with_each_formula_beside_its_value() {
    let read = doc(&["read", WORKBOOK]).unwrap();
    has(
        &read,
        &[
            "workbook.xlsx: 7 sheets: Sales, Q1 sales, Log, Big data, Draft, Old (hidden), \
             Chart (a chart)",
            "Name Revenue = Sales!$E$2:$E$4",
            "## Sales (A1:E5)",
            "| | A | B | C | D | E |",
            "| 1 | Day | Region | Units | Price | Revenue |",
            "| 2 | 2026-01-05 | North | 120 | 19.5 | =C2*D2 → 2340 |",
            "| 5 | Total |  |  |  | =SUM(E2:E4) → 5905 |",
            // Numbers as a spreadsheet shows them, and newer functions without Excel's prefix.
            "| 4 | =SUM(A2:A3) → 0.3 |",
            "| 5 | =XLOOKUP(0.2,A2:A3,A2:A3) → 0.2 |",
            "## Log (B2:F2)",
            "| 2 | 2026-01-31 14:30:00 | a, \"quoted\" \\| piped\\nline | true |  | 42 |",
            "| 1 | 2 | =A1*2 → not computed yet |",
            "## Old (A1)",
        ],
    );
    assert!(!read.contains("## Chart"), "a chart sheet has no cells");
}

#[test]
fn a_long_sheet_says_how_to_read_the_rest() {
    let read = doc(&["read", WORKBOOK, "--sheet", "big DATA"]).unwrap();
    assert!(
        read.contains("| 100 | 100 |") && !read.contains("| 101 |"),
        "{read}"
    );
    has(
        &read,
        &[
            "… rows 101 to 250 not shown: glave doc read workbook.xlsx --sheet 'Big data' \
           --rows 101-200",
        ],
    );
    let next = doc(&["read", WORKBOOK, "--sheet", "Big data", "--rows", "201-400"]).unwrap();
    assert!(
        next.contains("| 201 | 201 |") && next.contains("| 250 | 250 |"),
        "{next}"
    );
    assert!(
        !next.contains("| 200 |") && !next.contains("not shown"),
        "{next}"
    );
}

#[test]
fn a_sheet_reads_as_csv_with_its_computed_values() {
    let sales = doc(&["csv", WORKBOOK]).unwrap();
    assert_eq!(
        sales,
        "Day,Region,Units,Price,Revenue\n2026-01-05,North,120,19.5,2340\n\
         2026-01-06,South,100,21.25,2125\n2026-01-07,West,80,18,1440\nTotal,,,,5905\n"
    );
    let log = doc(&["csv", WORKBOOK, "--sheet", "Log"]).unwrap();
    assert_eq!(
        log,
        "2026-01-31 14:30:00,\"a, \"\"quoted\"\" | piped\nline\",true,,42\n"
    );
}

#[test]
fn mistakes_are_said_so_they_can_be_put_right() {
    let sheet = doc(&["read", WORKBOOK, "--sheet", "Q2"])
        .unwrap_err()
        .to_string();
    assert_eq!(
        sheet,
        "there is no sheet named \"Q2\"; the sheets are Sales, Q1 sales, Log, Big data, \
         Draft, Old"
    );
    let dir = TempDir::new("office-mistakes").unwrap();
    let text = dir.join("notes.txt");
    std::fs::write(&text, "not a workbook").unwrap();
    let not = doc(&["read", &text.to_string_lossy()])
        .unwrap_err()
        .to_string();
    assert!(
        not.starts_with("this is not a workbook glave can read"),
        "{not}"
    );
    let missing = Path::new(&*dir).join("gone.xlsx");
    let gone = doc(&["csv", &missing.to_string_lossy()])
        .unwrap_err()
        .to_string();
    assert!(
        gone.starts_with("could not read") && gone.contains("gone.xlsx"),
        "{gone}"
    );
    assert_eq!(doc(&[]).unwrap(), USAGE);
}
