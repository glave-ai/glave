//! What Office needs of a package's parts beyond each being well-formed: a content type for
//! every part, and every part that the content types or a relationship name is there.

use super::Part;
use crate::error::OfficeError;
use crate::package::{CONTENT_TYPES, content_type, resolved};
use crate::xml::{self, Element};

/// That `parts` tie together: each has a content type, and each part `[Content_Types].xml` or
/// a relationship names is one of them.
pub(super) fn tied(parts: &[Part]) -> Result<(), OfficeError> {
    let missing = |name: &str| {
        !parts
            .iter()
            .any(|part| part.name.eq_ignore_ascii_case(name))
    };
    let types = parsed(parts, CONTENT_TYPES)?;
    if let Some(part) = parts
        .iter()
        .find(|part| part.name != CONTENT_TYPES && content_type(&types, &part.name).is_none())
    {
        return Err(untied(&format!(
            "{name} has no content type: add <Override PartName=\"/{name}\" ContentType=\"…\"/> \
             to {CONTENT_TYPES}, with the content type parts like it have",
            name = part.name
        )));
    }
    for named in types
        .elements()
        .filter(|element| element.name == "Override")
    {
        let name = named.attribute("PartName").unwrap_or_default();
        if missing(name.trim_start_matches('/')) {
            return Err(untied(&format!(
                "{CONTENT_TYPES} names {name}, which is not in the folder: take out its \
                 <Override>, or put the part back"
            )));
        }
    }
    for part in parts {
        let Some(folder) = relating(&part.name) else {
            continue;
        };
        let list = parsed(parts, &part.name)?;
        for relationship in list
            .elements()
            .filter(|element| element.name == "Relationship")
        {
            if relationship.attribute("TargetMode") == Some("External") {
                continue;
            }
            let target = resolved(folder, relationship.attribute("Target").unwrap_or_default());
            if missing(&target) {
                let id = relationship.attribute("Id").unwrap_or_default();
                return Err(untied(&format!(
                    "{} points {id} to {target}, which is not in the folder: take out the \
                     relationship and what uses {id}, or put the part back",
                    part.name
                )));
            }
        }
    }
    Ok(())
}

/// The folder whose parts a relationships part's targets are named from: `word` for
/// `word/_rels/document.xml.rels`, the package's root for `_rels/.rels`; `None` for a part
/// that is not one.
fn relating(name: &str) -> Option<&str> {
    let (folder, file) = name.rsplit_once('/')?;
    if !file.to_ascii_lowercase().ends_with(".rels") {
        return None;
    }
    match folder {
        "_rels" => Some(""),
        folder => folder.strip_suffix("/_rels"),
    }
}

/// The part named `name` in `parts`, read as XML; an empty element when it isn't there.
fn parsed(parts: &[Part], name: &str) -> Result<Element, OfficeError> {
    let Some(part) = parts.iter().find(|part| part.name == name) else {
        return Ok(Element::default());
    };
    let text = std::str::from_utf8(&part.bytes)
        .map_err(|_| untied(&format!("{name} is not XML in UTF-8")))?;
    xml::parse(text).map_err(|broken| OfficeError::Broken {
        part: name.to_owned(),
        broken: broken.to_string(),
    })
}

fn untied(why: &str) -> OfficeError {
    OfficeError::Parts(format!("{why}. Nothing was packed."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::tests::{rels, types};

    const SLIDE: &str = "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";

    fn part(name: &str, text: &str) -> Part {
        Part {
            name: name.to_owned(),
            bytes: text.as_bytes().to_vec(),
            stored: false,
        }
    }

    /// A deck's parts, but for what `leave` says to leave out, and with `more`.
    fn deck(leave: &str, more: &[&str]) -> Vec<Part> {
        let slide = "ppt/slides/slide1.xml";
        let mut parts = vec![
            part(CONTENT_TYPES, &types(slide, SLIDE)),
            part(
                "_rels/.rels",
                &rels(&[("rId1", "officeDocument", "ppt/a.xml")]),
            ),
            part("ppt/a.xml", "<p/>"),
            part(
                "ppt/_rels/a.xml.rels",
                &rels(&[("rId2", "slide", "slides/slide1.xml")]),
            ),
            part(slide, "<p:sld/>"),
            part(
                "ppt/slides/_rels/slide1.xml.rels",
                "<Relationships><Relationship Id=\"rId1\" Target=\"https://example.com/\" \
                 TargetMode=\"External\"/></Relationships>",
            ),
        ];
        parts.retain(|part| part.name != leave);
        parts.extend(more.iter().map(|name| part(name, "<x/>")));
        parts
    }

    fn untied_by(parts: &[Part]) -> String {
        tied(parts).unwrap_err().to_string()
    }

    #[test]
    fn parts_that_tie_together_pack() {
        assert!(tied(&deck("", &[])).is_ok());
    }

    #[test]
    fn what_office_would_find_missing_is_said() {
        let unnamed = untied_by(&deck("", &["ppt/media/a.png"]));
        assert!(
            unnamed.starts_with("ppt/media/a.png has no content type: add <Override"),
            "{unnamed}"
        );
        let gone = untied_by(&deck("ppt/slides/slide1.xml", &[]));
        assert!(
            gone.starts_with(
                "[Content_Types].xml names /ppt/slides/slide1.xml, which is not in the folder"
            ),
            "{gone}"
        );
        let mut parts = deck("", &[]);
        parts.retain(|part| part.name != "ppt/a.xml");
        let pointed = untied_by(&parts);
        assert_eq!(
            pointed,
            "_rels/.rels points rId1 to ppt/a.xml, which is not in the folder: take out the \
             relationship and what uses rId1, or put the part back. Nothing was packed."
        );
    }

    #[test]
    fn relationships_are_named_from_the_folder_they_describe() {
        assert_eq!(relating("_rels/.rels"), Some(""));
        assert_eq!(relating("word/_rels/document.xml.rels"), Some("word"));
        assert_eq!(
            relating("ppt/slides/_rels/slide1.xml.rels"),
            Some("ppt/slides")
        );
        assert_eq!(relating("word/document.xml"), None);
        assert_eq!(relating("word/a.rels"), None);
    }
}
