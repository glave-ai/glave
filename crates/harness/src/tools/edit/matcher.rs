//! Finding `old_string` in a file and replacing it. Matching is tried in stages, from exact
//! to forgiving of the slips smaller models make (copied line numbers, whitespace,
//! indentation, typographic quotes), and stops at the first stage that matches. A match must
//! be unique unless every occurrence is to be replaced. There is no similarity matching.

/// How a match was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Exact,
    LineNumbers,
    TrailingSpace,
    Indentation,
    Typography,
}

impl Stage {
    pub(crate) fn note(self) -> &'static str {
        match self {
            Self::Exact => "",
            Self::LineNumbers => " (matched after removing copied line numbers)",
            Self::TrailingSpace => " (matched ignoring trailing whitespace)",
            Self::Indentation => " (matched ignoring indentation)",
            Self::Typography => " (matched ignoring quote and dash styles)",
        }
    }
}

/// The result of a successful edit.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Edited {
    pub text: String,
    pub replacements: usize,
    /// Lines of the first replacement in the new text, 1-based.
    pub first_line: usize,
    pub last_line: usize,
    pub stage: Stage,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EditError {
    Empty,
    Identical,
    NotFound { hint: Option<(usize, String)> },
    Ambiguous { lines: Vec<usize> },
}

type Span = (usize, usize);

/// Replaces `old` with `new` in `text`. All three use `\n` line endings.
pub(crate) fn apply(
    text: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<Edited, EditError> {
    if old.is_empty() {
        return Err(EditError::Empty);
    }
    if old == new {
        return Err(EditError::Identical);
    }
    let exact = exact_spans(text, old);
    if !exact.is_empty() {
        return finish(text, &exact, replace_all, Stage::Exact, |_| new.to_owned());
    }
    if let Some(stripped) = strip_numbers(old) {
        let spans = exact_spans(text, &stripped);
        if !spans.is_empty() {
            let new = strip_numbers(new).unwrap_or_else(|| new.to_owned());
            return finish(text, &spans, replace_all, Stage::LineNumbers, |_| {
                new.clone()
            });
        }
    }
    let old_lines = lines_of(old);
    let new_body = new
        .strip_suffix('\n')
        .filter(|_| old.ends_with('\n'))
        .unwrap_or(new);
    for (stage, normalize) in [
        (Stage::TrailingSpace, trailing as fn(&str) -> String),
        (Stage::Indentation, both_ends),
        (Stage::Typography, typography),
    ] {
        let spans = line_spans(text, &old_lines, normalize);
        if spans.is_empty() {
            continue;
        }
        let old_indent = first_indent(old);
        return finish(text, &spans, replace_all, stage, |matched| {
            if stage == Stage::TrailingSpace {
                new_body.to_owned()
            } else {
                reindent(new_body, old_indent, first_indent(matched))
            }
        });
    }
    Err(EditError::NotFound {
        hint: hint(text, old),
    })
}

fn finish(
    text: &str,
    spans: &[Span],
    replace_all: bool,
    stage: Stage,
    replacement: impl Fn(&str) -> String,
) -> Result<Edited, EditError> {
    if spans.len() > 1 && !replace_all {
        return Err(EditError::Ambiguous {
            lines: spans
                .iter()
                .map(|(start, _)| line_of(text, *start))
                .collect(),
        });
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut first = None;
    for (start, end) in spans {
        out.push_str(text.get(cursor..*start).unwrap_or(""));
        let replaced = replacement(text.get(*start..*end).unwrap_or(""));
        if first.is_none() {
            first = Some((out.len(), out.len().saturating_add(replaced.len())));
        }
        out.push_str(&replaced);
        cursor = *end;
    }
    out.push_str(text.get(cursor..).unwrap_or(""));
    let (first_start, first_end) = first.unwrap_or((0, 0));
    Ok(Edited {
        first_line: line_of(&out, first_start),
        last_line: line_of(&out, first_end.saturating_sub(1).max(first_start)),
        text: out,
        replacements: spans.len(),
        stage,
    })
}

fn exact_spans(text: &str, old: &str) -> Vec<Span> {
    text.match_indices(old)
        .map(|(start, found)| (start, start.saturating_add(found.len())))
        .collect()
}

/// The lines of `old`, without a final empty line from a trailing newline.
fn lines_of(old: &str) -> Vec<&str> {
    old.strip_suffix('\n').unwrap_or(old).split('\n').collect()
}

/// Spans of whole lines whose normalized text equals the normalized lines of `old`.
fn line_spans(text: &str, old_lines: &[&str], normalize: fn(&str) -> String) -> Vec<Span> {
    let wanted: Vec<String> = old_lines.iter().map(|line| normalize(line)).collect();
    if wanted.iter().all(String::is_empty) {
        return Vec::new();
    }
    let mut starts = Vec::new();
    let mut offset: usize = 0;
    for line in text.split('\n') {
        starts.push((offset, line));
        offset = offset.saturating_add(line.len()).saturating_add(1);
    }
    let mut spans = Vec::new();
    let mut index: usize = 0;
    while let Some(window) = starts.get(index..index.saturating_add(wanted.len())) {
        let matches = window.len() == wanted.len()
            && window
                .iter()
                .zip(&wanted)
                .all(|((_, line), want)| normalize(line) == *want);
        if matches
            && let (Some((start, _)), Some((last_start, last))) = (window.first(), window.last())
        {
            spans.push((*start, last_start.saturating_add(last.len())));
            index = index.saturating_add(wanted.len());
        } else {
            index = index.saturating_add(1);
        }
    }
    spans
}

fn trailing(line: &str) -> String {
    line.trim_end().to_owned()
}

fn both_ends(line: &str) -> String {
    line.trim().to_owned()
}

fn typography(line: &str) -> String {
    line.trim()
        .chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            '\u{00A0}' | '\u{2007}' | '\u{202F}' => ' ',
            other => other,
        })
        .collect()
}

