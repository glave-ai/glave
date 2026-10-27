//! The XML an Office file's parts hold, read into a small tree for the readers. Each name is
//! written with the prefix Office's own files give its namespace (`w:p`, `a:t`, `r:id`),
//! whatever prefix the file declared, so a Strict file reads as a transitional one does. A part
//! that is not well-formed says where it breaks.

use std::borrow::Cow;
use std::fmt;

use quick_xml::escape::EscapeError;
use quick_xml::events::attributes::AttrError;
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::reader::NsReader;
use quick_xml::{Error, XmlVersion};

/// The namespaces the readers look in, each with the prefix Office writes for it: `""` for the
/// ones Office leaves unprefixed. Strict Open XML names the same ones apart.
const PREFIXES: [(&str, &str); 14] = [
    (
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
        "w",
    ),
    ("http://purl.oclc.org/ooxml/wordprocessingml/main", "w"),
    ("http://schemas.openxmlformats.org/drawingml/2006/main", "a"),
    ("http://purl.oclc.org/ooxml/drawingml/main", "a"),
    (
        "http://schemas.openxmlformats.org/presentationml/2006/main",
        "p",
    ),
    ("http://purl.oclc.org/ooxml/presentationml/main", "p"),
    (
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
        "r",
    ),
    (
        "http://purl.oclc.org/ooxml/officeDocument/relationships",
        "r",
    ),
    (
        "http://schemas.openxmlformats.org/markup-compatibility/2006",
        "mc",
    ),
    (
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "",
    ),
    ("http://purl.oclc.org/ooxml/spreadsheetml/main", ""),
    (
        "http://schemas.openxmlformats.org/package/2006/relationships",
        "",
    ),
    (
        "http://schemas.openxmlformats.org/package/2006/content-types",
        "",
    ),
    ("http://www.w3.org/XML/1998/namespace", "xml"),
];

/// An element: its name, its attributes and what it holds, in order.
#[derive(Debug, Default)]
pub(crate) struct Element {
    pub(crate) name: String,
    attributes: Vec<(String, String)>,
    children: Vec<Node>,
}

#[derive(Debug)]
enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    /// The attribute named `name` (`w:val`, `r:id`, `Target`).
    pub(crate) fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// The elements it holds, in order.
    pub(crate) fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|node| match node {
            Node::Element(element) => Some(element),
            Node::Text(_) => None,
        })
    }

    /// The first element it holds named `name`.
    pub(crate) fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|element| element.name == name)
    }

    /// The element reached through `path`, each name a child of the one before.
    pub(crate) fn at(&self, path: &[&str]) -> Option<&Element> {
        path.iter()
            .try_fold(self, |element, name| element.child(name))
    }

    /// Every element inside it named `name`, at any depth, in document order; not inside one
    /// already found.
    pub(crate) fn every<'a>(&'a self, name: &str, found: &mut Vec<&'a Element>) {
        for element in self.elements() {
            if element.name == name {
                found.push(element);
            } else {
                element.every(name, found);
            }
        }
    }

    /// All the text inside it, at any depth.
    pub(crate) fn text(&self) -> String {
        let mut text = String::new();
        self.gather(&mut text);
        text
    }

    fn gather(&self, text: &mut String) {
        for node in &self.children {
            match node {
                Node::Text(more) => text.push_str(more),
                Node::Element(element) => element.gather(text),
            }
        }
    }
}

/// Where a part's XML stops being well-formed, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Broken {
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) why: String,
    /// The XML around where it breaks, on one line.
    pub(crate) near: String,
}

impl fmt::Display for Broken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            line,
            column,
            why,
            near,
        } = self;
        write!(f, "at line {line}, column {column}, {why}. Near: {near}")
    }
}

/// The root element of `text`, the whole tree under it. It must be well-formed, as an Office
/// app needs each part to be: one root, every tag closed by its own, attributes quoted once
/// each, every prefix declared, only XML's entities.
pub(crate) fn parse(text: &str) -> Result<Element, Broken> {
    read(text, true)
}

