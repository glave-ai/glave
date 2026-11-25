//! Workbooks too big to hold, made tiny: cells far apart, whose ranges calamine would fill with
//! every cell between them, in xlsx and xlsb, by values or by formulas alone, and sheets that
//! each fit but not all together. Each is refused in plain words before it is held.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use glave_ports::testing::TempDir;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::workbooks::doc;

/// What `glave doc` says, refusing a workbook whose sheets span too much at `sheet`.
fn too_big(sheet: &str) -> String {
    format!(
        "the workbook is too big to read whole: its sheets span more than the 10000000 cells \
         glave reads, from each sheet's first cell to its last, at the sheet \"{sheet}\". Read \
         one sheet with --sheet, or open it in a spreadsheet app and copy the part you need to \
         a file of its own"
    )
}

/// Asserts each of `commands` refuses the workbook at `path`, at `sheet`.
fn refused(path: &Path, commands: &[&str], sheet: &str) {
    let path = path.to_string_lossy();
    for command in commands {
        let said = doc(&[command, &path]).unwrap_err().to_string();
        assert_eq!(said, too_big(sheet), "{command}");
    }
}

/// A zip of `parts` at `path`.
fn zipped(path: &Path, parts: &[(&str, &[u8])]) {
    let mut zip = ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, bytes) in parts {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn values_far_apart_are_refused_and_previewed_as_a_stream() {
    let dir = TempDir::new("office-wide").unwrap();
    let path = dir.join("wide.xlsx");
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet();
    sheet.write(0, 0, "first").unwrap();
    sheet.write(1_048_575, 16_383, "last").unwrap();
    book.save(&path).unwrap();
    refused(&path, &["read", "csv", "sql"], "Sheet1");
    // A preview reads the cells as a stream, holding only the first rows.
    let bytes = std::fs::read(&path).unwrap();
    let preview = glave_office::preview(&bytes, "application/octet-stream").unwrap();
    assert!(preview.markdown.contains("first"), "{}", preview.markdown);
}

/// Formulas with no values yet, which calamine holds in a range of their own.
#[test]
fn formulas_far_apart_are_refused() {
    let dir = TempDir::new("office-formulas").unwrap();
    let written = dir.join("written.xlsx");
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet();
    sheet.write_formula(0, 0, "=1+1").unwrap();
    sheet.write_formula(1_048_575, 16_383, "=2+2").unwrap();
    book.save(&written).unwrap();
    // The same workbook with no value computed for either formula.
    let path = dir.join("formulas.xlsx");
    rezipped(&written, &path, SHEET, SHEET, |old, out| {
        let text = String::from_utf8(old).unwrap().replace("<v>0</v>", "");
        assert!(!text.contains("<v>"), "{text}");
        out.write_all(text.as_bytes()).unwrap();
    });
    refused(&path, &["read", "csv"], "Sheet1");
}

/// Where rust_xlsxwriter writes the first sheet and the shared strings.
const SHEET: &str = "xl/worksheets/sheet1.xml";
const STRINGS: &str = "xl/sharedStrings.xml";

/// The workbook at `from` copied to `to`, its `part` written by `write` as `name` from what it
/// held, last, as a stream.
fn rezipped(
    from: &Path,
    to: &Path,
    part: &str,
    name: &str,
    write: impl FnOnce(Vec<u8>, &mut dyn std::io::Write),
) {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(from).unwrap()).unwrap();
    let mut out = ZipWriter::new(std::fs::File::create(to).unwrap());
    let mut old = Vec::new();
    for at in 0..zip.len() {
        let mut entry = zip.by_index(at).unwrap();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
        let entry = entry.name().unwrap().into_owned();
        if entry == part {
            old = bytes;
            continue;
        }
        out.start_file(entry, SimpleFileOptions::default()).unwrap();
        out.write_all(&bytes).unwrap();
    }
    out.start_file(name, SimpleFileOptions::default()).unwrap();
    write(old, &mut out);
    out.finish().unwrap();
}

/// A workbook of one sheet holding `text` in A1, saved at `path`.
fn one_string(path: &Path, text: &str) {
    let mut book = rust_xlsxwriter::Workbook::new();
    book.add_worksheet().write(0, 0, text).unwrap();
    book.save(path).unwrap();
}

