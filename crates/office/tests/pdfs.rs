//! `glave doc merge`, `pages` and `rotate` on PDFs made here with lopdf, as #646's spike made
//! them: each page says which it is, and the pages from the third on sit a level down in the
//! page tree, turned a quarter, taking their size, font and turn from the tree above them.

use std::path::Path;

use glave_ports::testing::TempDir;
use lopdf::content::{Content, Operation};
use lopdf::{
    Document, EncryptionState, EncryptionVersion, Object, ObjectId, Permissions, Stream, dictionary,
};

use crate::workbooks::doc;

/// A PDF of a page for each of `texts`, each page saying its text.
pub(crate) fn paged(texts: &[&str]) -> Document {
    let mut document = Document::with_version("1.5");
    let (root, lower) = (document.new_object_id(), document.new_object_id());
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let (mut top, mut below) = (Vec::new(), Vec::new());
    for (index, text) in texts.iter().enumerate() {
        let parent = if index < 2 { root } else { lower };
        let page = page(&mut document, parent, text);
        if index < 2 { &mut top } else { &mut below }.push(Object::from(page));
    }
    if !below.is_empty() {
        let count = i64::try_from(below.len()).unwrap();
        let tree = dictionary! {
            "Type" => "Pages", "Parent" => root, "Kids" => below, "Count" => count,
            "Rotate" => 90,
        };
        document.objects.insert(lower, tree.into());
        top.push(lower.into());
    }
    let tree = dictionary! {
        "Type" => "Pages", "Kids" => top, "Count" => i64::try_from(texts.len()).unwrap(),
        "MediaBox" => vec![0.into(), 0.into(), 300.into(), 200.into()],
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
    };
    document.objects.insert(root, tree.into());
    let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => root });
    document.trailer.set("Root", catalog);
    document
}

/// A page under `parent` that says `text`.
pub(crate) fn page(document: &mut Document, parent: ObjectId, text: &str) -> ObjectId {
    let operations = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), 24.into()]),
        Operation::new("Td", vec![20.into(), 100.into()]),
        Operation::new("Tj", vec![Object::string_literal(text)]),
        Operation::new("ET", vec![]),
    ];
    let content = Content { operations }.encode().unwrap();
    let content = document.add_object(Stream::new(dictionary! {}, content));
    document.add_object(dictionary! { "Type" => "Page", "Parent" => parent, "Contents" => content })
}

/// `document` saved as a PDF at `path`.
pub(crate) fn saved(mut document: Document, path: &Path) -> String {
    document.save(path).unwrap();
    path.to_string_lossy().into_owned()
}

/// What each page of the PDF at `path` says, with how far it is turned, the page tree above
/// it counted.
pub(crate) fn pages(path: &str) -> Vec<(String, i64)> {
    let document = Document::load(path).unwrap();
    let turned = |page: ObjectId| {
        let mut node = document.get_dictionary(page).ok();
        while let Some(dictionary) = node {
            if let Ok(turn) = dictionary.get(b"Rotate").and_then(Object::as_i64) {
                return turn;
            }
            let parent = dictionary.get(b"Parent").and_then(Object::as_reference);
            node = parent.ok().and_then(|id| document.get_dictionary(id).ok());
        }
        0
    };
    document
        .page_iter()
        .map(|page| {
            let content = Content::decode(&document.get_page_content(page)).unwrap();
            let said = content.operations.iter().filter(|o| o.operator == "Tj");
            let said = said.map(|o| String::from_utf8_lossy(o.operands[0].as_str().unwrap()));
            (said.collect(), turned(page))
        })
        .collect()
}

fn said(pages: &[(&str, i64)]) -> Vec<(String, i64)> {
    pages
        .iter()
        .map(|(text, turn)| ((*text).to_owned(), *turn))
        .collect()
}

