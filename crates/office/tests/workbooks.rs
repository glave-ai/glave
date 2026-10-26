//! Workbooks written and read through `glave doc`, as the model runs it.

use std::path::Path;

use glave_office::{OfficeError, SKILLS, USAGE, run};
use glave_ports::testing::TempDir;
use rust_xlsxwriter::{Formula, Workbook};

fn doc(words: &[&str], input: &str) -> Result<String, OfficeError> {
    let words: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
    run(&words, &mut input.as_bytes())
}

fn path(dir: &Path, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

/// The JSON in the skill's example, between its heredoc's lines.
fn skill_example() -> String {
    let lines: Vec<&str> = SKILLS[0].lines().collect();
    let start = lines
        .iter()
        .position(|line| line.ends_with("<<'EOF'"))
        .unwrap();
    let end = start
        + lines[start..]
            .iter()
            .position(|line| *line == "EOF")
            .unwrap();
    lines[start + 1..end].join("\n")
}

#[test]
fn the_skills_example_writes_a_workbook_that_reads_back_with_its_formulas_and_chart() {
    let dir = TempDir::new("office-example").unwrap();
    let out = path(&dir, "report.xlsx");
    let said = doc(&["write", &out, "-"], &skill_example()).unwrap();
    assert_eq!(said, format!("Wrote {out}."));
    let bytes = std::fs::read(&out).unwrap();
    // A zip names its parts in the clear: the native chart is there.
    assert!(
        bytes
            .windows(20)
            .any(|part| part == b"xl/charts/chart1.xml")
    );
    let read = doc(&["read", &out], "").unwrap();
    let expected = [
        "report.xlsx: 1 sheet: Sales",
        "Name Revenue = Sales!$D$2:$D$3",
        "## Sales (A1:D4)",
        "| | A | B | C | D |",
        "| 1 | Month | Units | Price | Revenue |",
        "| 2 | Jan | 120 | 19.5 | =B2*C2 → not computed yet |",
        "| 4 | Total | =SUM(B2:B3) → not computed yet |  | =SUM(D2:D3) → not computed yet |",
    ];
    for line in expected {
        assert!(
            read.lines().any(|seen| seen == line),
            "no {line:?} in:\n{read}"
        );
    }
    assert!(
        !read.contains("_xlnm"),
        "Excel's own names are left out: {read}"
    );
}

#[test]
fn a_formula_shows_beside_the_value_a_spreadsheet_app_computed() {
    let dir = TempDir::new("office-computed").unwrap();
    let file = dir.join("computed.xlsx");
    let mut book = Workbook::new();
    let sheet = book.add_worksheet().set_name("Q1 sales").unwrap();
    sheet.write_string(0, 0, "Units").unwrap();
    sheet.write_number(1, 0, 0.1).unwrap();
    sheet.write_number(2, 0, 0.2).unwrap();
    sheet
        .write_formula(
            3,
            0,
            Formula::new("=SUM(A2:A3)").set_result("0.30000000000000004"),
        )
        .unwrap();
    book.save(&file).unwrap();
    let read = doc(&["read", &file.to_string_lossy()], "").unwrap();
    assert!(read.contains("| 4 | =SUM(A2:A3) → 0.3 |"), "{read}");
    let csv = doc(&["csv", &file.to_string_lossy(), "--sheet", "q1 SALES"], "").unwrap();
    assert_eq!(csv, "Units\n0.1\n0.2\n0.3\n");
}

#[test]
fn dates_formats_and_awkward_text_come_back_as_written() {
    let dir = TempDir::new("office-cells").unwrap();
    let out = path(&dir, "cells.xlsx");
    let description = r#"{"sheets": [{"name": "Log", "start": "B2", "rows": [
        [{"date": "2026-01-31"}, "a, \"quoted\" | piped\nline", {"text": "=not a formula"}, true, null, 42]
    ], "formats": {"G2": "0.0%"}, "widths": {"C": 30}, "freeze": "C3"}]}"#;
    doc(&["write", &out, "-"], description).unwrap();
    let read = doc(&["read", &out], "").unwrap();
    assert!(
        read.contains(
            "| 2 | 2026-01-31 | a, \"quoted\" \\| piped\\nline | =not a formula | true |  | 42 |"
        ),
        "{read}"
    );
    let csv = doc(&["csv", &out], "").unwrap();
    assert_eq!(
        csv,
        "2026-01-31,\"a, \"\"quoted\"\" | piped\nline\",=not a formula,true,,42\n"
    );
}

