//! A reply that repeats itself, seen as it streams: the way Gemini CLI's loop detection sees
//! one (`packages/core/src/services/loopDetectionService.ts`), the same 50 characters ten times
//! close together, with little else between them. A model fallen into such a loop writes it
//! until its output limit, so its call is stopped instead.
//!
//! Code blocks (fenced or indented) and table rows are not read, as Gemini CLI leaves them out:
//! a test file's lines and a table's cells repeat by nature. A list's items that begin alike are
//! not a loop either, since what lies between their repeats differs (each item's own words or
//! number); nor is one line said again and again, as a log pasted outside a block, nor a run
//! of one character: the loops models fall into cycle through a few lines, or within one.

use std::collections::{BTreeSet, HashMap, VecDeque};

/// The characters of one chunk compared, as Gemini CLI's `CONTENT_CHUNK_SIZE`.
const CHUNK: usize = 50;
/// How many times a chunk comes before the reply counts as repeating, as its
/// `CONTENT_LOOP_THRESHOLD`.
const TIMES: usize = 10;
/// The most text kept to compare against, in characters, as its `MAX_HISTORY_LENGTH`.
const KEPT: usize = 5_000;
/// The most, on average, between a chunk's repeats, in characters: five chunks, as there.
const APART: usize = CHUNK * 5;
/// The most different texts between a chunk's last repeats that still make a loop: half of
/// them, as there; more are items of a list that begin alike.
const PERIODS: usize = TIMES / 2;
/// The most of a line held before it is read, in bytes: a line is told apart (a table's row,
/// code) whole, and one longer is read as it comes.
const HELD: usize = 400;

/// Where the reply's current line stands.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    /// Held until it ends, or grows past [`HELD`], to be told what it is.
    Held(String),
    /// Prose longer than what is held, read as it comes.
    Read,
    /// Not read: code, or a table's row, longer than what is held.
    Skipped,
}

/// Watches one reply's visible text for a loop.
#[derive(Debug)]
pub(crate) struct Looping {
    line: Line,
    in_code: bool,
    /// The text read lately, at most [`KEPT`] characters, and where its first one is in all
    /// read.
    kept: VecDeque<char>,
    first: usize,
    /// For each chunk read lately, where it ended, each at least a chunk after the one before.
    seen: HashMap<String, VecDeque<usize>>,
    /// Each chunk recorded, in order, so one that falls out of what is kept is forgotten.
    order: VecDeque<(usize, String)>,
    found: bool,
}

impl Default for Looping {
    fn default() -> Self {
        Self {
            line: Line::Held(String::new()),
            in_code: false,
            kept: VecDeque::new(),
            first: 0,
            seen: HashMap::new(),
            order: VecDeque::new(),
            found: false,
        }
    }
}

impl Looping {
    /// Reads `text`, the next of the reply: true once the reply repeats itself.
    pub(crate) fn see(&mut self, text: &str) -> bool {
        for char in text.chars() {
            if self.found {
                break;
            }
            self.next(char);
        }
        self.found
    }

    fn next(&mut self, char: char) {
        let ends = char == '\n';
        match &mut self.line {
            Line::Held(held) if ends => {
                let held = std::mem::take(held);
                self.ended(&held);
            }
            Line::Held(held) => {
                held.push(char);
                if held.len() > HELD {
                    let held = std::mem::take(held);
                    self.line = if self.reads(&held) {
                        held.trim_start().chars().for_each(|char| self.read(char));
                        Line::Read
                    } else {
                        Line::Skipped
                    };
                }
            }
            Line::Read if ends => {
                self.read('\n');
                self.line = Line::Held(String::new());
            }
            Line::Read => self.read(char),
            Line::Skipped if ends => self.line = Line::Held(String::new()),
            Line::Skipped => {}
        }
    }

