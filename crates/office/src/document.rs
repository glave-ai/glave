//! A Word document (.docx, .docm, .dotx) read as text in reading order: headings marked `#`,
//! list items `-` or numbered, tables as tables, each comment after the paragraph it is on, and
//! tracked changes as Word shows them with markup off: insertions read, deletions left out.

mod styles;

use std::collections::BTreeMap;

use crate::error::OfficeError;
use crate::package::Package;
use crate::table;
use crate::xml::Element;
use styles::{List, Numbering, Styles};

/// The most characters of commented text quoted beside its comment.
const QUOTED: usize = 60;

/// The most columns one cell is taken to span.
const MAX_SPAN: usize = 64;

/// The Word document whose main part is `main`, named `name`, as text.
pub(crate) fn read(
    package: &mut Package<'_>,
    main: &str,
    name: &str,
) -> Result<String, OfficeError> {
    let document = package.xml(main)?.unwrap_or_default();
    let related = package.related(main)?;
    let mut reader = Reader {
        styles: Styles::read(package.related_xml(&related, "styles")?),
        numbering: Numbering::read(package.related_xml(&related, "numbering")?),
        comments: comments(package.related_xml(&related, "comments")?),
        ..Reader::default()
    };
    // Headers and footers, each said once though each section may repeat it.
    let mut margins: Vec<String> = Vec::new();
    for relationship in &related {
        let place = match relationship.kind.as_str() {
            "header" => "Header",
            "footer" => "Footer",
            _ => continue,
        };
        let Some(part) = relationship.target.as_deref() else {
            continue;
        };
        let mut lines = Vec::new();
        if let Some(part) = package.xml(part)? {
            reader.blocks(&part, &mut lines);
        }
        let line = format!("{place}: {}", lines.join(" / "));
        if !lines.is_empty() && !margins.contains(&line) {
            margins.push(line);
        }
    }
    let mut body = Vec::new();
    if let Some(content) = document.child("w:body") {
        reader.blocks(content, &mut body);
    }
    let mut text = format!("{name}: a Word document");
    if reader.tracked {
        text.push_str(
            " with tracked changes not yet accepted: insertions read as text here, and deletions \
             are left out",
        );
    }
    for line in margins.iter().chain(&body) {
        text.push('\n');
        text.push_str(line);
    }
    text.push('\n');
    Ok(text)
}

/// A comment: who made it, and what it says.
#[derive(Debug)]
struct Comment {
    author: Option<String>,
    text: String,
}

/// Every comment in the comments part, by its id.
fn comments(part: Option<Element>) -> BTreeMap<String, Comment> {
    let Some(part) = part else {
        return BTreeMap::new();
    };
    part.elements()
        .filter(|element| element.name == "w:comment")
        .filter_map(|comment| {
            let mut lines = Vec::new();
            Reader::default().blocks(comment, &mut lines);
            let said = Comment {
                author: comment.attribute("w:author").map(str::to_owned),
                text: lines.join(" "),
            };
            Some((comment.attribute("w:id")?.to_owned(), said))
        })
        .collect()
}

/// What a paragraph says, gathered run by run.
#[derive(Debug, Default)]
struct Said {
    text: String,
    /// The comments it carries, by id.
    comments: Vec<String>,
    /// The lines of text boxes anchored in it, read after it.
    boxes: Vec<String>,
}

/// Reads a part's paragraphs and tables, keeping count as it goes.
#[derive(Debug, Default)]
struct Reader {
    styles: Styles,
    numbering: Numbering,
    comments: BTreeMap<String, Comment>,
    /// How far each list has counted at each of its levels.
    counted: BTreeMap<(String, usize), u32>,
    /// The comments whose ranges are open, with the text they cover so far.
    covering: BTreeMap<String, String>,
    /// The text each closed comment range covered.
    covered: BTreeMap<String, String>,
    /// Whether any tracked change was met.
    tracked: bool,
}

impl Reader {
    /// The paragraphs and tables in `parent`, a line or more each.
    fn blocks(&mut self, parent: &Element, lines: &mut Vec<String>) {
        for child in parent.elements() {
            match child.name.as_str() {
                "w:p" => self.paragraph(child, lines),
                "w:tbl" => self.table(child, lines),
                // Content controls and custom markup hold paragraphs of their own.
                "w:sdt" | "w:sdtContent" | "w:customXml" => self.blocks(child, lines),
                _ => {}
            }
        }
    }

    fn paragraph(&mut self, paragraph: &Element, lines: &mut Vec<String>) {
        let mut said = Said::default();
        self.inline(paragraph, &mut said);
        for covered in self.covering.values_mut() {
            covered.push(' ');
        }
        let text = said.text.trim();
        if !text.is_empty() {
            lines.push(match self.marker(paragraph) {
                Some(marker) => format!("{marker} {text}"),
                None => text.to_owned(),
            });
        }
        lines.append(&mut said.boxes);
        lines.extend(said.comments.iter().filter_map(|id| self.comment(id)));
    }