#[test]
fn a_long_sheet_says_how_to_read_the_rest() {
    let dir = TempDir::new("office-long").unwrap();
    let out = path(&dir, "long.xlsx");
    let rows: Vec<String> = (1..=250).map(|n| format!("[{n}]")).collect();
    let description = format!(
        r#"{{"sheets": [{{"name": "Big data", "rows": [{}]}}]}}"#,
        rows.join(",")
    );
    doc(&["write", &out, "-"], &description).unwrap();
    let read = doc(&["read", &out], "").unwrap();
    assert!(
        read.contains("| 100 | 100 |") && !read.contains("| 101 |"),
        "{read}"
    );
    assert!(
        read.contains("… rows 101 to 250 not shown: glave doc read long.xlsx --sheet 'Big data' --rows 101-200"),
        "{read}"
    );
    let next = doc(
        &["read", &out, "--sheet", "Big data", "--rows", "201-400"],
        "",
    )
    .unwrap();
    assert!(
        next.contains("| 201 | 201 |") && next.contains("| 250 | 250 |"),
        "{next}"
    );
    assert!(!next.contains("not shown"), "{next}");
}

#[test]
fn mistakes_are_said_so_they_can_be_put_right() {
    let dir = TempDir::new("office-mistakes").unwrap();
    let out = path(&dir, "out.xlsx");
    let said = |description: &str| {
        doc(&["write", &out, "-"], description)
            .unwrap_err()
            .to_string()
    };
    assert!(said(r#"{"sheets": []}"#).contains("names no sheets"));
    let typo = said(r#"{"sheets": [{"name": "A", "rowz": []}]}"#);
    assert!(typo.contains("unknown field `rowz`"), "{typo}");
    let range = said(r#"{"sheets": [{"name": "A", "formats": {"B9:B2": "0%"}}]}"#);
    assert!(
        range.contains(r#"sheet "A": formats: "B9:B2" is not a range"#),
        "{range}"
    );
    let cell = said(r#"{"sheets": [{"name": "A", "rows": [[1, [2]]]}]}"#);
    assert!(
        cell.contains("B1: a cell holds one value, not a list"),
        "{cell}"
    );
    let chart = said(
        r#"{"sheets": [{"name": "A", "charts": [{"type": "radar", "at": "A1", "values": []}]}]}"#,
    );
    assert!(chart.contains("unknown variant `radar`"), "{chart}");
    assert!(
        !dir.join("out.xlsx").exists(),
        "nothing is written from a wrong description"
    );

    doc(
        &["write", &out, "-"],
        r#"{"sheets": [{"name": "Only", "rows": [[1]]}]}"#,
    )
    .unwrap();
    let sheet = doc(&["read", &out, "--sheet", "Other"], "")
        .unwrap_err()
        .to_string();
    assert_eq!(
        sheet,
        "there is no sheet named \"Other\"; the sheets are Only"
    );
    let text = dir.join("notes.txt");
    std::fs::write(&text, "not a workbook").unwrap();
    let not = doc(&["read", &text.to_string_lossy()], "")
        .unwrap_err()
        .to_string();
    assert!(
        not.starts_with("this is not a workbook glave can read"),
        "{not}"
    );
    assert_eq!(doc(&[], "").unwrap(), USAGE);
}