/// That `text` is well-formed, as [`parse`] needs it to be, without keeping its tree: a
/// sheet's XML may hold a million cells.
pub(crate) fn check(text: &str) -> Result<(), Broken> {
    read(text, false).map(drop)
}

/// The root element of `text`, with the tree under it when `keep`.
fn read(text: &str, keep: bool) -> Result<Element, Broken> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = NsReader::from_str(text);
    reader.config_mut().check_comments = true;
    let mut open: Vec<Element> = Vec::new();
    let mut root = None;
    loop {
        let start = position(reader.buffer_position());
        let (namespace, event) = match reader.read_resolved_event() {
            Ok(read) => read,
            Err(error) => {
                let at = position(reader.error_position());
                return Err(broken(text, at, &said(&error)));
            }
        };
        let name = match &event {
            Event::Start(tag) | Event::Empty(tag) => Some(named(&namespace, tag)),
            _ => None,
        };
        let fail = |why: &str| broken(text, start, why);
        let opens = matches!(event, Event::Start(_));
        match event {
            Event::Start(tag) | Event::Empty(tag) => {
                if root.is_some() {
                    return Err(fail("a second root element: a part holds only one"));
                }
                let name = name
                    .transpose()
                    .map_err(|why| fail(&why))?
                    .unwrap_or_default();
                let attributes = attributes(&reader, &tag)
                    .map_err(|(at, why)| broken(text, start + 1 + at, &why))?;
                let element = Element {
                    name,
                    attributes: if keep { attributes } else { Vec::new() },
                    children: Vec::new(),
                };
                if opens {
                    open.push(element);
                } else {
                    close(element, &mut open, &mut root, keep);
                }
            }
            Event::End(_) => {
                if let Some(element) = open.pop() {
                    close(element, &mut open, &mut root, keep);
                }
            }
            Event::Text(content) => add(&content.xml10_content(), &mut open, keep, &fail)?,
            Event::CData(content) => add(&content.xml10_content(), &mut open, keep, &fail)?,
            Event::GeneralRef(reference) => {
                let character = entity(&reference).map_err(|why| fail(&why))?;
                add(&character, &mut open, keep, &fail)?;
            }
            Event::Eof => {
                if let Some(element) = open.last() {
                    let why = format!("<{}> is never closed", element.name);
                    return Err(broken(text, text.len(), &why));
                }
                return root.ok_or_else(|| broken(text, text.len(), "there is no root element"));
            }
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) | Event::DocType(_) => {}
        }
    }
}

/// `element`, finished: into the one holding it when the tree is kept, or the root.
fn close(element: Element, open: &mut [Element], root: &mut Option<Element>, keep: bool) {
    match open.last_mut() {
        Some(parent) if keep => parent.children.push(Node::Element(element)),
        Some(_) => {}
        None => *root = Some(element),
    }
}

/// Text, into the element holding it when the tree is kept: outside the root only blank space
/// may be.
fn add(
    text: &str,
    open: &mut [Element],
    keep: bool,
    fail: &dyn Fn(&str) -> Broken,
) -> Result<(), Broken> {
    if let Some(bad) = text
        .chars()
        .find(|c| *c < ' ' && !matches!(c, '\t' | '\n' | '\r'))
    {
        let code = u32::from(bad);
        return Err(fail(&format!(
            "the control character U+{code:04X}, which XML does not allow"
        )));
    }
    match open.last_mut() {
        Some(parent) if keep => match parent.children.last_mut() {
            Some(Node::Text(before)) => before.push_str(text),
            _ => parent.children.push(Node::Text(text.to_owned())),
        },
        Some(_) => {}
        None if text.trim().is_empty() => {}
        None => return Err(fail("text outside the root element")),
    }
    Ok(())
}

/// A tag's name in Office's own prefix for its namespace.
fn named(namespace: &ResolveResult<'_>, tag: &BytesStart<'_>) -> Result<String, String> {
    let written = tag.name();
    let local = tag.local_name();
    qualified(namespace, written.as_ref(), local.as_ref())
}

