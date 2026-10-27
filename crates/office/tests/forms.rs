//! `glave doc fields` and `fill` on a form made here with lopdf, as #646's spike made one, and
//! what the page commands do to a form's fields.

use glave_ports::testing::TempDir;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

use crate::pdfs::{encrypted, paged, pages, saved};
use crate::workbooks::{answer, doc};

/// A leave form as a form-making app saves one. On its first page: "Name" (described),
/// "Days" (at most 2 characters, 3 now), "Paid" (a checkbox), "Type" (radio buttons, Annual
/// or Sick), "Team" (a drop-down), "Total" (read-only), "Sign" (a signature) and
/// "Address.Street" (a field under another); on its second, "Notes" (several lines).
fn form() -> Document {
    let mut document = paged(&["Leave form", "Notes"]);
    let pages: Vec<ObjectId> = document.page_iter().collect();
    let (first, second) = (pages[0], pages[1]);
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let [yes, annual, sick] = ["Yes", "Annual", "Sick"].map(|on| looks(&mut document, on));
    let (kind, address) = (document.new_object_id(), document.new_object_id());
    let fields = [
        dictionary! { "FT" => "Tx", "T" => text("Name"), "TU" => text("Your full name") },
        dictionary! { "FT" => "Tx", "T" => text("Days"), "MaxLen" => 2, "V" => text("3") },
        dictionary! { "FT" => "Btn", "T" => text("Paid"), "V" => "Off", "AS" => "Off", "AP" => yes },
        dictionary! { "Parent" => kind, "AS" => "Off", "AP" => annual },
        dictionary! { "Parent" => kind, "AS" => "Off", "AP" => sick },
        dictionary! { "FT" => "Ch", "Ff" => 1 << 17, "T" => text("Team"), "Opt" => vec![text("Sales"), text("Support")] },
        dictionary! { "FT" => "Tx", "Ff" => 1, "T" => text("Total"), "V" => text("12") },
        dictionary! { "FT" => "Sig", "T" => text("Sign") },
        dictionary! { "FT" => "Tx", "T" => text("Street"), "Parent" => address },
    ];
    let mut widgets = Vec::new();
    for (y, fields) in (10..=170).rev().step_by(20).zip(fields) {
        widgets.push(widget(&mut document, first, y, fields));
    }
    let [name, days, paid, annual, sick, team, total, sign, street] = widgets[..] else {
        unreachable!("nine widgets")
    };
    let kids = |ids: &[ObjectId]| ids.iter().map(|id| Object::from(*id)).collect::<Vec<_>>();
    let radio = dictionary! {
        "FT" => "Btn", "Ff" => 1 << 15, "T" => text("Type"), "V" => "Off",
        "Kids" => kids(&[annual, sick]),
    };
    document.objects.insert(kind, radio.into());
    let parent = dictionary! { "T" => text("Address"), "Kids" => kids(&[street]) };
    document.objects.insert(address, parent.into());
    let notes = dictionary! { "FT" => "Tx", "Ff" => 1 << 12, "T" => text("Notes") };
    let notes = widget(&mut document, second, 100, notes);
    for (page, on) in [(first, kids(&widgets)), (second, kids(&[notes]))] {
        document.get_dictionary_mut(page).unwrap().set("Annots", on);
    }
    let form = dictionary! {
        "Fields" => kids(&[name, days, paid, kind, team, total, sign, address, notes]),
        "DA" => text("/Helv 0 Tf 0 g"),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => font } },
    };
    document.catalog_mut().unwrap().set("AcroForm", form);
    document
}

fn text(text: &str) -> Object {
    Object::string_literal(text)
}

/// A widget on `page` at `y`, with its field's own entries.
fn widget(document: &mut Document, page: ObjectId, y: i64, mut fields: Dictionary) -> ObjectId {
    fields.set("Type", "Annot");
    fields.set("Subtype", "Widget");
    fields.set(
        "Rect",
        vec![100.into(), y.into(), 250.into(), (y + 18).into()],
    );
    fields.set("P", page);
    document.add_object(fields)
}

/// A checkbox's or radio button's looks, on as `on` and off.
fn looks(document: &mut Document, on: &str) -> Dictionary {
    let drawn = |document: &mut Document| {
        let bounds = vec![0.into(), 0.into(), 10.into(), 10.into()];
        let form = dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => bounds };
        Object::from(document.add_object(Stream::new(form, Vec::new())))
    };
    let mut normal = Dictionary::new();
    normal.set(on, drawn(document));
    normal.set("Off", drawn(document));
    dictionary! { "N" => normal }
}

