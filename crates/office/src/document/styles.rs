//! What a Word document's styles and numbering say of a paragraph: whether it is a heading,
//! and at what level, or an item of a list, numbered or bulleted.

use std::collections::BTreeMap;

use crate::xml::Element;

/// The deepest heading marked: Markdown's six.
const DEEPEST: usize = 6;

/// How many styles may be based one on another before the chain is taken as broken.
const CHAIN: usize = 16;

/// The deepest level of a list, as Word counts them from 0.
const LAST_LEVEL: usize = 8;

/// The paragraph styles, by id.
#[derive(Debug, Default)]
pub(super) struct Styles(BTreeMap<String, Style>);

#[derive(Debug, Default)]
struct Style {
    /// Its name as Word knows it, in lower case: `heading 1`, `title`, `list bullet`.
    name: String,
    based_on: Option<String>,
    /// The heading level it gives.
    outline: Option<usize>,
    /// The list it makes its paragraphs items of, and at which level.
    list: (Option<String>, Option<usize>),
}

/// A list a paragraph is an item of, and its level there, from 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct List {
    pub(super) id: String,
    pub(super) level: usize,
}

impl Styles {
    pub(super) fn read(part: Option<Element>) -> Self {
        let mut styles = BTreeMap::new();
        let Some(part) = part else {
            return Self(styles);
        };
        for style in part.elements().filter(|element| element.name == "w:style") {
            let Some(id) = style.attribute("w:styleId") else {
                continue;
            };
            let value = |name: &str| {
                style
                    .child(name)
                    .and_then(|value| value.attribute("w:val"))
                    .map(str::to_owned)
            };
            let style = Style {
                name: value("w:name").unwrap_or_default().to_lowercase(),
                based_on: value("w:basedOn"),
                outline: style.at(&["w:pPr", "w:outlineLvl"]).and_then(outline),
                list: numbered(style.at(&["w:pPr", "w:numPr"])),
            };
            styles.insert(id.to_owned(), style);
        }
        Self(styles)
    }

    /// The heading level the style `id` gives, from 1, through the styles it is based on.
    pub(super) fn heading(&self, id: Option<&str>) -> Option<usize> {
        let mut id = id;
        for _ in 0..CHAIN {
            let current = id?;
            let Some(style) = self.0.get(current) else {
                // A document with no styles part still names its headings by id.
                return named(&current.to_lowercase());
            };
            if let Some(level) = named(&style.name).or(style.outline) {
                return Some(level);
            }
            id = style.based_on.as_deref();
        }
        None
    }

    /// The list a paragraph of the style `id` with its own numbering `own` is an item of.
    pub(super) fn list(&self, id: Option<&str>, own: Option<&Element>) -> Option<List> {
        let (mut list, mut level) = numbered(own);
        let mut id = id;
        for _ in 0..CHAIN {
            let Some(style) = id.and_then(|id| self.0.get(id)) else {
                break;
            };
            list = list.or_else(|| style.list.0.clone());
            level = level.or(style.list.1);
            id = style.based_on.as_deref();
        }
        // A list of 0 is Word's way of saying a paragraph is in none.
        let id = list.filter(|id| id != "0")?;
        let level = level.unwrap_or(0).min(LAST_LEVEL);
        Some(List { id, level })
    }
}

/// The heading level a style's name gives: `title` 1, `heading 2` 2.
fn named(name: &str) -> Option<usize> {
    if name == "title" {
        return Some(1);
    }
    let level: usize = name.strip_prefix("heading")?.trim().parse().ok()?;
    (1..=9).contains(&level).then(|| level.min(DEEPEST))
}

/// The heading level an outline level gives, from 1: Word's levels count from 0, and its 9 is
/// body text.
pub(super) fn outline(element: &Element) -> Option<usize> {
    let level: usize = element.attribute("w:val")?.parse().ok()?;
    (level <= LAST_LEVEL).then(|| (level + 1).min(DEEPEST))
}

/// The list and level a `w:numPr` names, each if it does.
fn numbered(numbering: Option<&Element>) -> (Option<String>, Option<usize>) {
    let value = |name: &str| {
        numbering
            .and_then(|numbering| numbering.child(name))
            .and_then(|value| value.attribute("w:val"))
    };
    (
        value("w:numId").map(str::to_owned),
        value("w:ilvl").and_then(|level| level.parse().ok()),
    )
}

/// The numbering part: which lists count, and from where.
#[derive(Debug, Default)]
pub(super) struct Numbering {
    /// The numbering each list follows, by the list's id.
    follows: BTreeMap<String, String>,
    /// Where each numbered level starts, by its numbering and level; bulleted levels are not
    /// here.
    starts: BTreeMap<(String, usize), u32>,
}

impl Numbering {
    pub(super) fn read(part: Option<Element>) -> Self {
        let mut numbering = Self::default();
        let Some(part) = part else {
            return numbering;
        };
        for element in part.elements() {
            let value = |element: &Element, name: &str| {
                element
                    .child(name)
                    .and_then(|value| value.attribute("w:val"))
                    .map(str::to_owned)
            };
            match element.name.as_str() {
                "w:abstractNum" => {
                    let Some(id) = element.attribute("w:abstractNumId") else {
                        continue;
                    };
                    for level in element.elements().filter(|level| level.name == "w:lvl") {
                        let format = value(level, "w:numFmt");
                        if matches!(format.as_deref(), Some("bullet" | "none")) {
                            continue;
                        }
                        let at = level.attribute("w:ilvl").and_then(|at| at.parse().ok());
                        let start = value(level, "w:start").and_then(|start| start.parse().ok());
                        numbering
                            .starts
                            .insert((id.to_owned(), at.unwrap_or(0)), start.unwrap_or(1));
                    }
                }
                "w:num" => {
                    let id = element.attribute("w:numId").map(str::to_owned);
                    if let Some((id, follows)) = id.zip(value(element, "w:abstractNumId")) {
                        numbering.follows.insert(id, follows);
                    }
                }
                _ => {}
            }
        }
        numbering
    }

    /// Where `list`'s level starts counting; `None` when it is bulleted.
    pub(super) fn start(&self, list: &List) -> Option<u32> {
        let follows = self.follows.get(&list.id)?;
        self.starts.get(&(follows.clone(), list.level)).copied()
    }
}