/// One cell named again and again spans one cell, but each is kept: their number is spent too.
#[test]
fn a_cell_named_again_and_again_is_refused() {
    let dir = TempDir::new("office-again").unwrap();
    let written = dir.join("written.xlsx");
    one_string(&written, "x");
    let path = dir.join("again.xlsx");
    rezipped(&written, &path, SHEET, SHEET, |_, out| {
        let main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
        write!(out, "<worksheet xmlns=\"{main}\"><sheetData><row r=\"1\">").unwrap();
        let chunk = "<c r=\"A1\"><v>1</v></c>".repeat(100_000);
        for _ in 0..101 {
            out.write_all(chunk.as_bytes()).unwrap();
        }
        out.write_all(b"</row></sheetData></worksheet>").unwrap();
    });
    refused(&path, &["read"], "Sheet1");
}

/// calamine finds the shared strings whatever the case or slashes of their part's name, so a
/// part named otherwise is measured all the same.
#[test]
fn shared_strings_are_measured_whatever_their_part_is_called() {
    let dir = TempDir::new("office-strings").unwrap();
    let written = dir.join("written.xlsx");
    one_string(&written, "x");
    let path = dir.join("strings.xlsx");
    rezipped(
        &written,
        &path,
        STRINGS,
        "XL\\SharedStrings.XML",
        |_, out| {
            let main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
            write!(
                out,
                "<sst xmlns=\"{main}\" count=\"1\" uniqueCount=\"1\"><si><t>"
            )
            .unwrap();
            let chunk = "a".repeat(1 << 20);
            for _ in 0..130 {
                out.write_all(chunk.as_bytes()).unwrap();
            }
            out.write_all(b"</t></si></sst>").unwrap();
        },
    );
    let said = doc(&["read", &path.to_string_lossy()])
        .unwrap_err()
        .to_string();
    assert_eq!(
        said,
        "the workbook's text takes more than 128 MB unpacked, more than glave reads. Open it in \
         a spreadsheet app and copy the part you need to a file of its own"
    );
}

/// calamine reserves room for as many shared strings as the part says it holds before it
/// reads one, so what it says is checked first.
#[test]
fn a_workbook_saying_it_shares_too_many_strings_is_refused() {
    let dir = TempDir::new("office-count").unwrap();
    let written = dir.join("written.xlsx");
    one_string(&written, "x");
    for count in ["4000000000000", "1e12"] {
        let path = dir.join("count.xlsx");
        rezipped(&written, &path, STRINGS, STRINGS, |old, out| {
            let text = String::from_utf8(old).unwrap();
            let text = text.replace("uniqueCount=\"1\"", &format!("uniqueCount=\"{count}\""));
            assert!(text.contains(count), "{text}");
            out.write_all(text.as_bytes()).unwrap();
        });
        let said = doc(&["read", &path.to_string_lossy()])
            .unwrap_err()
            .to_string();
        assert_eq!(
            said,
            format!(
                "the workbook says it shares {count} strings, more than the 8388608 glave \
                 reads. Open it in a spreadsheet app and copy the part you need to a file of \
                 its own"
            )
        );
    }
}

/// One record of an xlsb part: its type and length, each written seven bits to a byte.
fn record(out: &mut Vec<u8>, kind: u16, body: &[u8]) {
    let low = u8::try_from(kind & 0x7F).unwrap();
    if kind < 0x80 {
        out.push(low);
    } else {
        out.push(low | 0x80);
        out.push(u8::try_from(kind >> 7).unwrap());
    }
    let mut length = body.len();
    loop {
        let byte = u8::try_from(length & 0x7F).unwrap();
        length >>= 7;
        if length == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
    out.extend_from_slice(body);
}

/// A string as xlsb writes one: its length in characters, then UTF-16.
fn wide(text: &str) -> Vec<u8> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut out = u32::try_from(units.len()).unwrap().to_le_bytes().to_vec();
    out.extend(units.iter().flat_map(|unit| unit.to_le_bytes()));
    out
}