/// Each line `glave doc fields` gives after its first.
fn listed(path: &str) -> Vec<String> {
    doc(&["fields", path])
        .unwrap()
        .lines()
        .skip(1)
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_forms_fields_list_with_their_kinds_pages_and_values() {
    let dir = TempDir::new("office-pdf-fields").unwrap();
    let path = saved(form(), &dir.join("form.pdf"));
    let fields = doc(&["fields", &path]).unwrap();
    assert!(
        fields.starts_with(&format!(
            "{path}: 9 fields. glave doc fill takes them by these names:\n"
        )),
        "{fields}"
    );
    assert_eq!(
        listed(&path),
        [
            r#""Name" (text, page 1): empty. Described as "Your full name""#,
            r#""Days" (text, at most 2 characters, page 1): "3""#,
            r#""Paid" (checkbox, page 1): not ticked"#,
            r#""Type" (one of "Annual", "Sick", page 1): none chosen"#,
            r#""Team" (one of "Sales", "Support", page 1): empty"#,
            r#""Total" (text, read-only, page 1): "12""#,
            r#""Sign" (a signature, which glave can't fill, page 1): empty"#,
            r#""Address.Street" (text, page 1): empty"#,
            r#""Notes" (text of several lines, page 2): empty"#,
        ]
    );
    let plain = saved(paged(&["No form"]), &dir.join("plain.pdf"));
    assert_eq!(
        doc(&["fields", &plain]).unwrap(),
        format!("{plain} has no form fields to fill")
    );
}

#[test]
fn a_form_is_filled_by_name_and_shows_what_it_holds() {
    let dir = TempDir::new("office-pdf-fill").unwrap();
    let path = saved(form(), &dir.join("form.pdf"));
    let filled = dir.join("filled.pdf").to_string_lossy().into_owned();
    let values = r#"{"Name": "Sam Okafor", "Days": 5, "Paid": true, "Type": "Sick",
        "Team": "Support", "Address.Street": "1 Station Road", "Notes": "Back on\nMonday"}"#;
    let wrote = answer(&["fill", &path, &filled], values).unwrap().printed;
    assert_eq!(
        wrote,
        format!(
            "Wrote {filled}: 7 fields filled: Address.Street, Days, Name, Notes, Paid, Team, \
             Type. Check them: glave doc fields {filled}"
        )
    );
    assert_eq!(
        listed(&filled),
        [
            r#""Name" (text, page 1): "Sam Okafor". Described as "Your full name""#,
            r#""Days" (text, at most 2 characters, page 1): "5""#,
            r#""Paid" (checkbox, page 1): ticked"#,
            r#""Type" (one of "Annual", "Sick", page 1): "Sick""#,
            r#""Team" (one of "Sales", "Support", page 1): "Support""#,
            r#""Total" (text, read-only, page 1): "12""#,
            r#""Sign" (a signature, which glave can't fill, page 1): empty"#,
            r#""Address.Street" (text, page 1): "1 Station Road""#,
            r#""Notes" (text of several lines, page 2): "Back on\nMonday""#,
        ]
    );
    let document = Document::load(&filled).unwrap();
    let form = document
        .catalog()
        .unwrap()
        .get(b"AcroForm")
        .unwrap()
        .as_dict()
        .unwrap();
    assert!(form.get(b"NeedAppearances").unwrap().as_bool().unwrap());
    let shown = |name: &[u8]| {
        let found = document.objects.values().find(|object| {
            object
                .as_dict()
                .is_ok_and(|d| d.get(b"T").and_then(Object::as_str).ok() == Some(name))
        });
        found.unwrap().as_dict().unwrap().clone()
    };
    // The checkbox and the chosen radio button show as on; the other radio button as off.
    assert_eq!(
        shown(b"Paid").get(b"AS").unwrap().as_name().unwrap(),
        b"Yes"
    );
    let kids = shown(b"Type")
        .get(b"Kids")
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    let states: Vec<Vec<u8>> = kids
        .iter()
        .map(|kid| {
            let kid = document
                .get_dictionary(kid.as_reference().unwrap())
                .unwrap();
            kid.get(b"AS").unwrap().as_name().unwrap().to_vec()
        })
        .collect();
    assert_eq!(states, [b"Off".to_vec(), b"Sick".to_vec()]);
    // The name is drawn in its box, for viewers that show what the file holds.
    let looks = shown(b"Name")
        .get(b"AP")
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    let drawn = looks.get(b"N").unwrap().as_reference().unwrap();
    let drawn = document.get_object(drawn).unwrap().as_stream().unwrap();
    let content = String::from_utf8_lossy(&drawn.content).into_owned();
    assert!(content.contains("(Sam Okafor) Tj"), "{content}");
}

