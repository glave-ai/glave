//! The model's reasoning in a line, as it streams and once recorded: its headline, the model's
//! own words, and how long it reasoned. Read from what the model already wrote: nothing more is
//! asked of it.

use std::time::Duration;

use glave_event::{Part, Thought};
use glave_ports::Delta;
use jiff::Timestamp;

use crate::turn::Live;

/// The longest headline kept, in characters; one longer is cut, an ellipsis marking the cut.
const LONGEST: usize = 200;

/// Where the sentences of a reasoning end.
const ENDS: [char; 7] = ['.', '!', '?', '…', '。', '！', '？'];

/// The headline of reasoning `text`: its last title, a line of its own in bold (as OpenAI's
/// reasoning summaries begin each part), when it has one, else its last whole sentence. A
/// sentence still being written is not whole: none ends the text without its stop.
pub(crate) fn headline(text: &str) -> Option<String> {
    let mut title = None;
    let mut sentence = None;
    let mut lines = text.split('\n').peekable();
    while let Some(line) = lines.next() {
        let ended = lines.peek().is_some();
        if let Some(bold) = titled(line.trim()) {
            title = Some(bold);
            continue;
        }
        if let Some(last) = last_sentence(line, ended) {
            sentence = Some(last);
        }
    }
    title.or(sentence).map(|said| cut(&said))
}

/// The words of `line` when it is a title: bold, and nothing else.
fn titled(line: &str) -> Option<String> {
    let inner = line.strip_prefix("**")?.strip_suffix("**")?;
    let inner = inner.trim();
    (!inner.is_empty() && !inner.contains("**")).then(|| inner.to_owned())
}

/// The last whole sentence of `line`: one ended by its stop, or the line itself once `ended`.
fn last_sentence(line: &str, ended: bool) -> Option<String> {
    let mut last = None;
    let mut start = 0;
    let mut chars = line.char_indices().peekable();
    while let Some((at, char)) = chars.next() {
        let next = chars.peek().map(|&(_, next)| next);
        if ENDS.contains(&char) && next.is_none_or(char::is_whitespace) {
            let end = at.saturating_add(char.len_utf8());
            last = plain(line.get(start..end).unwrap_or_default()).or(last);
            start = end;
        }
    }
    if ended {
        last = plain(line.get(start..).unwrap_or_default()).or(last);
    }
    last
}

/// `said` as plain words: without a list's or heading's mark before it, bold or code marks, or
/// runs of spaces; `None` when nothing is left.
fn plain(said: &str) -> Option<String> {
    let said = said
        .trim()
        .trim_start_matches(['#', '>', '-', '*', '+', ' '])
        .replace("**", "")
        .replace('`', "");
    let words: Vec<&str> = said.split_whitespace().collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// `said`, at most `LONGEST` characters.
fn cut(said: &str) -> String {
    if said.chars().count() <= LONGEST {
        return said.to_owned();
    }
    let kept: String = said.chars().take(LONGEST.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

/// One model call's reasoning as it streams: the text so far, its headline, when it began and
/// how long it went on, which ends as the model first writes anything else or the call ends.
#[derive(Debug, Default)]
pub(crate) struct Thinking {
    text: String,
    headline: Option<String>,
    began: Option<Timestamp>,
    took: Option<Duration>,
}

impl Thinking {
    /// Takes `delta`, read at `now`, telling `live` it and what the person sees of it: the
    /// reasoning's headline each time it changes, and how long it took as it ends.
    pub(crate) fn see(
        &mut self,
        delta: Delta,
        now: Timestamp,
        live: &mut (dyn FnMut(Live) + Send),
    ) {
        match delta {
            Delta::Reasoning(text) => {
                self.began.get_or_insert(now);
                self.text.push_str(&text);
                // Only a stop or a line's end can make a new headline.
                let closes = text.contains(|c: char| c == '\n' || c == '*' || ENDS.contains(&c));
                live(Live::Reasoning(text));
                if closes {
                    let headline = headline(&self.text);
                    if headline.is_some() && headline != self.headline {
                        self.headline.clone_from(&headline);
                        live(Live::Thinking(headline.unwrap_or_default()));
                    }
                }
            }
            Delta::Text(text) => {
                self.end(now, live);
                live(Live::Text(text));
            }
        }
    }

    /// Ends the reasoning at `now`, when it began and has not ended, telling `live` how long it
    /// took.
    pub(crate) fn end(&mut self, now: Timestamp, live: &mut (dyn FnMut(Live) + Send)) {
        if let (Some(began), None) = (self.began, self.took) {
            let took = now
                .duration_since(began)
                .unsigned_abs()
                .min(Duration::from_secs(u64::from(u32::MAX)));
            self.took = Some(took);
            live(Live::Thought(took));
        }
    }

    /// What the reasoning of `parts`, the call's recorded message, came to: its headline and
    /// how long it took, when it streamed; `None` when it has no readable reasoning.
    pub(crate) fn thought(&self, parts: &[Part]) -> Option<Thought> {
        thought_of(parts).map(|thought| Thought {
            ms: self
                .took
                .map(|took| u64::try_from(took.as_millis()).unwrap_or(u64::MAX)),
            ..thought
        })
    }
}

/// What the reasoning of a message's `parts` came to, read from its text alone: its headline,
/// how long it took unknown; `None` when it has no readable reasoning. For a message recorded
/// before glave kept its thought, read again as it is served.
pub fn thought_of(parts: &[Part]) -> Option<Thought> {
    let text: Vec<&str> = parts
        .iter()
        .filter_map(|part| match part {
            Part::Reasoning {
                text: Some(text), ..
            } if !text.trim().is_empty() => Some(text.as_str()),
            _ => None,
        })
        .collect();
    (!text.is_empty()).then(|| Thought {
        headline: headline(&text.join("\n")),
        ms: None,
    })
}

#[cfg(test)]
mod tests;