#[test]
fn pdfs_merge_each_ones_pages_after_the_one_befores() {
    let dir = TempDir::new("office-pdf-merge").unwrap();
    let a = saved(paged(&["A1", "A2", "A3"]), &dir.join("a.pdf"));
    let b = saved(paged(&["B1", "B2"]), &dir.join("b.pdf"));
    let all = dir.join("all.pdf").to_string_lossy().into_owned();
    let wrote = doc(&["merge", &all, &a, &b]).unwrap();
    assert_eq!(wrote, format!("Wrote {all}: 5 pages, {a}'s 3 then {b}'s 2"));
    assert_eq!(
        pages(&all),
        said(&[("A1", 0), ("A2", 0), ("A3", 90), ("B1", 0), ("B2", 0)])
    );
    // Each page took its size and font from its own tree, which is gone.
    let merged = Document::load(&all).unwrap();
    for page in merged.page_iter() {
        let page = merged.get_dictionary(page).unwrap();
        assert!(page.has(b"MediaBox") && page.has(b"Resources"));
    }
}

#[test]
fn pages_are_kept_in_the_order_named_and_the_rest_leave_the_file() {
    let dir = TempDir::new("office-pdf-pages").unwrap();
    let a = saved(paged(&["P1", "P2", "P3", "P4"]), &dir.join("a.pdf"));
    let part = dir.join("part.pdf").to_string_lossy().into_owned();
    let wrote = doc(&["pages", &a, &part, "3, 1-2"]).unwrap();
    assert_eq!(
        wrote,
        format!("Wrote {part}: {a}'s pages 3,1-2, 3 of its 4")
    );
    assert_eq!(pages(&part), said(&[("P3", 90), ("P1", 0), ("P2", 0)]));
    let bytes = std::fs::read(&part).unwrap();
    assert!(
        !bytes.windows(4).any(|window| window == b"(P4)"),
        "page 4 is out of the file"
    );
    let backwards = dir.join("backwards.pdf").to_string_lossy().into_owned();
    doc(&["pages", &a, &backwards, "4-3"]).unwrap();
    assert_eq!(pages(&backwards), said(&[("P4", 90), ("P3", 90)]));
}

#[test]
fn pages_turn_clockwise_or_back_and_every_page_when_none_are_named() {
    let dir = TempDir::new("office-pdf-rotate").unwrap();
    let a = saved(paged(&["R1", "R2", "R3"]), &dir.join("a.pdf"));
    let one = dir.join("one.pdf").to_string_lossy().into_owned();
    let wrote = doc(&["rotate", &a, &one, "90", "2"]).unwrap();
    assert_eq!(
        wrote,
        format!("Wrote {one}: pages 2 of 3 turned 90 degrees clockwise")
    );
    assert_eq!(pages(&one), said(&[("R1", 0), ("R2", 90), ("R3", 90)]));
    // The file itself, turned back with --replace.
    let wrote = doc(&["rotate", &one, &one, "-90", "--replace"]).unwrap();
    assert!(
        wrote.ends_with(": all 3 pages turned 90 degrees anticlockwise"),
        "{wrote}"
    );
    assert_eq!(pages(&one), said(&[("R1", 270), ("R2", 0), ("R3", 0)]));
}

#[test]
fn a_new_pdf_never_writes_over_a_file_unless_asked() {
    let dir = TempDir::new("office-pdf-replace").unwrap();
    let a = saved(paged(&["K1", "K2"]), &dir.join("a.pdf"));
    let mine = dir.join("mine.pdf");
    std::fs::write(&mine, "mine").unwrap();
    let mine = mine.to_string_lossy().into_owned();
    for words in [
        &["pages", &a, &mine, "1"][..],
        &["rotate", &a, &mine, "180"],
        &["merge", &mine, &a, &a],
    ] {
        let kept = doc(words).unwrap_err().to_string();
        assert!(kept.contains("is already there. Add --replace"), "{kept}");
        assert_eq!(std::fs::read(&mine).unwrap(), b"mine");
    }
    doc(&["merge", &mine, &a, &a, "--replace"]).unwrap();
    assert_eq!(pages(&mine).len(), 4);
}

