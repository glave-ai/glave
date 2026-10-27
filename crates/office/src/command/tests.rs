use super::*;
use crate::pdf::Ranges;

fn asked(words: &[&str]) -> Result<Asked, OfficeError> {
    parse(&words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>())
}

#[test]
fn the_words_say_what_is_asked() {
    assert_eq!(asked(&[]).unwrap(), Asked::Help);
    assert_eq!(asked(&["read", "--help"]).unwrap(), Asked::Help);
    assert_eq!(
        asked(&["read", "--sheet", "Q1 sales", "a.xlsx", "--rows", "101-200"]).unwrap(),
        Asked::Read(
            "a.xlsx".into(),
            Reading {
                sheet: Some("Q1 sales".into()),
                rows: Some((101, 200))
            }
        )
    );
    assert_eq!(
        asked(&["read", "a.xlsx", "--rows", "7"]).unwrap(),
        Asked::Read(
            "a.xlsx".into(),
            Reading {
                sheet: None,
                rows: Some((7, 7))
            }
        )
    );
    assert_eq!(
        asked(&["csv", "a.xlsx"]).unwrap(),
        Asked::Csv("a.xlsx".into(), None)
    );
    for (words, description, replace) in [
        (&["write", "out.XLSX"][..], None, false),
        (&["write", "out.XLSX", "-"], None, false),
        (
            &["write", "--replace", "out.XLSX", "a.json"],
            Some("a.json"),
            true,
        ),
    ] {
        let description = description.map(PathBuf::from);
        let out = "out.XLSX".into();
        let writing = Writing {
            out,
            description,
            replace,
        };
        assert_eq!(asked(words).unwrap(), Asked::Write(writing));
    }
    for wrong in [
        &["read"][..],
        &["read", "a.xlsx", "b.xlsx"],
        &["read", "a.xlsx", "--rows", "9-2"],
        &["read", "a.xlsx", "--rows", "0-2"],
        &["read", "a.xlsx", "--rows"],
        &["read", "a.xlsx", "--replace"],
        &["csv", "a.xlsx", "--rows", "1-2"],
        &["read", "a.xlsx", "--all"],
        &["write"],
        &["write", "book.json", "out.xlsx"],
        &["write", "out.xls"],
        &["write", "out.xlsx", "a.json", "b.json"],
        &["write", "out.xlsx", "--sheet", "Sales"],
        &["open", "a.xlsx"],
    ] {
        let error = asked(wrong).unwrap_err().to_string();
        assert!(error.ends_with(USAGE), "{wrong:?}: {error}");
    }
    let swapped = asked(&["write", "book.json", "out.xlsx"])
        .unwrap_err()
        .to_string();
    assert!(
        swapped.starts_with("glave doc write makes .xlsx workbooks: name the new one first"),
        "{swapped}"
    );
}

#[test]
fn unpacking_and_packing_take_a_file_and_a_folder() {
    assert_eq!(
        asked(&["unpack", "a.docx", "a"]).unwrap(),
        Asked::Unpack("a.docx".into(), "a".into())
    );
    for (words, replace) in [
        (&["pack", "a", "a.docx"][..], false),
        (&["pack", "--replace", "a", "a.docx"], true),
    ] {
        let packing = Packing {
            folder: "a".into(),
            out: "a.docx".into(),
            replace,
        };
        assert_eq!(asked(words).unwrap(), Asked::Pack(packing));
    }
    for wrong in [
        &["unpack", "a.docx"][..],
        &["unpack", "a.docx", "a", "--replace"],
        &["pack", "a"],
        &["pack", "a", "a.docx", "--sheet", "Sales"],
    ] {
        let error = asked(wrong).unwrap_err().to_string();
        assert!(error.ends_with(USAGE), "{wrong:?}: {error}");
    }
}

#[test]
fn the_pdf_commands_name_the_new_pdf_and_what_it_is_made_from() {
    let pdf = |out: &str, replace, change| {
        let out = out.into();
        Asked::Pdf(Making { out, replace }, change)
    };
    let ranges = |text| Ranges::parse(text).unwrap();
    let merged = vec![PathBuf::from("a.pdf"), PathBuf::from("b.pdf")];
    for (words, wanted) in [
        (
            &["merge", "all.pdf", "a.pdf", "b.pdf"][..],
            pdf("all.pdf", false, Change::Merge(merged)),
        ),
        (
            &["pages", "a.pdf", "b.pdf", "1-3,7", "--replace"],
            pdf(
                "b.pdf",
                true,
                Change::Pages("a.pdf".into(), ranges("1-3,7")),
            ),
        ),
        (
            &["rotate", "a.pdf", "b.pdf", "-90"],
            pdf("b.pdf", false, Change::Rotate("a.pdf".into(), -90, None)),
        ),
        (
            &["rotate", "a.pdf", "b.pdf", "180", "2-"],
            pdf(
                "b.pdf",
                false,
                Change::Rotate("a.pdf".into(), 180, Some(ranges("2-"))),
            ),
        ),
        (
            &["fill", "a.pdf", "b.pdf", "-"],
            pdf("b.pdf", false, Change::Fill("a.pdf".into(), None)),
        ),
        (
            &["fill", "a.pdf", "b.pdf", "values.json"],
            pdf(
                "b.pdf",
                false,
                Change::Fill("a.pdf".into(), Some("values.json".into())),
            ),
        ),
        (&["fields", "a.pdf"], Asked::Fields("a.pdf".into())),
    ] {
        assert_eq!(asked(words).unwrap(), wanted, "{words:?}");
    }
    for wrong in [
        &["merge", "all.pdf", "a.pdf"][..],
        &["merge", "all", "a.pdf", "b.pdf"],
        &["pages", "a.pdf", "b.pdf"],
        &["pages", "a.pdf", "b.pdf", "one"],
        &["rotate", "a.pdf", "b.pdf", "45"],
        &["rotate", "a.pdf", "b.pdf", "360"],
        &["rotate", "a.pdf", "b.pdf", "90", "1", "2"],
        &["fill", "a.pdf"],
        &["fields", "a.pdf", "--replace"],
        &["fields", "a.pdf", "--sheet", "Sales"],
    ] {
        let error = asked(wrong).unwrap_err().to_string();
        assert!(error.ends_with(USAGE), "{wrong:?}: {error}");
    }
}