    /// Takes in a whole `line` held: a fence opens or closes a code block, and prose is read.
    fn ended(&mut self, line: &str) {
        if fence(line) {
            self.in_code = !self.in_code;
        } else if self.reads(line) {
            line.trim_start().chars().for_each(|char| self.read(char));
            self.read('\n');
        }
    }

    /// Whether `line` is prose: not in a code block or one indented, nor a table's row,
    /// whatever its first character.
    fn reads(&self, line: &str) -> bool {
        let indented = line.starts_with('\t') || line.starts_with("    ");
        !self.in_code && !fence(line) && !indented && !line.contains('|')
    }

    /// Reads one character of prose, and whether the chunk it ends repeats.
    fn read(&mut self, char: char) {
        self.kept.push_back(char);
        if self.kept.len() > KEPT {
            self.kept.pop_front();
            self.first = self.first.saturating_add(1);
            self.forget();
        }
        let len = self.kept.len();
        if len < CHUNK {
            return;
        }
        let end = self.first.saturating_add(len);
        let chunk: String = self.kept.range(len.saturating_sub(CHUNK)..).collect();
        // A run of one character, a divider or a bar, is never a loop.
        let mut chars = chunk.chars();
        let first = chars.next();
        if chars.all(|char| Some(char) == first) {
            return;
        }
        let ends = self.seen.entry(chunk.clone()).or_default();
        if ends
            .back()
            .is_some_and(|last| end < last.saturating_add(CHUNK))
        {
            return;
        }
        ends.push_back(end);
        self.order.push_back((end, chunk.clone()));
        self.found = self.repeats(&chunk);
    }

    /// Forgets the chunks that began before what is kept.
    fn forget(&mut self) {
        while let Some((end, _)) = self.order.front() {
            if end.saturating_sub(CHUNK) >= self.first {
                break;
            }
            if let Some((end, chunk)) = self.order.pop_front()
                && let Some(ends) = self.seen.get_mut(&chunk)
            {
                ends.retain(|at| *at != end);
                if ends.is_empty() {
                    self.seen.remove(&chunk);
                }
            }
        }
    }

    /// Whether `chunk`'s last [`TIMES`] repeats are close together with little else between.
    fn repeats(&self, chunk: &str) -> bool {
        let Some(ends) = self.seen.get(chunk) else {
            return false;
        };
        if ends.len() < TIMES {
            return false;
        }
        let last: Vec<usize> = ends
            .iter()
            .skip(ends.len().saturating_sub(TIMES))
            .copied()
            .collect();
        let (Some(from), Some(to)) = (last.first(), last.last()) else {
            return false;
        };
        if to.saturating_sub(*from) / (TIMES - 1) > APART {
            return false;
        }
        let mut periods: Vec<String> = last
            .windows(2)
            .filter_map(|pair| match pair {
                [from, to] => Some(self.between(*from, *to)),
                _ => None,
            })
            .collect();
        periods.sort_unstable();
        periods.dedup();
        periods.len() <= PERIODS && cycles(&self.between(*from, *to))
    }

    /// The text kept from where `from` ends to where `to` does.
    fn between(&self, from: usize, to: usize) -> String {
        let len = self.kept.len();
        let end = to.saturating_sub(self.first).min(len);
        let start = from.saturating_sub(self.first).min(end);
        self.kept.range(start..end).collect()
    }
}

/// Whether `span`, the text a loop repeats in, cycles within a line, or through at least two
/// different lines: one line said again and again is a log, or a list, not a model's loop.
fn cycles(span: &str) -> bool {
    if !span.contains('\n') {
        return true;
    }
    let lines: Vec<&str> = span.split('\n').collect();
    // Only the lines whole in it: the first and last may be cut.
    let whole = lines
        .get(1..lines.len().saturating_sub(1))
        .unwrap_or_default();
    let different: BTreeSet<&str> = (whole.iter())
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect();
    different.len() >= 2
}

/// Whether `line` opens or closes a code block.
fn fence(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("```") || line.starts_with("~~~")
}

#[cfg(test)]
mod tests;