/// An xlsb workbook of one sheet holding a number at each of `cells`.
fn xlsb(path: &Path, cells: &[(u32, u32)]) {
    let mut workbook = Vec::new();
    let mut sheet = [0u32.to_le_bytes(), 1u32.to_le_bytes()].concat();
    sheet.extend(wide("rId1"));
    sheet.extend(wide("Sheet1"));
    record(&mut workbook, 0x009C, &sheet); // BrtBundleSh
    record(&mut workbook, 0x0090, &[]); // BrtEndBundleShs
    record(&mut workbook, 0x009D, &[]); // BrtCalcProp, after the names
    let mut data = Vec::new();
    record(&mut data, 0x0081, &[]); // BrtBeginSheet
    let last = cells.iter().fold((0, 0), |(r, c), (row, column)| {
        (r.max(*row), c.max(*column))
    });
    let dimension = [0, last.0, 0, last.1].map(u32::to_le_bytes).concat();
    record(&mut data, 0x0094, &dimension); // BrtWsDim
    record(&mut data, 0x0091, &[]); // BrtBeginSheetData
    for (row, column) in cells {
        record(&mut data, 0x0000, &row.to_le_bytes()); // BrtRowHdr
        let mut cell = column.to_le_bytes().to_vec();
        cell.extend([0; 4]);
        cell.extend(1.5f64.to_le_bytes());
        record(&mut data, 0x0005, &cell); // BrtCellReal
    }
    record(&mut data, 0x0092, &[]); // BrtEndSheetData
    let rels = "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
                <Relationship Id=\"rId1\" Target=\"worksheets/sheet1.bin\"/></Relationships>";
    zipped(
        path,
        &[
            ("xl/workbook.bin", &workbook),
            ("xl/_rels/workbook.bin.rels", rels.as_bytes()),
            ("xl/worksheets/sheet1.bin", &data),
        ],
    );
}

#[test]
fn an_xlsb_workbook_is_read_by_cell_and_refused_alike() {
    let dir = TempDir::new("office-xlsb").unwrap();
    let small: PathBuf = dir.join("small.xlsb");
    xlsb(&small, &[(0, 0), (2, 1)]);
    assert_eq!(
        doc(&["csv", &small.to_string_lossy()]).unwrap(),
        "1.5,\n,\n,1.5\n"
    );
    let far = dir.join("far.xlsb");
    xlsb(&far, &[(0, 0), (1_048_575, 16_383)]);
    refused(&far, &["read", "csv", "sql"], "Sheet1");
}

/// `glave doc sql` and `read` keep every sheet at once, so the budget is the workbook's: three
/// sheets that each fit alone are refused together, and each is read alone.
#[test]
fn sheets_that_each_fit_are_refused_together() {
    let dir = TempDir::new("office-sheets").unwrap();
    let path = dir.join("sheets.xlsx");
    let mut book = rust_xlsxwriter::Workbook::new();
    for _ in 0..3 {
        let sheet = book.add_worksheet();
        sheet.write(0, 0, 1).unwrap();
        sheet.write(1_899, 1_899, 2).unwrap();
    }
    book.save(&path).unwrap();
    refused(&path, &["read", "sql"], "Sheet3");
    let path = path.to_string_lossy();
    let one = doc(&["csv", &path, "--sheet", "Sheet2"]).unwrap();
    assert_eq!(one.lines().count(), 1_900);
}

/// One shared string, which calamine copies into every cell that names it: a tiny file of
/// 9,000 cells naming one 32,000-character string would hold 288 MB of text.
#[test]
fn text_copied_into_many_cells_is_refused() {
    let dir = TempDir::new("office-text").unwrap();
    let path = dir.join("text.xlsx");
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet();
    let text = "a".repeat(32_000);
    for row in 0..9_000 {
        sheet.write(row, 0, &text).unwrap();
    }
    book.save(&path).unwrap();
    let path = path.to_string_lossy();
    for command in ["read", "csv", "sql"] {
        let said = doc(&[command, &path]).unwrap_err().to_string();
        assert_eq!(
            said,
            "the workbook's text takes more than 256 MB unpacked, more than glave reads. Open it \
             in a spreadsheet app and copy the part you need to a file of its own",
            "{command}"
        );
    }
}
