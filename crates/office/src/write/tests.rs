use super::*;

/// What is wrong with `json`, as `glave doc write` says it.
fn wrong(json: &str) -> String {
    match write(json) {
        Ok(_) => panic!("{json} was written"),
        Err(error) => error.to_string(),
    }
}

/// What is wrong with a sheet named `S` described by `fields`.
fn wrong_in_sheet(fields: &str) -> String {
    wrong(&format!(r#"{{"sheets": [{{"name": "S", {fields}}}]}}"#))
}

fn report(json: &str) -> String {
    write(json).unwrap().report
}

#[test]
fn the_report_says_what_each_sheet_holds_and_that_formulas_wait_for_an_app() {
    let said = report(
        r#"{"sheets": [
            {"name": "Orders", "header": true, "rows": [["Region", "Amount"], ["North", 12.5]]},
            {"name": "Summary", "rows": [["North", "=SUMIF(Orders!A2:A2,A1,Orders!B2:B2)"]],
             "charts": [{"type": "column", "at": "D2", "values": ["B1"], "categories": "A1"}]}
        ]}"#,
    );
    assert_eq!(
        said,
        "Orders (2 rows), Summary (1 row, 1 formula, 1 chart).\nIts formulas have no results \
         until a spreadsheet app opens the file and works them out, so glave doc read shows \
         each as not computed yet."
    );
    assert_eq!(
        report(r#"{"sheets": [{"name": "Empty"}]}"#),
        "Empty (0 rows)."
    );
}

#[test]
fn numbers_written_as_text_are_pointed_out_but_a_header_and_codes_are_not() {
    let said = report(
        r#"{"sheets": [{"name": "Orders", "header": true, "rows": [["Order", "2026"],
            ["007", "1250.00"], ["008", "830.5"], ["009", "-12"], ["010", "4e2"], ["011", "+3"]]}]}"#,
    );
    assert!(
        said.ends_with(
            "\nNumbers written as text, which formulas don't add up: Orders!B2 \"1250.00\", \
             Orders!B3 \"830.5\", Orders!B4 \"-12\" and 1 more. Write numbers without quotes \
             unless they are labels."
        ),
        "{said}"
    );
    for (text, number) in [
        ("0.5", true),
        (".5", true),
        ("0", true),
        ("1e5", false),
        ("inf", false),
        ("1,250", false),
        ("-", false),
        ("2026-09-01", false),
    ] {
        assert_eq!(reads_as_number(text), number, "{text:?}");
    }
}