#[test]
fn a_wrong_name_or_value_writes_nothing_and_says_each() {
    let dir = TempDir::new("office-pdf-fill-wrong").unwrap();
    let path = saved(form(), &dir.join("form.pdf"));
    let filled = dir.join("filled.pdf");
    let out = filled.to_string_lossy().into_owned();
    let values = r#"{"Nmae": "Sam", "name": "Sam", "Paid": "maybe", "Total": "13",
        "Sign": "Sam", "Days": "123", "Team": "Marketing", "Type": 3}"#;
    let wrong = answer(&["fill", &path, &out], values)
        .unwrap_err()
        .to_string();
    assert_eq!(
        wrong,
        format!(
            "Nothing was written: no field is named \"Nmae\" or \"name\" (did you mean \
             \"Name\"?); the fields are \"Name\", \"Days\", \"Paid\", \"Type\", \"Team\", \
             \"Total\", \"Sign\", \"Address.Street\", \"Notes\"; \"Days\" takes at most 2 \
             characters, and \"123\" has 3; \"Paid\" is a checkbox: true ticks it and false \
             clears it; \"Sign\" is a signature, which glave can't fill; \"Team\" takes one of \
             \"Sales\", \"Support\"; \"Total\" is read-only; \"Type\" takes one of \"Annual\", \
             \"Sick\". glave doc fields {path} says what each field takes"
        )
    );
    assert!(!filled.exists());
    for (values, said) in [
        ("", "Nothing was written: no field values were given"),
        ("{}", "Nothing was written: the JSON names no fields"),
        (
            "[1, 2]",
            "Nothing was written: the values are not a JSON object",
        ),
    ] {
        let wrong = answer(&["fill", &path, &out], values)
            .unwrap_err()
            .to_string();
        assert!(wrong.starts_with(said), "{wrong}");
    }
    let plain = saved(paged(&["No form"]), &dir.join("plain.pdf"));
    let none = answer(&["fill", &plain, &out], r#"{"Name": "Sam"}"#).unwrap_err();
    assert_eq!(
        none.to_string(),
        format!("{plain} has no form fields to fill")
    );
}

#[test]
fn the_page_commands_keep_a_forms_fields_and_take_out_those_on_pages_left_out() {
    let dir = TempDir::new("office-pdf-form-pages").unwrap();
    let path = saved(form(), &dir.join("form.pdf"));
    let first = dir.join("first.pdf").to_string_lossy().into_owned();
    let wrote = doc(&["pages", &path, &first, "1"]).unwrap();
    assert!(
        wrote.ends_with(". The form fields on the pages left out went with them"),
        "{wrote}"
    );
    let kept = listed(&first);
    assert_eq!(kept.len(), 8);
    assert!(kept.iter().all(|line| !line.contains("Notes")), "{kept:?}");
    let other = saved(paged(&["Receipt"]), &dir.join("receipt.pdf"));
    let merged = dir.join("merged.pdf").to_string_lossy().into_owned();
    doc(&["merge", &merged, &other, &path]).unwrap();
    assert_eq!(pages(&merged).len(), 3);
    let fields = listed(&merged);
    assert_eq!(fields.len(), 9);
    assert_eq!(
        fields[0],
        r#""Name" (text, page 2): empty. Described as "Your full name""#
    );
}

#[test]
fn a_form_its_maker_protected_reads_but_never_fills() {
    let dir = TempDir::new("office-pdf-form-guarded").unwrap();
    let path = saved(encrypted(form(), ""), &dir.join("guarded.pdf"));
    assert_eq!(listed(&path).len(), 9);
    let out = dir.join("filled.pdf").to_string_lossy().into_owned();
    let refused = answer(&["fill", &path, &out], r#"{"Name": "Sam"}"#).unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("is encrypted by its maker against changes")
    );
}
