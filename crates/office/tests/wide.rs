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
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&written).unwrap()).unwrap();
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    for at in 0..zip.len() {
        let mut part = zip.by_index(at).unwrap();
        let name = part.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut part, &mut bytes).unwrap();
        if name == "xl/worksheets/sheet1.xml" {
            let text = String::from_utf8(bytes).unwrap().replace("<v>0</v>", "");
            assert!(!text.contains("<v>"), "{text}");
            bytes = text.into_bytes();
        }
        parts.push((name, bytes));
    }
    let path = dir.join("formulas.xlsx");
    let parts: Vec<(&str, &[u8])> = parts
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    zipped(&path, &parts);
    refused(&path, &["read", "csv"], "Sheet1");
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
