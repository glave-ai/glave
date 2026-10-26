//! `glave doc` end to end, as the model runs it through bash: a workbook written from a
//! description on standard input and read back, glave's own folder never touched, and a
//! mistake answered with the usage.

use glave_provider_openai::fake::FakeServer;

use crate::support::*;

#[test]
fn glave_doc_writes_and_reads_a_workbook_without_glaves_own_folder() {
    let place = place("doc");
    let model = FakeServer::start(vec![]).unwrap();
    let description = r#"{"sheets": [{"name": "Sales", "header": true,
        "rows": [["Region", "Revenue"], ["North", 1250.5], ["Total", "=SUM(B2:B2)"]]}]}"#;
    let written = glave(
        &place,
        &model,
        &["doc", "write", "sales.xlsx", "-"],
        description,
    );
    assert!(written.status.success(), "{}", text(&written));
    let read = glave(&place, &model, &["doc", "read", "sales.xlsx"], "");
    let shown = text(&read);
    assert!(read.status.success(), "{shown}");
    assert!(shown.contains("| 2 | North | 1250.5 |"), "{shown}");
    assert!(
        shown.contains("| 3 | Total | =SUM(B2:B2) → not computed yet |"),
        "{shown}"
    );
    let csv = glave(&place, &model, &["doc", "csv", "sales.xlsx"], "");
    assert_eq!(
        String::from_utf8_lossy(&csv.stdout),
        "Region,Revenue\nNorth,1250.5\nTotal,\n"
    );
    let wrong = glave(&place, &model, &["doc", "open", "sales.xlsx"], "");
    assert_eq!(wrong.status.code(), Some(2), "{}", text(&wrong));
    assert!(
        text(&wrong).contains("glave doc read <file>"),
        "{}",
        text(&wrong)
    );
    assert!(
        !place.home.exists(),
        "glave doc keeps out of glave's own folder"
    );
    assert!(model.received().is_empty(), "no model is asked");
}
