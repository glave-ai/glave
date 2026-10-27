use glave_ports::testing::TempDir;
use zip::write::SimpleFileOptions;

use super::*;
use crate::package::tests::{rels, types};

const MAIN: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";

/// A Word document as Word zips one, but for a picture it stores rather than deflates and a
/// folder of its own, as some zips list.
fn document() -> Vec<u8> {
    let doc = "word/document.xml";
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.add_directory("word/", SimpleFileOptions::default())
        .unwrap();
    let parts: [(&str, &str, CompressionMethod); 4] = [
        (
            CONTENT_TYPES,
            &types(doc, MAIN),
            CompressionMethod::Deflated,
        ),
        (
            "_rels/.rels",
            &rels(&[("rId1", "officeDocument", doc)]),
            CompressionMethod::Deflated,
        ),
        (
            doc,
            "<?xml version=\"1.0\"?>\r\n<w:document xmlns:w=\"w\"><w:body><w:p><w:r><w:t>Hi\
             </w:t></w:r></w:p></w:body></w:document>",
            CompressionMethod::Deflated,
        ),
        (
            "word/media/image1.xml",
            "<picture/>",
            CompressionMethod::Stored,
        ),
    ];
    for (name, text, method) in parts {
        let options = SimpleFileOptions::default().compression_method(method);
        zip.start_file(name, options).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// Each part of the zip in `bytes`: its name, how it was zipped and what it holds.
fn contents(bytes: &[u8]) -> Vec<(String, CompressionMethod, Vec<u8>)> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..zip.len())
        .filter_map(|index| {
            let mut part = zip.by_index(index).unwrap();
            let mut held = Vec::new();
            part.read_to_end(&mut held).unwrap();
            let name = part.name().unwrap().into_owned();
            (!part.is_dir()).then_some((name, part.compression(), held))
        })
        .collect()
}

#[test]
fn a_file_unpacked_and_packed_untouched_comes_back_as_it_was() {
    let dir = TempDir::new("office-round-trip").unwrap();
    let (folder, out) = (dir.join("doc"), dir.join("doc.docx"));
    let file = Path::new("doc.docx");
    let said = unpack(&document(), file, &folder).unwrap();
    assert_eq!(
        said,
        format!(
            "Unpacked doc.docx into {}: 4 parts. The text is in word/document.xml.\n\
             When it is edited, pack it back: glave doc pack {} doc.docx --replace",
            folder.display(),
            folder.display()
        )
    );
    let laid_out = std::fs::read_to_string(folder.join("word/document.xml")).unwrap();
    assert!(
        laid_out.contains("<w:body>\n <w:p>\n  <w:r>\n   <w:t>Hi</w:t>"),
        "{laid_out}"
    );
    let packed = pack(&folder, &out, false).unwrap();
    assert!(
        packed.starts_with(&format!("Packed {} into ", folder.display())),
        "{packed}"
    );
    assert_eq!(
        contents(&std::fs::read(&out).unwrap()),
        contents(&document())
    );
}

#[test]
fn packing_keeps_the_order_unpacking_wrote_and_puts_new_parts_after() {
    let dir = TempDir::new("office-order").unwrap();
    std::fs::write(
        dir.join(ORDER),
        "# order\nstored b.xml\ndeflated [Content_Types].xml\ndeflated gone.xml\nstored a.bin\n",
    )
    .unwrap();
    let found: BTreeSet<String> = [
        "a.bin",
        "b.xml",
        "new/z.xml",
        "[Content_Types].xml",
        "c.xml",
    ]
    .map(str::to_owned)
    .into();
    let order = ordered(&dir, found).unwrap();
    let names: Vec<(&str, bool)> = order.iter().map(|(name, s)| (name.as_str(), *s)).collect();
    assert_eq!(
        names,
        [
            (CONTENT_TYPES, false),
            ("b.xml", true),
            ("a.bin", true),
            ("c.xml", false),
            ("new/z.xml", false),
        ]
    );
}

#[test]
fn parts_never_land_outside_the_folder() {
    let folder = Path::new("out");
    assert_eq!(
        inside(folder, "word/_rels/.rels"),
        Some(folder.join("word").join("_rels").join(".rels"))
    );
    for wrong in ["../x.xml", "/etc/x", "a//b", "a/./b", "a\\b", "C:x", "a/"] {
        assert_eq!(inside(folder, wrong), None, "{wrong}");
    }
}

#[test]
fn a_folder_with_hidden_files_and_links_is_packed_only_from_its_own_parts() {
    let dir = TempDir::new("office-files").unwrap();
    std::fs::create_dir_all(dir.join("_rels")).unwrap();
    for name in ["_rels/.rels", ".DS_Store", ORDER, "a.xml"] {
        std::fs::write(dir.join(name), "x").unwrap();
    }
    let mut found = BTreeSet::new();
    files(&dir, "", &mut found).unwrap();
    assert_eq!(
        found.into_iter().collect::<Vec<_>>(),
        ["_rels/.rels", "a.xml"]
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.join("a.xml"), dir.join("b.xml")).unwrap();
        let mut found = BTreeSet::new();
        let link = files(&dir, "", &mut found).unwrap_err().to_string();
        assert!(link.starts_with("b.xml is a link"), "{link}");
    }
}