fn qualified(namespace: &ResolveResult<'_>, written: &str, local: &str) -> Result<String, String> {
    match namespace {
        ResolveResult::Bound(Namespace(uri)) => {
            Ok(match PREFIXES.iter().find(|(known, _)| known == uri) {
                Some((_, "")) => local.to_owned(),
                Some((_, prefix)) => format!("{prefix}:{local}"),
                None => written.to_owned(),
            })
        }
        ResolveResult::Unbound => Ok(written.to_owned()),
        ResolveResult::Unknown(prefix) => Err(format!(
            "the prefix {prefix}: in <{written}> is not declared: declare it with xmlns:{prefix} \
             as the root element does in parts like this one"
        )),
    }
}

/// A tag's attributes, each read and named as [`named`] names tags; the namespaces declared
/// are left out. What is wrong says where in the tag, counted from after its `<`.
fn attributes(
    reader: &NsReader<&[u8]>,
    tag: &BytesStart<'_>,
) -> Result<Vec<(String, String)>, (usize, String)> {
    let mut read = Vec::new();
    for attribute in tag.attributes() {
        let attribute = attribute.map_err(|error| misquoted(&error))?;
        let written = attribute.key.as_ref();
        if written == "xmlns" || written.starts_with("xmlns:") {
            continue;
        }
        let (namespace, local) = reader.resolver().resolve_attribute(attribute.key);
        let name = qualified(&namespace, written, local.as_ref()).map_err(|why| (0, why))?;
        let value = attribute
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|error| match error {
                Error::Escape(EscapeError::UnrecognizedEntity(_, name)) => (0, unknown(&name)),
                error => (0, said(&error)),
            })?;
        read.push((name, value.into_owned()));
    }
    Ok(read)
}

/// What is wrong with an attribute, and where in its tag.
fn misquoted(error: &AttrError) -> (usize, String) {
    let (at, why) = match *error {
        AttrError::ExpectedEq(at) => (at, "an attribute's name must be followed by ="),
        AttrError::ExpectedValue(at) => (at, "= must be followed by the attribute's value"),
        AttrError::UnquotedValue(at) => (at, "an attribute's value must be in quotes"),
        AttrError::ExpectedQuote(at, _) => (at, "an attribute's value has no closing quote"),
        AttrError::Duplicated(at, _) => (at, "an attribute is given twice in one tag"),
    };
    (at, why.to_owned())
}

/// What a reference stands for: `&amp;` or `&#160;`, never an entity XML doesn't define.
fn entity(reference: &BytesRef<'_>) -> Result<Cow<'static, str>, String> {
    if let Ok(Some(character)) = reference.resolve_char_ref() {
        return Ok(Cow::Owned(character.to_string()));
    }
    let name: &str = reference;
    quick_xml::escape::resolve_predefined_entity(name)
        .map(Cow::Borrowed)
        .ok_or_else(|| unknown(name))
}

/// That `&name;` is not one of XML's own five entities, which are all it has.
fn unknown(name: &str) -> String {
    format!(
        "&{name}; is not an entity XML knows: write the character itself, or its number (&#160; \
         for a non-breaking space); only &amp; &lt; &gt; &quot; and &apos; have names"
    )
}

/// quick-xml's error, in its own words but for the backticks it puts around a tag.
fn said(error: &dyn fmt::Display) -> String {
    error.to_string().replace('`', "")
}

fn position(at: u64) -> usize {
    usize::try_from(at).unwrap_or(usize::MAX)
}

/// The XML characters shown around where a part breaks, each way.
const NEAR: usize = 40;

/// Where `at` is in `text`, by line and column, with the text around it.
fn broken(text: &str, at: usize, why: &str) -> Broken {
    let at = floor(text, at.min(text.len()));
    let before = text.get(..at).unwrap_or_default();
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .count()
        + 1;
    let from = floor(text, at.saturating_sub(NEAR));
    let to = floor(text, at.saturating_add(NEAR).min(text.len()));
    let near = text
        .get(from..to)
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Broken {
        line,
        column,
        why: why.to_owned(),
        near,
    }
}

/// The nearest character boundary of `text` at or before `at`.
fn floor(text: &str, mut at: usize) -> usize {
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[cfg(test)]
mod tests;
