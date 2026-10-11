//! What the person sees in the terminal: the answer as it streams, approval prompts, and the
//! receipt that closes each turn. Every status starts with a word; color only reinforces it.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{IsTerminal as _, Write as _};
use std::time::Duration;

use glave_event::{Currency, GoalReason, Money, Outcome, ThreadId};
use glave_harness::Question;
use glave_session::protocol::{TurnResult, Update};

use crate::waiting::Waiting;

const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
pub const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn color_enabled() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

pub fn paint(color: bool, code: &str, text: &str) -> String {
    if color {
        format!("{code}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

pub fn flush() {
    let _ = std::io::stdout().flush();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Showing {
    Nothing,
    Text,
    Reasoning,
}

/// Prints live progress and receipts.
pub struct Printer {
    color: bool,
    showing: Showing,
    /// Who the answers come from: the model's own name, never one it is not.
    speaker: String,
    /// The protections in force, such as `approval: auto · sandbox: project`.
    protections: String,
    waiting: Waiting,
    /// The subagents running now, by their thread, with what each is for.
    subagents: HashMap<ThreadId, String>,
}

impl Printer {
    pub fn new(model: &str, protections: String) -> Self {
        let color = color_enabled();
        let speaker = speaker(model).to_owned();
        Self {
            color,
            showing: Showing::Nothing,
            waiting: Waiting::new(color, &speaker),
            subagents: HashMap::new(),
            speaker,
            protections,
        }
    }

    /// The line that counts the seconds while the model has not answered, for a clock to
    /// update.
    pub fn waiting(&self) -> Waiting {
        self.waiting.clone()
    }

    pub fn banner(&self, model: &str, provider: &str, folder: &str, note: &str) {
        println!(
            "{} {}",
            paint(self.color, GREEN, "glave"),
            paint(
                self.color,
                DIM,
                &format!(
                    "· {model} via {provider} · {folder} · {} · {note}",
                    self.protections
                )
            )
        );
        println!(
            "{}",
            paint(self.color, DIM, "Type a request. /exit to leave.")
        );
    }

    pub fn ask(&self) {
        print!("\n{} ", paint(self.color, GREEN, "›"));
        flush();
    }

    pub fn update(&mut self, update: Update) {
        self.waiting.stop();
        match update {
            Update::Waiting => self.waiting.start(),
            Update::Retrying {
                retry,
                of,
                wait_ms,
                error,
            } => {
                let error: String = error.chars().take(ERROR_CHARS).collect();
                let seconds = Duration::from_millis(wait_ms).as_secs_f64();
                let detail = format!("· {error}; again in {seconds:.0} s ({retry} of {of})");
                self.note(YELLOW, "retrying", &detail);
            }
            Update::Text { text } => {
                if self.showing != Showing::Text {
                    print!("\n{} ", paint(self.color, GREEN, &self.speaker));
                    self.showing = Showing::Text;
                }
                print!("{text}");
            }
            Update::Reasoning { text } => {
                if self.showing != Showing::Reasoning {
                    println!();
                    self.showing = Showing::Reasoning;
                }
                print!("{}", paint(self.color, DIM, &text));
            }
            Update::ToolStarted { summary } => {
                println!();
                print!("  {} ", paint(self.color, DIM, &format!("{summary} …")));
                self.showing = Showing::Nothing;
            }
            Update::ToolDeclined {
                summary,
                no_one_there,
            } => {
                let why = if no_one_there {
                    "not run: no one here to approve it"
                } else {
                    "declined"
                };
                println!();
                println!(
                    "  {} {}",
                    paint(self.color, DIM, &format!("{summary} …")),
                    paint(self.color, YELLOW, why)
                );
                self.showing = Showing::Nothing;
            }
            Update::Compacted {
                before_tokens,
                after_tokens,
            } => {
                let detail = format!(
                    "the conversation to fit: ~{} → ~{} tokens",
                    thousands(before_tokens),
                    thousands(after_tokens)
                );
                self.note(DIM, "compacted", &detail);
            }
            Update::CompactionFailed { error } => {
                self.note(YELLOW, "could not compact the conversation:", &error);
            }
            // Shown by the client, which answers it.
            Update::Asked { .. } => {}
            Update::Memory {
                change,
                memory,
                text,
            } => self.memory(&change, memory, text),
            Update::SubagentStarted { .. } | Update::SubagentEnded { .. } => self.subagent(update),
            Update::ToolFinished { is_error, .. } => {
                if is_error {
                    println!("{}", paint(self.color, RED, "error"));
                } else {
                    println!("{}", paint(self.color, CYAN, "done"));
                }
            }
        }
        flush();
    }

    /// A memory saved, edited or forgotten: shown at once, with how to change it.
    fn memory(&mut self, change: &str, memory: glave_event::MemoryId, text: Option<String>) {
        let detail = match text {
            Some(text) => format!("\"{text}\" · glave memory edit|forget {memory}"),
            None => memory.to_string(),
        };
        self.note(GREEN, &format!("{change}:"), &detail);
    }

    /// A line of its own about the turn: a word in `color`, then dim detail.
    fn note(&mut self, color: &str, word: &str, detail: &str) {
        println!();
        println!(
            "  {} {}",
            paint(self.color, color, word),
            paint(self.color, DIM, detail)
        );
        self.showing = Showing::Nothing;
    }

    /// The receipt that closes a turn: what happened and what it cost.
    pub fn receipt(&mut self, summary: &TurnResult, local: bool) {
        self.waiting.stop();
        let (word, color) = match summary.outcome {
            Outcome::Completed => ("done", CYAN),
            Outcome::LimitReached => ("stopped at a limit", YELLOW),
            Outcome::Interrupted => ("interrupted", YELLOW),
            Outcome::Cancelled => ("cancelled", YELLOW),
            Outcome::Failed => ("failed", RED),
        };
        let tokens = &summary.tokens;
        let mut line = format!("{} · ", self.protections);
        let _ = write!(
            line,
            "{} model call{} · {} tool call{} · {} tokens",
            summary.model_calls,
            plural(u64::from(summary.model_calls)),
            summary.tool_calls,
            plural(u64::from(summary.tool_calls)),
            thousands(tokens.input.saturating_add(tokens.output)),
        );
        if tokens.cache_read > 0 {
            let _ = write!(line, " ({} cached)", thousands(tokens.cache_read));
        }
        let cost = match summary.cost {
            Some(money) if money.micros > 0 || !local => format_money(money),
            Some(_) | None if local => "$0.00, on this computer".to_owned(),
            _ => "cost not reported".to_owned(),
        };
        let _ = write!(line, " · {cost}");
        if summary.estimated {
            line.push_str(" (estimated)");
        }
        println!(
            "\n\n{} {}",
            paint(self.color, color, word),
            paint(self.color, DIM, &format!("· {line}"))
        );
        self.showing = Showing::Nothing;
        flush();
    }

    /// Asks the person about an action the model wants to take; the answer is typed next.
    pub fn approval(&mut self, summary: &str) {
        self.waiting.stop();
        print!(
            "\n\n{} {summary}\n  allow? [y] once  [a] always in this thread  [N] no › ",
            paint(self.color, YELLOW, "needs you")
        );
        self.showing = Showing::Nothing;
        flush();
    }

    /// A subagent starting or ending.
    fn subagent(&mut self, update: Update) {
        match update {
            Update::SubagentStarted {
                subagent,
                description,
            } => {
                println!();
                println!("  {} {description}", paint(self.color, CYAN, "agent"));
                self.subagents.insert(subagent, description);
            }
            Update::SubagentEnded { subagent, ok } => {
                let description = self.subagents.remove(&subagent).unwrap_or_default();
                let (color, word) = if ok {
                    (CYAN, "agent done")
                } else {
                    (RED, "agent failed")
                };
                println!("  {} {description}", paint(self.color, color, word));
            }
            _ => {}
        }
        self.showing = Showing::Nothing;
    }

    /// What a subagent did, one finished step per line under its description. Its words go
    /// to the parent, not here.
    pub fn child(&mut self, subagent: ThreadId, update: Update) {
        let Some(label) = self.subagents.get(&subagent) else {
            return;
        };
        let step = match update {
            Update::ToolFinished { summary, is_error } => {
                format!("{summary} · {}", if is_error { "error" } else { "done" })
            }
            Update::ToolDeclined {
                summary,
                no_one_there,
            } => {
                let why = if no_one_there {
                    "not run: no one here to approve it"
                } else {
                    "declined"
                };
                format!("{summary} · {why}")
            }
            Update::Retrying { retry, of, .. } => format!("retrying ({retry} of {of})"),
            Update::Compacted { .. } => "compacted its conversation".to_owned(),
            _ => return,
        };
        self.waiting.stop();
        if self.showing != Showing::Nothing {
            println!();
        }
        println!(
            "    {}",
            paint(self.color, DIM, &format!("↳ {label} · {step}"))
        );
        self.showing = Showing::Nothing;
        flush();
    }

    /// `summary` with the subagent it comes from, when a subagent asks.
    pub fn labeled(&self, subagent: ThreadId, summary: &str) -> String {
        match self.subagents.get(&subagent) {
            Some(label) => format!("[{label}] {summary}"),
            None => summary.to_owned(),
        }
    }

    /// One of the model's questions; the answer is typed next.
    pub fn ask_question(&mut self, question: &Question) {
        self.waiting.stop();
        print!("{}", crate::asking::shown(self.color, question));
        self.showing = Showing::Nothing;
        flush();
    }

    /// What the person can change to fix the error just shown.
    pub fn fix(&mut self, text: &str) {
        println!("  {} {text}", paint(self.color, YELLOW, "fix:"));
        flush();
    }

    /// How a goal ended, the tokens its turns used, and, when paused, what the model needs.
    pub fn goal_ended(&mut self, reason: GoalReason, tokens: u64, note: Option<&str>) {
        self.waiting.stop();
        let (word, color, why) = match reason {
            GoalReason::Met => ("goal met", CYAN, "the model checked it is done"),
            GoalReason::Paused => ("goal paused", YELLOW, note.unwrap_or("the model needs you")),
            GoalReason::Stopped => ("goal stopped", YELLOW, "you stopped it"),
            GoalReason::Budget => ("goal stopped", YELLOW, "its token budget ran out"),
            GoalReason::Stalled => (
                "goal stopped",
                YELLOW,
                "three turns in a row without a tool call",
            ),
        };
        let detail = format!("{why} · {} tokens", thousands(tokens));
        self.note(color, word, &detail);
        flush();
    }

    /// Something beside the turn that went wrong; the turn goes on without it.
    pub fn warning(&mut self, message: &str) {
        println!("{} {message}", paint(self.color, YELLOW, "warning"));
        flush();
    }

    /// What glave is doing before the turn starts.
    pub fn status(&mut self, message: &str) {
        println!("{}", paint(self.color, DIM, message));
        flush();
    }

    pub fn error(&mut self, message: &str) {
        self.waiting.stop();
        println!("\n\n{} {message}", paint(self.color, RED, "error"));
        self.showing = Showing::Nothing;
        flush();
    }
}

/// The most of an error shown on a retry line.
const ERROR_CHARS: usize = 200;

/// A model's short name for labeling its answers: `anthropic/claude-haiku-5.5` is
/// `claude-haiku-5.5`.
fn speaker(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}

fn plural(count: u64) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// `1234567` as `1,234,567`.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len().saturating_add(digits.len() / 3));
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len().saturating_sub(index)) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Money from exact micros: four decimals under a cent, two above.
fn format_money(money: Money) -> String {
    let sign = if money.micros < 0 { "-" } else { "" };
    let micros = money.micros.unsigned_abs();
    let whole = micros / 1_000_000;
    let fraction = micros % 1_000_000;
    let amount = if micros < 10_000 {
        format!("{whole}.{:04}", fraction / 100)
    } else {
        format!("{whole}.{:02}", fraction / 10_000)
    };
    if money.currency == Currency::USD {
        format!("{sign}${amount}")
    } else {
        format!("{sign}{amount} {}", money.currency)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_labeled_with_the_model_name() {
        assert_eq!(speaker("anthropic/claude-haiku-5.5"), "claude-haiku-5.5");
        assert_eq!(speaker("gemma4:12b"), "gemma4:12b");
    }

    #[test]
    fn numbers_and_money_read_well() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(1_204), "1,204");
        assert_eq!(thousands(1_234_567), "1,234,567");
        let usd = |micros| Money {
            micros,
            currency: Currency::USD,
        };
        assert_eq!(format_money(usd(4_200)), "$0.0042");
        assert_eq!(format_money(usd(1_230_000)), "$1.23");
        assert_eq!(format_money(usd(0)), "$0.0000");
        let eur = Money {
            micros: 2_500_000,
            currency: Currency::new("EUR").unwrap(),
        };
        assert_eq!(format_money(eur), "2.50 EUR");
    }
}