    /// What `element` says inside a paragraph, in order.
    fn inline(&mut self, element: &Element, said: &mut Said) {
        for child in element.elements() {
            let id = child.attribute("w:id");
            match child.name.as_str() {
                "w:t" => self.say(said, &child.text()),
                "w:tab" | "w:ptab" => self.say(said, "\t"),
                "w:br" | "w:cr" => {
                    // A page or column break starts no new line of text.
                    if matches!(child.attribute("w:type"), None | Some("textWrapping")) {
                        self.say(said, "\n");
                    }
                }
                "w:noBreakHyphen" => self.say(said, "-"),
                "w:del" | "w:moveFrom" => self.tracked = true,
                "w:ins" | "w:moveTo" => {
                    self.tracked = true;
                    self.inline(child, said);
                }
                "w:commentRangeStart" => {
                    if let Some(id) = id {
                        self.covering.insert(id.to_owned(), String::new());
                    }
                }
                "w:commentRangeEnd" => {
                    if let Some((id, text)) = id.and_then(|id| self.covering.remove_entry(id)) {
                        self.covered.insert(id, text);
                    }
                }
                "w:commentReference" => said.comments.extend(id.map(str::to_owned)),
                "w:txbxContent" => self.blocks(child, &mut said.boxes),
                // Properties, field codes, deleted text, and the copy of a drawing kept for
                // apps that can't show it.
                "w:pPr" | "w:rPr" | "w:instrText" | "w:delText" | "w:delInstrText"
                | "mc:Fallback" => {}
                // Runs, links, fields, smart tags, content controls and drawings hold text.
                _ => self.inline(child, said),
            }
        }
    }

    /// `text` said, and covered by every comment range open.
    fn say(&mut self, said: &mut Said, text: &str) {
        said.text.push_str(text);
        for covered in self.covering.values_mut() {
            if covered.chars().count() <= QUOTED {
                covered.push_str(text);
            }
        }
    }

    /// How a paragraph is marked: `#` for a heading, by its level, `-` or a number for a
    /// list item, indented by its level; `None` for plain text.
    fn marker(&mut self, paragraph: &Element) -> Option<String> {
        let properties = paragraph.child("w:pPr");
        let style = properties
            .and_then(|properties| properties.child("w:pStyle"))
            .and_then(|style| style.attribute("w:val"));
        let outline = properties
            .and_then(|properties| properties.child("w:outlineLvl"))
            .and_then(styles::outline);
        if let Some(level) = outline.or_else(|| self.styles.heading(style)) {
            return Some("#".repeat(level));
        }
        let own = properties.and_then(|properties| properties.child("w:numPr"));
        let list = self.styles.list(style, own)?;
        Some(self.counted(&list))
    }

    /// A list item's marker, counting it.
    fn counted(&mut self, list: &List) -> String {
        let indent = "  ".repeat(list.level);
        let Some(start) = self.numbering.start(list) else {
            return format!("{indent}-");
        };
        // An item starts every deeper level of its list again.
        self.counted
            .retain(|(id, level), _| id != &list.id || *level <= list.level);
        let count = self
            .counted
            .entry((list.id.clone(), list.level))
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(start);
        format!("{indent}{count}.")
    }

    /// A comment on its own line: who made it, the text it is on, and what it says.
    fn comment(&self, id: &str) -> Option<String> {
        let comment = self.comments.get(id)?;
        let by = comment
            .author
            .as_ref()
            .map(|author| format!(" by {author}"))
            .unwrap_or_default();
        let covered = self.covered.get(id).or_else(|| self.covering.get(id));
        let on = covered
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|text| !text.is_empty())
            .map(|text| format!(" on \"{}\"", shortened(&text)))
            .unwrap_or_default();
        Some(format!("[comment{by}{on}: {}]", comment.text.trim()))
    }

    /// A table: a row to a line, each cell's paragraphs on one line of it.
    fn table(&mut self, table: &Element, lines: &mut Vec<String>) {
        let mut rows = Vec::new();
        for row in table.elements().filter(|element| element.name == "w:tr") {
            let mut cells = Vec::new();
            for cell in row.elements().filter(|element| element.name == "w:tc") {
                let properties = cell.child("w:tcPr");
                // A cell merged into the one above holds nothing of its own.
                let merged = properties
                    .and_then(|properties| properties.child("w:vMerge"))
                    .is_some_and(|merge| merge.attribute("w:val") != Some("restart"));
                let mut inside = Vec::new();
                self.blocks(cell, &mut inside);
                cells.push(if merged {
                    String::new()
                } else {
                    inside.join("\n")
                });
                let span = properties
                    .and_then(|properties| properties.child("w:gridSpan"))
                    .and_then(|span| span.attribute("w:val"))
                    .and_then(|span| span.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, MAX_SPAN);
                cells.extend(std::iter::repeat_n(String::new(), span - 1));
            }
            rows.push(cells);
        }
        lines.extend(table::lines(&rows));
    }
}

/// `text`, cut to [`QUOTED`] characters.
fn shortened(text: &str) -> String {
    if text.chars().count() <= QUOTED {
        return text.to_owned();
    }
    let cut: String = text.chars().take(QUOTED - 1).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests;
