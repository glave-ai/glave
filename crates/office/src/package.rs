//! An Office file as what it is: a zip of parts (Open Packaging Conventions), each with a
//! content type in `[Content_Types].xml`, tied together by relationships (`_rels/*.rels`).

use std::io::{Cursor, Read};

use zip::ZipArchive;

use crate::error::OfficeError;
use crate::xml::{self, Element};

/// The list of every part's content type, the first part of every Office file.
pub(crate) const CONTENT_TYPES: &str = "[Content_Types].xml";

/// The largest part read: a document's XML is read whole, into a tree.
const MAX_PART: u64 = 64 * 1024 * 1024;

/// What an Office file is, from its main part's content type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// .docx, .docm, .dotx, .dotm
    Word,
    /// .pptx, .pptm, .potx, .ppsx
    PowerPoint,
    /// .xlsx, .xlsm and the rest calamine reads.
    Workbook,
}

impl Kind {
    /// What a file of this kind is, in words.
    pub(crate) fn said(self) -> &'static str {
        match self {
            Self::Word => "a Word document",
            Self::PowerPoint => "a PowerPoint deck",
            Self::Workbook => "a workbook",
        }
    }
}

/// An Office file, read part by part as asked.
pub(crate) struct Package<'a> {
    zip: ZipArchive<Cursor<&'a [u8]>>,
}

/// A relationship from one part to another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Relationship {
    pub(crate) id: String,
    /// The last word of its type: `officeDocument`, `slide`, `notesSlide`, `comments`.
    pub(crate) kind: String,
    /// The part it points to, named from the package's root (`word/styles.xml`); `None` for a
    /// target outside the file, such as a link.
    pub(crate) target: Option<String>,
}

impl<'a> Package<'a> {
    /// The file in `bytes`, if it is a zip with the content types an Office file starts with.
    pub(crate) fn open(bytes: &'a [u8]) -> Option<Self> {
        let zip = ZipArchive::new(Cursor::new(bytes)).ok()?;
        let package = Self { zip };
        package.index(CONTENT_TYPES).is_some().then_some(package)
    }

    /// Where the part `name` is, its case aside, as part names are compared.
    fn index(&self, name: &str) -> Option<usize> {
        self.zip.index_for_name(name).or_else(|| {
            self.zip
                .file_names()
                .position(|found| found.is_ok_and(|found| found.eq_ignore_ascii_case(name)))
        })
    }

    /// The part `name`'s bytes, uncompressed; `None` if the file has no such part.
    pub(crate) fn part(&mut self, name: &str) -> Result<Option<Vec<u8>>, OfficeError> {
        let Some(index) = self.index(name) else {
            return Ok(None);
        };
        let damaged = |why: String| OfficeError::Damaged(format!("{name}: {why}"));
        let part = self
            .zip
            .by_index(index)
            .map_err(|error| damaged(error.to_string()))?;
        let mut bytes = Vec::new();
        part.take(MAX_PART + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| damaged(error.to_string()))?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_PART {
            return Err(damaged(format!(
                "it is larger than the {MAX_PART} bytes glave reads of a part"
            )));
        }
        Ok(Some(bytes))
    }

    /// The part `name` read as XML; `None` if the file has no such part.
    pub(crate) fn xml(&mut self, name: &str) -> Result<Option<Element>, OfficeError> {
        let Some(bytes) = self.part(name)? else {
            return Ok(None);
        };
        let text = String::from_utf8(bytes)
            .map_err(|_| OfficeError::Damaged(format!("{name} is not XML in UTF-8")))?;
        xml::parse(&text)
            .map(Some)
            .map_err(|broken| OfficeError::Broken {
                part: name.to_owned(),
                broken: broken.to_string(),
            })
    }

