//! What the person sees in the terminal: the answer as it streams, approval prompts, and the
//! receipt that closes each turn. Every status starts with a word; color only reinforces it.

use std::fmt::Write as _;
use std::future::Future;
use std::io::{IsTerminal as _, Write as _};

use glave_event::{Currency, Money, Outcome};
use glave_harness::{ApprovalRequest, Approver, Decision, Live, TurnSummary};

use crate::input::Input;

const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn color_enabled() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

fn paint(color: bool, code: &str, text: &str) -> String {
    if color {
        format!("{code}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

fn flush() {
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
    /// The protections in force, such as `approval: edits · sandbox: project`.
    protections: String,
}

impl Printer {
    pub fn new(model: &str, protections: String) -> Self {
        Self {
            color: color_enabled(),
            showing: Showing::Nothing,
            speaker: speaker(model).to_owned(),
            protections,
        }
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

    pub fn live(&mut self, live: Live) {
        match live {
            Live::Text(text) => {
                if self.showing != Showing::Text {
                    print!("\n{} ", paint(self.color, GREEN, &self.speaker));
                    self.showing = Showing::Text;
                }
                print!("{text}");
            }
            Live::Reasoning(text) => {
                if self.showing != Showing::Reasoning {
                    println!();
                    self.showing = Showing::Reasoning;
                }
                print!("{}", paint(self.color, DIM, &text));
            }
            Live::ToolStarted { summary } => {
                println!();
                print!("  {} ", paint(self.color, DIM, &format!("{summary} …")));
                self.showing = Showing::Nothing;
            }
            Live::ToolDeclined {
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
            Live::Compacted {
                before_tokens,
                after_tokens,
            } => {
                println!();
                println!(
                    "  {}",
                    paint(
                        self.color,
                        DIM,
                        &format!(
                            "compacted the conversation to fit: ~{} → ~{} tokens",
                            thousands(before_tokens),
                            thousands(after_tokens)
                        )
                    )
                );
                self.showing = Showing::Nothing;
            }
            Live::CompactionFailed(error) => {
                println!();
                println!(
                    "  {} {error}",
                    paint(self.color, YELLOW, "could not compact the conversation:")
                );
                self.showing = Showing::Nothing;
            }
            Live::ToolFinished { is_error, .. } => {
                if is_error {
                    println!("{}", paint(self.color, RED, "error"));
                } else {
                    println!("{}", paint(self.color, CYAN, "done"));
                }
            }
        }
        flush();
    }

    /// The receipt that closes a turn: what happened and what it cost.
    pub fn receipt(&mut self, summary: &TurnSummary, local: bool) {
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

    pub fn error(&mut self, message: &str) {
        println!("\n\n{} {message}", paint(self.color, RED, "error"));
        self.showing = Showing::Nothing;
        flush();
    }
}

/// Asks the person in the terminal before anything changes.
pub struct AskInTerminal {
    color: bool,
    input: Input,
}

impl AskInTerminal {
    pub fn new(input: Input) -> Self {
        Self {
            color: color_enabled(),
            input,
        }
    }
}

impl Approver for AskInTerminal {
    fn decide(&self, request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        let prompt = format!(
            "\n\n{} {}\n  allow? [y] once  [a] always in this thread  [N] no › ",
            paint(self.color, YELLOW, "needs you"),
            request.summary
        );
        async move {
            print!("{prompt}");
            flush();
            match self
                .input
                .line()
                .await
                .map(|answer| answer.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("y" | "yes") => Decision::AllowOnce,
                Some("a" | "always") => Decision::AllowForThread,
                _ => Decision::Deny,
            }
        }
    }
}

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
