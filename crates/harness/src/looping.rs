//! A reply that repeats itself, seen as it streams: the way Gemini CLI's loop detection sees
//! one (`packages/core/src/services/loopDetectionService.ts`), the same 50 characters ten times
//! close together, with little else between them. A model fallen into such a loop writes it
//! until its output limit, so its call is stopped instead.
//!
//! Code blocks and table rows are not read, as Gemini CLI leaves them out: a test file's lines
//! and a table's cells repeat by nature. A list's items that begin alike are not a loop either,
//! since what lies between their repeats differs (each item's own words or number).

use std::collections::{HashMap, VecDeque};

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

/// Where the reply's current line stands.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    /// Its start, holding what of it is not space so far, until it can be told what it is.
    Start(String),
    /// Prose, read as it comes.
    Read,
    /// Not read: a table row, or a line of a code block.
    Skipped,
    /// A fence that opens or closes a code block.
    Fence,
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
            line: Line::Start(String::new()),
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
        if char == '\n' {
            if self.line == Line::Read {
                self.read('\n');
            }
            if self.line == Line::Fence {
                self.in_code = !self.in_code;
            }
            self.line = Line::Start(String::new());
            return;
        }
        match &mut self.line {
            Line::Start(lead) => {
                if lead.is_empty() && char.is_whitespace() {
                    return;
                }
                lead.push(char);
                let lead = std::mem::take(lead);
                self.line = self.told(&lead);
                if self.line == Line::Read {
                    lead.chars().for_each(|char| self.read(char));
                } else if matches!(self.line, Line::Start(_)) {
                    self.line = Line::Start(lead);
                }
            }
            Line::Read => self.read(char),
            Line::Skipped | Line::Fence => {}
        }
    }

    /// What a line beginning with `lead` is, or `Start` while that can't be told yet.
    fn told(&self, lead: &str) -> Line {
        let fence = lead.starts_with("```") || lead.starts_with("~~~");
        let maybe_fence = "```".starts_with(lead) || "~~~".starts_with(lead);
        if fence {
            Line::Fence
        } else if maybe_fence {
            Line::Start(lead.to_owned())
        } else if self.in_code || lead.starts_with('|') {
            Line::Skipped
        } else {
            Line::Read
        }
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
        periods.len() <= PERIODS
    }

    /// The text kept from where `from` ends to where `to` does.
    fn between(&self, from: usize, to: usize) -> String {
        let len = self.kept.len();
        let end = to.saturating_sub(self.first).min(len);
        let start = from.saturating_sub(self.first).min(end);
        self.kept.range(start..end).collect()
    }
}

#[cfg(test)]
mod tests;