#[test]
fn a_description_that_is_not_the_json_expected_says_what_and_where() {
    let typo = wrong(r#"{"sheets": [{"name": "A", "rowz": []}]}"#);
    assert!(
        typo.starts_with(
            "the workbook's description is not right: it is not the JSON glave expects: \
             unknown field `rowz`, expected one of `name`, `rows`"
        ) && typo.ends_with("at line 1 column 32"),
        "{typo}"
    );
    let kind = wrong_in_sheet(r#""charts": [{"type": "radar"}]"#);
    assert!(kind.contains("unknown variant `radar`, expected one of `column`"));
    assert!(wrong(r#"{"sheets": [{"rows": []}]}"#).contains("missing field `name`"));
    assert!(wrong("Sales by month").contains("expected value at line 1 column 1"));
    assert_eq!(
        wrong(r#"{"sheets": []}"#),
        "the workbook's description is not right: it has no sheets: give \"sheets\" at least one"
    );
    let twice = wrong(r#"{"sheets": [{"name": "Sales"}, {"name": "SALES"}]}"#);
    assert!(twice.contains("'sales' has already been used"), "{twice}");
    let slash = wrong(r#"{"sheets": [{"name": "Q1/Q2"}]}"#);
    assert!(
        slash.contains("cannot contain invalid characters"),
        "{slash}"
    );
}

#[test]
fn a_wrong_cell_range_or_pane_is_named_with_its_sheet() {
    assert_eq!(
        wrong_in_sheet(r#""rows": [[1, [2]]]"#),
        "the workbook's description is not right: in the sheet \"S\", B1: a cell is a number, \
         text, true, false, null, a formula like \"=B2*C2\" or a date like \
         {\"date\": \"2026-01-31\"}"
    );
    assert!(wrong_in_sheet(r#""rows": [[], [{"value": 2}]]"#).contains("\"S\", A2: a cell is"));
    assert!(
        wrong_in_sheet(r#""rows": [[{"date": "31/01/2026"}]]"#)
            .contains("A1: \"31/01/2026\" is not a date like 2026-01-31 or 2026-01-31 14:30")
    );
    assert!(
        wrong_in_sheet(r#""formats": {"B9:B2": "0%"}"#)
            .ends_with("in the sheet \"S\", formats: \"B9:B2\" is not a range like C2:D13")
    );
    assert!(
        wrong_in_sheet(r#""widths": {"A1": 10}"#)
            .ends_with("widths: \"A1\" is not a column like A or AB")
    );
    assert!(
        wrong_in_sheet(r#""freeze": "row 2""#)
            .ends_with("freeze \"row 2\" is not a cell like B2, the first cell that scrolls")
    );
}

#[test]
fn a_wrong_chart_is_named_by_where_it_sits() {
    let chart = |chart: &str| {
        wrong(&format!(
            r#"{{"sheets": [{{"name": "Sales", "rows": [["Jan", 1]]}},
                {{"name": "Summary", "charts": [{chart}]}}]}}"#
        ))
    };
    assert!(
        chart(r#"{"type": "line", "at": "here", "values": ["B1"]}"#)
            .contains("the chart at here: \"at\" is not a cell like F2")
    );
    assert!(
        chart(r#"{"type": "line", "at": "D2", "values": []}"#).contains(
            "in the sheet \"Summary\", the chart at D2: \"values\" needs a range for each series"
        )
    );
    assert!(
        chart(r#"{"type": "pie", "at": "D2", "values": ["Sale!B1"]}"#).contains(
            "values \"Sale!B1\" is on a sheet the workbook doesn't have; its sheets are Sales, \
             Summary"
        )
    );
    assert!(
        chart(r#"{"type": "scatter", "at": "D2", "values": ["Sales!B1:Z"]}"#)
            .contains("values \"Sales!B1:Z\" is not a range like B2:B13 or Sales!B2:B13")
    );
    assert!(
        chart(r#"{"type": "scatter", "at": "D2", "values": ["Sales!B1"]}"#)
            .contains("Scatter style charts must contain a 'categories' range")
    );
}

#[test]
fn every_kind_of_chart_is_written_from_ranges_on_any_sheet() {
    for kind in [
        "column", "bar", "line", "pie", "doughnut", "area", "scatter",
    ] {
        let written = write(&format!(
            r#"{{"sheets": [{{"name": "Q1 sales", "rows": [["Jan", 1], ["Feb", 2]]}},
                {{"name": "Charts", "charts": [{{"type": "{kind}", "at": "B2",
                  "values": ["'q1 SALES'!B1:B2", "'Q1 sales'!B1:B2"], "names": ["One", "Two"],
                  "categories": "'Q1 sales'!A1:A2", "title": "Sales",
                  "x_title": "Month", "y_title": "Units"}}]}}]}}"#
        ))
        .unwrap();
        // A zip names its parts in the clear: the native chart is in the file.
        assert!(
            written
                .bytes
                .windows(20)
                .any(|part| part == b"xl/charts/chart1.xml"),
            "{kind}"
        );
    }
}

#[test]
fn a_cell_looks_as_its_header_and_number_format_say() {
    assert_eq!(date_format("2026-01-31"), "yyyy-mm-dd");
    assert_eq!(date_format("2026-01-31 14:30"), "yyyy-mm-dd hh:mm:ss");
    assert_eq!(date_format("14:30"), "hh:mm:ss");
    let date = serde_json::json!({"date": "2026-01-31"});
    let formatted = |number: &str| Some(Format::new().set_num_format(number));
    assert_eq!(look(false, None, &date), formatted("yyyy-mm-dd"));
    assert_eq!(look(false, Some("d mmm"), &date), formatted("d mmm"));
    assert_eq!(look(false, None, &serde_json::json!(1)), None);
    let header = Format::new()
        .set_bold()
        .set_border_bottom(FormatBorder::Thin);
    assert_eq!(
        look(true, Some("0%"), &serde_json::json!("Share")),
        Some(header.set_num_format("0%"))
    );
}