    /// The relationships from `part` (`""` for the package itself), in the order written.
    pub(crate) fn related(&mut self, part: &str) -> Result<Vec<Relationship>, OfficeError> {
        let (folder, file) = part.rsplit_once('/').unwrap_or(("", part));
        let rels = if folder.is_empty() {
            format!("_rels/{file}.rels")
        } else {
            format!("{folder}/_rels/{file}.rels")
        };
        let Some(list) = self.xml(&rels)? else {
            return Ok(Vec::new());
        };
        Ok(list
            .elements()
            .filter(|element| element.name == "Relationship")
            .map(|relationship| {
                let kind = relationship.attribute("Type").unwrap_or_default();
                let target = relationship.attribute("Target").unwrap_or_default();
                let external = relationship.attribute("TargetMode") == Some("External");
                Relationship {
                    id: relationship.attribute("Id").unwrap_or_default().to_owned(),
                    kind: kind.rsplit('/').next().unwrap_or_default().to_owned(),
                    target: (!external).then(|| resolved(folder, target)),
                }
            })
            .collect())
    }

    /// The part the first of `related` of `kind` points to, read as XML.
    pub(crate) fn related_xml(
        &mut self,
        related: &[Relationship],
        kind: &str,
    ) -> Result<Option<Element>, OfficeError> {
        match related
            .iter()
            .find(|relationship| relationship.kind == kind)
            .and_then(|relationship| relationship.target.clone())
        {
            Some(target) => self.xml(&target),
            None => Ok(None),
        }
    }

    /// The parts the elements of `list` in the part `part` point to, in order, each with its
    /// element's `name`: a deck's slides from `p:sldIdLst`, a workbook's sheets from `sheets`.
    pub(crate) fn listed(
        &mut self,
        part: &str,
        list: &str,
    ) -> Result<Vec<(String, String)>, OfficeError> {
        let root = self.xml(part)?.unwrap_or_default();
        let related = self.related(part)?;
        Ok(root
            .child(list)
            .into_iter()
            .flat_map(Element::elements)
            .filter_map(|item| {
                let id = item.attribute("r:id")?;
                let relationship = related.iter().find(|relationship| relationship.id == id)?;
                let name = item.attribute("name").unwrap_or_default().to_owned();
                Some((name, relationship.target.clone()?))
            })
            .collect())
    }

    /// The main part, and what kind of file its content type makes this.
    pub(crate) fn main(&mut self) -> Result<Option<(String, Kind)>, OfficeError> {
        let Some(main) = self
            .related("")?
            .into_iter()
            .find(|relationship| relationship.kind == "officeDocument")
            .and_then(|relationship| relationship.target)
        else {
            return Ok(None);
        };
        let types = self.xml(CONTENT_TYPES)?.unwrap_or_default();
        let kind = match content_type(&types, &main).unwrap_or_default() {
            word if word.contains("wordprocessingml") || word.contains("ms-word") => Kind::Word,
            deck if deck.contains("presentationml") || deck.contains("ms-powerpoint") => {
                Kind::PowerPoint
            }
            _ => Kind::Workbook,
        };
        Ok(Some((main, kind)))
    }
}

/// The content type `[Content_Types].xml` gives the part `name`: its own, or its extension's.
pub(crate) fn content_type<'a>(types: &'a Element, name: &str) -> Option<&'a str> {
    let part = format!("/{name}");
    let extension = name.rsplit_once('.').map(|(_, extension)| extension);
    let given = |kind: &str, key: &str, wanted: Option<&str>| {
        types
            .elements()
            .filter(|element| element.name == kind)
            .find(|element| {
                element
                    .attribute(key)
                    .zip(wanted)
                    .is_some_and(|(found, wanted)| found.eq_ignore_ascii_case(wanted))
            })
            .and_then(|element| element.attribute("ContentType"))
    };
    given("Override", "PartName", Some(&part)).or_else(|| given("Default", "Extension", extension))
}

/// A relationship's target as a part's name from the package's root: `slides/slide1.xml` from
/// `ppt` is `ppt/slides/slide1.xml`, and `/word/a.xml` or `../word/a.xml` are `word/a.xml`.
pub(crate) fn resolved(folder: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        folder.split('/').filter(|part| !part.is_empty()).collect()
    };
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

#[cfg(test)]
pub(crate) mod tests;
