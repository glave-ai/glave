//! A PowerPoint deck (.pptx, .pptm, .potx, .ppsx) read as text, slide by slide in the deck's
//! order: each slide's title, its text bulleted as the slide shows it, its tables, and its
//! speaker notes.

use std::fmt::Write as _;

use crate::error::OfficeError;
use crate::package::Package;
use crate::table;
use crate::xml::Element;

/// The deepest level of a paragraph on a slide, as PowerPoint counts them from 0.
const LAST_LEVEL: usize = 8;

/// The deck whose main part is `main`, named `name`, as text.
pub(crate) fn read(
    package: &mut Package<'_>,
    main: &str,
    name: &str,
) -> Result<String, OfficeError> {
    let presentation = package.xml(main)?.unwrap_or_default();
    let related = package.related(main)?;
    // The slides in the deck's order, which their parts' names need not follow.
    let slides: Vec<String> = presentation
        .child("p:sldIdLst")
        .into_iter()
        .flat_map(Element::elements)
        .filter_map(|slide| {
            let id = slide.attribute("r:id")?;
            let relationship = related.iter().find(|relationship| relationship.id == id)?;
            relationship.target.clone()
        })
        .collect();
    let mut text = match slides.len() {
        1 => format!("{name}: 1 slide\n"),
        count => format!("{name}: {count} slides\n"),
    };
    for (index, part) in slides.iter().enumerate() {
        let Some(slide) = package.xml(part)? else {
            continue;
        };
        let mut read = Slide::default();
        if let Some(tree) = slide.at(&["p:cSld", "p:spTree"]) {
            read.shapes(tree);
        }
        let number = index + 1;
        let hidden = if slide.attribute("show") == Some("0") {
            " (hidden)"
        } else {
            ""
        };
        let title = read.title.join(" ");
        let _ = if title.is_empty() {
            writeln!(text, "\n## Slide {number}{hidden}")
        } else {
            writeln!(text, "\n## Slide {number}{hidden}: {title}")
        };
        for line in &read.lines {
            let _ = writeln!(text, "{line}");
        }
        let notes = notes(package, part)?;
        if !notes.is_empty() {
            let _ = writeln!(text, "Notes: {}", notes.join("\n"));
        }
    }
    Ok(text)
}

/// The speaker notes of the slide `part`, a paragraph to a line.
fn notes(package: &mut Package<'_>, part: &str) -> Result<Vec<String>, OfficeError> {
    let related = package.related(part)?;
    let Some(notes) = package.related_xml(&related, "notesSlide")? else {
        return Ok(Vec::new());
    };
    let mut shapes = Vec::new();
    notes.every("p:sp", &mut shapes);
    // The notes are the page's body; its other shapes are the slide's picture and number.
    Ok(shapes
        .into_iter()
        .filter(|shape| placeholder(shape) == Some("body"))
        .filter_map(|shape| shape.child("p:txBody"))
        .flat_map(|body| paragraphs(body, false))
        .collect())
}

/// What a slide shows, gathered shape by shape.
#[derive(Debug, Default)]
struct Slide {
    title: Vec<String>,
    lines: Vec<String>,
}

impl Slide {
    /// The shapes in `tree`, in the order the slide stacks them.
    fn shapes(&mut self, tree: &Element) {
        for shape in tree.elements() {
            match shape.name.as_str() {
                "p:sp" => self.shape(shape),
                "p:grpSp" => self.shapes(shape),
                "p:graphicFrame" => {
                    let mut tables = Vec::new();
                    shape.every("a:tbl", &mut tables);
                    for found in tables {
                        self.lines.extend(table(found));
                    }
                }
                "mc:AlternateContent" => {
                    if let Some(choice) = shape.child("mc:Choice") {
                        self.shapes(choice);
                    }
                }
                _ => {}
            }
        }
    }

    fn shape(&mut self, shape: &Element) {
        let Some(body) = shape.child("p:txBody") else {
            return;
        };
        match placeholder(shape) {
            Some("title" | "ctrTitle") => {
                self.title.extend(paragraphs(body, false));
            }
            // The date, footer, header and slide number the layout puts on every slide.
            Some("dt" | "ftr" | "hdr" | "sldNum") => {}
            // A body or content placeholder is bulleted unless a paragraph says otherwise.
            kind => {
                let bulleted = matches!(kind, Some("body" | "obj"));
                self.lines.extend(paragraphs(body, bulleted));
            }
        }
    }
}

/// The kind of placeholder `shape` is, if it is one: one with no type holds content (`obj`).
fn placeholder(shape: &Element) -> Option<&str> {
    let placeholder = shape.at(&["p:nvSpPr", "p:nvPr", "p:ph"])?;
    Some(placeholder.attribute("type").unwrap_or("obj"))
}

/// A text body's paragraphs, a line each, bulleted and indented by level as shown.
fn paragraphs(body: &Element, bulleted: bool) -> Vec<String> {
    body.elements()
        .filter(|element| element.name == "a:p")
        .filter_map(|paragraph| {
            let text = said(paragraph);
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            let properties = paragraph.child("a:pPr");
            let level = properties
                .and_then(|properties| properties.attribute("lvl"))
                .and_then(|level| level.parse::<usize>().ok())
                .unwrap_or(0)
                .min(LAST_LEVEL);
            let bullet = properties
                .and_then(|properties| {
                    properties
                        .elements()
                        .find_map(|bullet| match bullet.name.as_str() {
                            "a:buNone" => Some(false),
                            "a:buChar" | "a:buAutoNum" | "a:buBlip" => Some(true),
                            _ => None,
                        })
                })
                .unwrap_or(bulleted);
            let indent = "  ".repeat(level);
            Some(if bullet {
                format!("{indent}- {text}")
            } else {
                format!("{indent}{text}")
            })
        })
        .collect()
}

/// A paragraph's text: its runs and fields, with its line breaks.
fn said(paragraph: &Element) -> String {
    let mut text = String::new();
    for child in paragraph.elements() {
        match child.name.as_str() {
            "a:r" | "a:fld" => {
                if let Some(run) = child.child("a:t") {
                    text.push_str(&run.text());
                }
            }
            "a:br" => text.push('\n'),
            _ => {}
        }
    }
    text
}

/// A table on a slide, a row to a line; a cell merged into another holds nothing of its own.
fn table(table: &Element) -> Vec<String> {
    let merged = |cell: &Element, side: &str| matches!(cell.attribute(side), Some("1" | "true"));
    let rows: Vec<Vec<String>> = table
        .elements()
        .filter(|element| element.name == "a:tr")
        .map(|row| {
            row.elements()
                .filter(|element| element.name == "a:tc")
                .map(|cell| {
                    if merged(cell, "hMerge") || merged(cell, "vMerge") {
                        return String::new();
                    }
                    cell.child("a:txBody")
                        .map(|body| paragraphs(body, false).join("\n"))
                        .unwrap_or_default()
                })
                .collect()
        })
        .collect();
    table::lines(&rows)
}

#[cfg(test)]
mod tests;
