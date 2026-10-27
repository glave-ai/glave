//! Workbooks made with `glave doc write` and read back with `glave doc`, as the model runs
//! them: the skill's own example, every kind of cell, and a file already there kept.

use glave_office::SKILLS;
use glave_ports::testing::TempDir;

use crate::workbooks::{answer, doc, has};

/// The JSON in the skill's example, between its heredoc's lines.
fn skill_example() -> String {
    let lines: Vec<&str> = SKILLS[0].lines().collect();
    let start = lines.iter().position(|line| line.ends_with("<<'EOF'"));
    let end = lines.iter().position(|line| *line == "EOF");
    lines[start.unwrap() + 1..end.unwrap()].join("\n")
}

#[test]
fn the_skills_example_reads_back_with_its_formulas_not_computed_yet() {
    let dir = TempDir::new("office-example").unwrap();
    let out = dir.join("report.xlsx").to_string_lossy().into_owned();
    let wrote = answer(&["write", &out], &skill_example()).unwrap();
    assert_eq!(wrote.warning, None);
    let summary = format!("Wrote {out}: Sales (4 rows, 4 formulas, 1 chart).\nIts formulas");
    assert!(wrote.printed.starts_with(&summary), "{}", wrote.printed);
    let read = doc(&["read", &out]).unwrap();
    has(
        &read,
        &[
            "report.xlsx: 1 sheet: Sales",
            "## Sales (A1:D4)",
            "| 1 | Month | Units | Price | Revenue |",
            "| 2 | Jan | 120 | 19.5 | =B2*C2 → not computed yet |",
            "| 4 | Total | =SUM(B2:B3) → not computed yet |  | =SUM(D2:D3) → not computed yet |",
        ],
    );
    assert!(
        !read.contains("_xlnm"),
        "the header's filter is Excel's own"
    );
    let csv = answer(&["csv", &out], "").unwrap();
    assert_eq!(
        csv.printed,
        "Month,Units,Price,Revenue\nJan,120,19.5,\nFeb,135,19.5,\nTotal,,,\n"
    );
    let warning = csv.warning.unwrap();
    assert!(warning.contains(": Sales!D2, D3, B4, D4. "), "{warning}");
}

#[test]
fn every_kind_of_cell_comes_back_as_written_from_a_description_in_a_file() {
    let dir = TempDir::new("office-cells").unwrap();
    let description = dir.join("log.json");
    std::fs::write(
        &description,
        r#"{"sheets": [{"name": "Q1 log", "freeze": "B2", "widths": {"B": 30},
            "formats": {"D1:F9": "0.00", "F1": "0.0%"},
            "rows": [[{"date": "2026-01-31 14:30"}, "a, \"quoted\" | piped\nline", true, null,
                      42, 0.125, "=", "=XLOOKUP(5,A1:A2,A1:A2)"]]}]}"#,
    )
    .unwrap();
    let out = dir.join("log.xlsx").to_string_lossy().into_owned();
    let wrote = doc(&["write", &out, &description.to_string_lossy()]).unwrap();
    assert!(wrote.starts_with(&format!("Wrote {out}: Q1 log (1 row, 1 formula).")));
    has(
        &doc(&["read", &out]).unwrap(),
        &[
            "| 1 | 2026-01-31 14:30:00 | a, \"quoted\" \\| piped\\nline | true |  | 42 | 0.125 | \
             = | =XLOOKUP(5,A1:A2,A1:A2) → not computed yet |",
        ],
    );
    assert_eq!(
        doc(&["csv", &out, "--sheet", "q1 LOG"]).unwrap(),
        "2026-01-31 14:30:00,\"a, \"\"quoted\"\" | piped\nline\",true,,42,0.125,=,\n"
    );
}

#[test]
fn a_file_already_there_is_kept_unless_replacing_it_was_asked() {
    let dir = TempDir::new("office-replace").unwrap();
    let out = dir.join("budget.xlsx");
    let path = out.to_string_lossy().into_owned();
    std::fs::write(&out, "a person's own workbook").unwrap();
    let one = r#"{"sheets": [{"name": "Budget", "rows": [[1]]}]}"#;
    let kept = answer(&["write", &path], one).unwrap_err().to_string();
    assert!(
        kept.starts_with(&format!(
            "{path} is already there, and glave doc write makes"
        )) && kept.ends_with("add --replace if replacing it is what was asked"),
        "{kept}"
    );
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "a person's own workbook"
    );
    answer(&["write", &path, "--replace"], one).unwrap();
    has(&doc(&["read", &path]).unwrap(), &["| 1 | 1 |"]);
}

#[test]
fn nothing_is_written_from_a_wrong_description_or_none() {
    let dir = TempDir::new("office-wrong").unwrap();
    let out = dir.join("out.xlsx");
    let path = out.to_string_lossy().into_owned();
    let wrong = answer(
        &["write", &path],
        r#"{"sheets": [{"name": "A", "rows": [[[2]]]}]}"#,
    );
    assert!(
        wrong
            .unwrap_err()
            .to_string()
            .contains("in the sheet \"A\", A1:")
    );
    assert_eq!(
        answer(&["write", &path], " \n").unwrap_err().to_string(),
        "the workbook's description is not right: there is none: give it on standard input, \
         or name its file after the workbook's"
    );
    let missing = dir.join("gone.json").to_string_lossy().into_owned();
    let gone = doc(&["write", &path, &missing]).unwrap_err().to_string();
    assert!(gone.starts_with("could not read") && gone.contains("gone.json"));
    assert!(!out.exists(), "nothing is written");
}