#[test]
fn what_is_wrong_is_said_plainly() {
    let dir = TempDir::new("office-pdf-wrong").unwrap();
    let a = saved(paged(&["W1", "W2", "W3"]), &dir.join("a.pdf"));
    let out = dir.join("out.pdf").to_string_lossy().into_owned();
    let wrong = |words: &[&str]| doc(words).unwrap_err().to_string();
    assert_eq!(
        wrong(&["pages", &a, &out, "2-7"]),
        format!("{a} has 3 pages, so it has no page 7")
    );
    assert_eq!(
        wrong(&["pages", &a, &out, "1,2,1"]),
        "page 1 is named twice: glave doc pages keeps each page once"
    );
    for (words, said) in [
        (
            &["pages", &a, &out, "first"][..],
            "glave doc pages takes pages like 1-3,7",
        ),
        (
            &["rotate", &a, &out, "45"],
            "glave doc rotate turns pages by 90, 180 or 270",
        ),
        (
            &["merge", &out, &a],
            "glave doc merge was not given what it takes",
        ),
        (
            &["merge", "all", &a, &a],
            "glave doc merge makes a new PDF: name it first",
        ),
    ] {
        let error = wrong(words);
        assert!(error.starts_with(said), "{words:?}: {error}");
    }
    // The new PDF named last by mistake: the first PDF is never written over.
    let swapped = wrong(&["merge", &a, &out, "all.pdf"]);
    assert!(
        swapped.ends_with("is already there. Add --replace to write over it, or name a new file")
    );
    let text = dir.join("notes.pdf");
    std::fs::write(&text, "just some notes").unwrap();
    let text = text.to_string_lossy().into_owned();
    assert_eq!(
        wrong(&["rotate", &text, &out, "90"]),
        format!("{text} is not a PDF")
    );
    let damaged = dir.join("damaged.pdf");
    std::fs::write(&damaged, "%PDF-1.7\n1 0 obj << /Type /Catalog").unwrap();
    let damaged = damaged.to_string_lossy().into_owned();
    assert!(
        wrong(&["rotate", &damaged, &out, "90"])
            .starts_with(&format!("{damaged} could not be read as a PDF: "))
    );
    assert!(!Path::new(&out).exists());
    let read = wrong(&["read", &a]);
    assert!(
        read.ends_with(
            "a.pdf is a PDF, which the read tool shows as it is; glave doc merge, \
                        pages, rotate, fields and fill change PDFs"
        ),
        "{read}"
    );
}

/// `document` encrypted, opening with `user` as its password, or with none when it is empty.
pub(crate) fn encrypted(mut document: Document, user: &str) -> Document {
    let id = Object::string_literal("0123456789abcdef");
    document.trailer.set("ID", vec![id.clone(), id]);
    let version = EncryptionVersion::V2 {
        document: &document,
        owner_password: "owner",
        user_password: user,
        key_length: 128,
        permissions: Permissions::PRINTABLE,
    };
    let state = EncryptionState::try_from(version).unwrap();
    document.encrypt(&state).unwrap();
    document
}

#[test]
fn an_encrypted_pdf_is_never_changed() {
    let dir = TempDir::new("office-pdf-encrypted").unwrap();
    let out = dir.join("out.pdf").to_string_lossy().into_owned();
    let locked = saved(encrypted(paged(&["E1"]), "locked"), &dir.join("locked.pdf"));
    let guarded = saved(encrypted(paged(&["E1"]), ""), &dir.join("guarded.pdf"));
    for (file, why) in [
        (&locked, "is encrypted and needs a password to open"),
        (&guarded, "is encrypted by its maker against changes"),
    ] {
        let refused = doc(&["rotate", file, &out, "90"]).unwrap_err().to_string();
        assert_eq!(
            refused,
            format!(
                "{file} {why}: glave doesn't change encrypted PDFs or take their protection \
                 off. Ask for a copy that isn't encrypted"
            )
        );
    }
    assert!(!Path::new(&out).exists());
}