/// Removes `123\t` read-output prefixes, if every non-empty line has one.
fn strip_numbers(text: &str) -> Option<String> {
    let mut stripped = Vec::new();
    for line in text.split('\n') {
        if line.is_empty() {
            stripped.push(line);
            continue;
        }
        let trimmed = line.trim_start();
        let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        stripped.push(trimmed.get(digits..)?.strip_prefix('\t')?);
    }
    Some(stripped.join("\n"))
}

/// The leading whitespace of the first non-blank line.
fn first_indent(text: &str) -> &str {
    let line = text
        .split('\n')
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let indent = line.len().saturating_sub(line.trim_start().len());
    line.get(..indent).unwrap_or("")
}

/// Shifts `new` from the indentation of `old` to the indentation found in the file.
fn reindent(new: &str, old_indent: &str, file_indent: &str) -> String {
    if old_indent == file_indent {
        return new.to_owned();
    }
    new.split('\n')
        .map(|line| match line.strip_prefix(old_indent) {
            Some(rest) if !line.trim().is_empty() => format!("{file_indent}{rest}"),
            _ => line.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 1-based line number of a byte offset.
fn line_of(text: &str, offset: usize) -> usize {
    text.get(..offset)
        .map_or(1, |before| before.matches('\n').count().saturating_add(1))
}

/// The first file line resembling the first line of `old`, to point the model at it.
fn hint(text: &str, old: &str) -> Option<(usize, String)> {
    let probe = old
        .split('\n')
        .map(str::trim)
        .find(|line| line.len() >= 4)?;
    text.split('\n')
        .enumerate()
        .find(|(_, line)| {
            let line = line.trim();
            line.len() >= 4 && (line.contains(probe) || probe.contains(line))
        })
        .map(|(index, line)| (index.saturating_add(1), line.to_owned()))
}

#[cfg(test)]
mod tests;
