//! What the person sees in the terminal: the answer as it streams, approval prompts, and the
//! receipt that closes each turn. Every status starts with a word; color only reinforces it.

use std::fmt::Write as _;
use std::io::{IsTerminal as _, Write as _};
use std::time::Duration;

use glave_event::{Currency, GoalReason, Money, Outcome, OutsideSandbox};
use glave_harness::Question;
use glave_session::protocol::{PlanInfo, TurnResult, Update};

const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
pub(crate) const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn color_enabled() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

pub(crate) fn paint(color: bool, code: &str, text: &str) -> String {
    if color {
        format!("{code}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

pub(crate) fn flush() {
    let _ = std::io::stdout().flush();
}

/// Prints each item of a turn as it comes, approval prompts, and the receipt that closes it.
pub(crate) struct Printer {
    color: bool,
    /// Whether the model's words are being printed, so the next of them follow on.
    speaking: bool,
    /// Who the answers come from: the model's own name, never one it is not.
    speaker: String,
    /// The protections in force, such as `permissions: auto · sandbox: project`.
    protections: String,
}

impl Printer {
    pub(crate) fn new(model: &str, protections: String) -> Self {
        Self {
            color: color_enabled(),
            speaking: false,
            speaker: speaker(model).to_owned(),
            protections,
        }
    }

    /// One item of the turn, on a line of its own; the model's words follow on as they come.
    pub(crate) fn update(&mut self, update: Update) {
        match update {
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
                if !self.speaking {
                    print!("\n{} ", paint(self.color, GREEN, &self.speaker));
                    self.speaking = true;
                }
                print!("{text}");
            }
            Update::ToolStarted { summary } => {
                println!();
                print!("  {} ", paint(self.color, DIM, &format!("{summary} …")));
                self.speaking = false;
            }
            Update::ToolDeclined {
                summary,
                caution,
                no_one_there,
            } => {
                let caution = caution.as_deref();
                print!("{}", declined(self.color, &summary, caution, no_one_there));
                self.speaking = false;
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
            Update::Memory {
                change,
                memory,
                text,
            } => self.memory(&change, memory, text),
            Update::SubagentStarted { description, .. } => {
                println!();
                println!("  {} {description}", paint(self.color, CYAN, "agent"));
                self.speaking = false;
            }
            Update::SubagentEnded { ok, .. } => {
                let (color, word) = if ok {
                    (CYAN, "agent done")
                } else {
                    (RED, "agent failed")
                };
                println!("  {}", paint(self.color, color, word));
                self.speaking = false;
            }
            Update::ToolFinished { is_error, .. } => {
                if is_error {
                    println!("{}", paint(self.color, RED, "error"));
                } else {
                    println!("{}", paint(self.color, CYAN, "done"));
                }
            }
            // The client answers questions and approvals; reasoning, the wait for the model
            // and a message given back show nothing.
            Update::Asked { .. }
            | Update::ApprovalPending { .. }
            | Update::Unread { .. }
            | Update::Reasoning { .. }
            | Update::Waiting => {}
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
        self.speaking = false;
    }

    /// The receipt that closes a turn: what happened and what it cost.
    pub(crate) fn receipt(&mut self, summary: &TurnResult) {
        let (word, color) = match summary.outcome {
            Outcome::Completed => ("done", CYAN),
            Outcome::LimitReached => ("stopped at a limit", YELLOW),
            Outcome::Interrupted => ("interrupted", YELLOW),
            Outcome::Cancelled => ("cancelled", YELLOW),
            Outcome::Failed => ("failed", RED),
        };
        let line = receipt_line(&self.protections, summary);
        println!(
            "\n\n{} {}",
            paint(self.color, color, word),
            paint(self.color, DIM, &format!("· {line}"))
        );
        self.speaking = false;
        flush();
    }

    /// Asks the person about an action the model wants to take, why when a caution says so,
    /// and, for a command that would run outside the sandbox, the model's reason; the answer
    /// is typed next.
    pub(crate) fn approval(
        &mut self,
        summary: &str,
        caution: Option<&str>,
        outside: Option<&OutsideSandbox>,
    ) {
        print!("{}", approval_prompt(self.color, summary, caution, outside));
        self.speaking = false;
        flush();
    }

    /// A plan a turn proposed, as the server says it, a line for each step; with `asks`, the
    /// question after it, whose answer is typed next.
    pub(crate) fn plan(&mut self, plan: &PlanInfo, asks: bool) {
        print!("{}", crate::client::plan::shown(self.color, plan, asks));
        self.speaking = false;
        flush();
    }

    /// One of the model's questions; the answer is typed next.
    pub(crate) fn ask_question(&mut self, question: &Question) {
        print!("{}", crate::asking::shown(self.color, question));
        self.speaking = false;
        flush();
    }

    /// What the person can change to fix the error just shown.
    pub(crate) fn fix(&mut self, text: &str) {
        println!("  {} {text}", paint(self.color, YELLOW, "fix:"));
        flush();
    }

    /// How a goal ended, the tokens its turns used, and, when paused, what the model needs.
    pub(crate) fn goal_ended(&mut self, reason: GoalReason, tokens: u64, note: Option<&str>) {
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
    pub(crate) fn warning(&mut self, message: &str) {
        println!("{} {message}", paint(self.color, YELLOW, "warning"));
        flush();
    }

    /// What glave is doing before the turn starts.
    pub(crate) fn status(&mut self, message: &str) {
        println!("{}", paint(self.color, DIM, message));
        flush();
    }

    pub(crate) fn error(&mut self, message: &str) {
        println!("\n\n{} {message}", paint(self.color, RED, "error"));
        self.speaking = false;
        flush();
    }
}

/// The most of an error shown on a retry line.
const ERROR_CHARS: usize = 200;

/// What a turn did and cost, as the server says, after the protections it ran under: the
/// commands that ran outside the sandbox or with its network, if any, then the calls, the
/// tokens and the cost.
fn receipt_line(protections: &str, summary: &TurnResult) -> String {
    let tokens = &summary.tokens;
    let mut line = format!("{protections} · ");
    let let_out = [
        (summary.outside_sandbox, "outside the sandbox"),
        (summary.with_network, "with the network"),
    ];
    for (count, how) in let_out.into_iter().filter(|(count, _)| *count > 0) {
        let _ = write!(line, "{count} command{} {how} · ", plural(u64::from(count)));
    }
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
    let cost = summary
        .cost
        .map_or_else(|| "cost not reported".to_owned(), format_money);
    let _ = write!(line, " · {cost}");
    if summary.estimated {
        line.push_str(" (estimated)");
    }
    line
}

/// An approval's prompt: what would run, why it asks on a line of its own, the model's reason
/// for running outside the sandbox as its words, then the answers. Running outside the sandbox
/// is allowed one command at a time, so it offers no "always".
fn approval_prompt(
    color: bool,
    summary: &str,
    caution: Option<&str>,
    outside: Option<&OutsideSandbox>,
) -> String {
    let mut prompt = format!("\n\n{} {summary}\n", paint(color, YELLOW, "needs you"));
    if let Some(caution) = caution {
        let _ = writeln!(prompt, "  {}", paint(color, YELLOW, caution));
    }
    if let Some(reason) = outside.and_then(|outside| outside.reason.as_deref()) {
        let _ = writeln!(prompt, "  The model says: \"{reason}\"");
    }
    prompt.push_str(if outside.is_some() {
        "  allow? [y] once  [N] no › "
    } else {
        "  allow? [y] once  [a] always in this thread  [N] no › "
    });
    prompt
}

/// Why an action did not run.
fn not_run(no_one_there: bool) -> &'static str {
    if no_one_there {
        "not run: no one here to approve it"
    } else {
        "declined"
    }
}

/// An action that did not run, and why. With no one here, also why it would have asked, on a
/// line of its own: no prompt showed it.
fn declined(color: bool, summary: &str, caution: Option<&str>, no_one_there: bool) -> String {
    let mut line = format!(
        "\n  {} {}\n",
        paint(color, DIM, &format!("{summary} …")),
        paint(color, YELLOW, not_run(no_one_there))
    );
    if let Some(caution) = caution.filter(|_| no_one_there) {
        let _ = writeln!(line, "    {}", paint(color, DIM, caution));
    }
    line
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
    let amount = if micros < 10_000 && micros > 0 {
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
    fn an_approval_says_why_it_asks_apart_from_what_would_run() {
        assert_eq!(
            approval_prompt(
                false,
                "bash: rm -rf ~/notes",
                Some("It deletes /home/ana/notes, outside the project."),
                None
            ),
            "\n\nneeds you bash: rm -rf ~/notes\n  It deletes /home/ana/notes, outside the \
             project.\n  allow? [y] once  [a] always in this thread  [N] no › "
        );
        assert_eq!(
            approval_prompt(false, "bash: touch a", None, None),
            "\n\nneeds you bash: touch a\n  allow? [y] once  [a] always in this thread  [N] no › "
        );
    }

    #[test]
    fn running_outside_the_sandbox_shows_the_model_s_reason_as_its_words_and_asks_once() {
        let caution =
            Some("It runs outside the sandbox, where it can read and change any file you can.");
        let outside = OutsideSandbox {
            reason: Some("cargo add writes to ~/.cargo/registry, which the sandbox blocks.".into()),
            ..OutsideSandbox::default()
        };
        assert_eq!(
            approval_prompt(false, "bash: cargo add serde", caution, Some(&outside)),
            "\n\nneeds you bash: cargo add serde\n  It runs outside the sandbox, where it can \
             read and change any file you can.\n  The model says: \"cargo add writes to \
             ~/.cargo/registry, which the sandbox blocks.\"\n  allow? [y] once  [N] no › "
        );
        let unexplained = OutsideSandbox::default();
        assert_eq!(
            approval_prompt(false, "bash: cargo add serde", caution, Some(&unexplained)),
            "\n\nneeds you bash: cargo add serde\n  It runs outside the sandbox, where it can \
             read and change any file you can.\n  allow? [y] once  [N] no › "
        );
    }

    #[test]
    fn the_receipt_counts_the_commands_that_ran_outside_the_sandbox() {
        let mut turn = TurnResult {
            outcome: Outcome::Completed,
            model_calls: 2,
            tool_calls: 1,
            outside_sandbox: 0,
            with_network: 0,
            tokens: glave_event::TokenCounts {
                input: 1_200,
                cache_read: 0,
                cache_write: 0,
                output: 34,
                reasoning: 0,
            },
            cost: Some(Money {
                micros: 0,
                currency: Currency::USD,
            }),
            estimated: false,
            via: Vec::new(),
        };
        let protections = "permissions: auto · sandbox: project + network";
        assert_eq!(
            receipt_line(protections, &turn),
            "permissions: auto · sandbox: project + network · 2 model calls · 1 tool call · 1,234 \
             tokens · $0.00"
        );
        turn.outside_sandbox = 1;
        assert_eq!(
            receipt_line(protections, &turn),
            "permissions: auto · sandbox: project + network · 1 command outside the sandbox · 2 \
             model calls · 1 tool call · 1,234 tokens · $0.00"
        );
        (turn.outside_sandbox, turn.with_network) = (0, 2);
        assert_eq!(
            receipt_line("permissions: auto · sandbox: project", &turn),
            "permissions: auto · sandbox: project · 2 commands with the network · 2 model calls · \
             1 tool call · 1,234 tokens · $0.00"
        );
        (turn.with_network, turn.cost, turn.estimated) = (0, None, true);
        assert_eq!(
            receipt_line("permissions: auto · sandbox: project", &turn),
            "permissions: auto · sandbox: project · 2 model calls · 1 tool call · 1,234 tokens · \
             cost not reported (estimated)"
        );
    }

    #[test]
    fn an_action_not_run_says_why_it_asked_only_when_no_prompt_did() {
        let push = Some("It pushes to a remote.");
        assert_eq!(
            declined(false, "bash: git push", push, true),
            "\n  bash: git push … not run: no one here to approve it\n    It pushes to a \
             remote.\n"
        );
        assert_eq!(
            declined(false, "bash: git push", push, false),
            "\n  bash: git push … declined\n"
        );
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
        assert_eq!(format_money(usd(0)), "$0.00");
        let eur = Money {
            micros: 2_500_000,
            currency: Currency::new("EUR").unwrap(),
        };
        assert_eq!(format_money(eur), "2.50 EUR");
    }
}
