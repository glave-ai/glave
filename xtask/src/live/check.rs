//! The checks a task must pass, judged only from the trace and the files left behind. No
//! model grades another, so a run needs no person and no judge.

use std::path::Path;
use std::process::{Command, Stdio};

use glave_event::Outcome;

use super::trace::Trace;

/// One thing a finished task must show.
#[derive(Debug, Clone, Copy)]
pub enum Check {
    /// Every turn ended normally.
    Completed,
    /// The final answer contains this text, ignoring case.
    AnswerContains(&'static str),
    /// The final answer contains none of these, ignoring case.
    AnswerLacks(&'static [&'static str]),
    /// The final answer names the model that ran, by a word of its short name.
    AnswerNamesModel,
    /// The file still holds exactly what the task started with.
    Unchanged(&'static str),
    /// The file does not exist.
    Missing(&'static str),
    /// This shell command succeeds in the project afterwards.
    Succeeds(&'static str),
    /// Every turn stayed in one thread.
    OneThread,
}

/// What a check can look at.
pub struct Context<'a> {
    pub trace: &'a Trace,
    pub project: &'a Path,
    pub model: &'a str,
    /// The task's starting files, to tell what changed.
    pub files: &'a [(&'a str, &'a str)],
    /// Where commands build, shared across tasks so builds are cached.
    pub cargo_target: &'a Path,
}

impl Check {
    /// `Ok` when the check holds, otherwise what went wrong.
    pub fn judge(self, cx: &Context<'_>) -> Result<(), String> {
        let answer = &cx.trace.answer;
        match self {
            Self::Completed => completed(&cx.trace.outcomes),
            Self::AnswerContains(text) => holds(mentions(answer, text), || {
                format!("the answer does not mention {text:?}")
            }),
            Self::AnswerLacks(texts) => match texts.iter().find(|text| mentions(answer, text)) {
                Some(text) => Err(format!("the answer says {text:?}")),
                None => Ok(()),
            },
            Self::AnswerNamesModel => {
                let words = model_words(cx.model);
                holds(words.iter().any(|word| mentions(answer, word)), || {
                    format!("the answer names none of {words:?}")
                })
            }
            Self::Unchanged(path) => unchanged(cx, path),
            Self::Missing(path) => holds(!cx.project.join(path).exists(), || {
                format!("{path} was created")
            }),
            Self::Succeeds(command) => succeeds(cx, command),
            Self::OneThread => holds(cx.trace.threads == 1, || {
                format!("the turns used {} threads", cx.trace.threads)
            }),
        }
    }
}

fn holds(ok: bool, failure: impl FnOnce() -> String) -> Result<(), String> {
    if ok { Ok(()) } else { Err(failure()) }
}

fn completed(outcomes: &[Outcome]) -> Result<(), String> {
    match outcomes
        .iter()
        .find(|outcome| **outcome != Outcome::Completed)
    {
        _ if outcomes.is_empty() => Err("no turn ended".to_owned()),
        Some(outcome) => Err(format!("a turn ended {outcome:?}")),
        None => Ok(()),
    }
}

fn unchanged(cx: &Context<'_>, path: &str) -> Result<(), String> {
    let original = cx
        .files
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, contents)| *contents)
        .ok_or_else(|| format!("{path} is not one of the task's files"))?;
    let now = std::fs::read_to_string(cx.project.join(path))
        .map_err(|err| format!("could not read {path}: {err}"))?;
    holds(now == original, || format!("{path} was changed"))
}

fn succeeds(cx: &Context<'_>, command: &str) -> Result<(), String> {
    let status = Command::new("sh")
        .args(["-c", command])
        .current_dir(cx.project)
        .env("CARGO_TARGET_DIR", cx.cargo_target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|err| format!("could not run `{command}`: {err}"))?;
    holds(status.success(), || {
        format!("`{command}` fails afterwards ({status})")
    })
}

/// Whether `text` appears in `haystack`, ignoring case and curly apostrophes.
fn mentions(haystack: &str, text: &str) -> bool {
    haystack
        .to_lowercase()
        .replace('\u{2019}', "'")
        .contains(&text.to_lowercase())
}

/// Words that do not identify a model.
const GENERIC: &[&str] = &["instruct", "free", "latest", "preview", "chat", "thinking"];

/// The distinctive words of a model's short name: `anthropic/claude-haiku-5.5` gives
/// `claude` and `haiku`.
fn model_words(model: &str) -> Vec<String> {
    let short = model.rsplit('/').next().unwrap_or(model);
    short
        .split(|c: char| !c.is_ascii_alphanumeric())
        .map(str::to_ascii_lowercase)
        .filter(|word| word.len() >= 3 && word.chars().any(|c| c.is_ascii_alphabetic()))
        .filter(|word| !GENERIC.contains(&word.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn judge(check: Check, trace: &Trace) -> Result<(), String> {
        check.judge(&Context {
            trace,
            project: Path::new("/nonexistent"),
            model: "anthropic/claude-haiku-5.5",
            files: &[],
            cargo_target: Path::new("/nonexistent"),
        })
    }

    fn answering(answer: &str) -> Trace {
        Trace {
            answer: answer.to_owned(),
            outcomes: vec![Outcome::Completed],
            threads: 1,
            ..Trace::default()
        }
    }

    #[test]
    fn model_words_skip_vendors_versions_and_generic_words() {
        assert_eq!(
            model_words("anthropic/claude-haiku-5.5"),
            ["claude", "haiku"]
        );
        assert_eq!(model_words("google/gemma-4-31b-it:free"), ["gemma", "31b"]);
        assert_eq!(model_words("gemma4:12b"), ["gemma4", "12b"]);
    }

    #[test]
    fn answers_are_matched_ignoring_case_and_curly_apostrophes() {
        let trace = answering("I\u{2019}m Claude Haiku, made by Anthropic.");
        assert_eq!(judge(Check::AnswerNamesModel, &trace), Ok(()));
        assert_eq!(judge(Check::AnswerContains("ANTHROPIC"), &trace), Ok(()));
        assert!(judge(Check::AnswerLacks(&["i'm claude"]), &trace).is_err());
        assert_eq!(judge(Check::AnswerLacks(&["i'm hope"]), &trace), Ok(()));
    }

    #[test]
    fn every_turn_must_complete() {
        let mut trace = answering("done");
        assert_eq!(judge(Check::Completed, &trace), Ok(()));
        trace.outcomes.push(Outcome::LimitReached);
        assert!(judge(Check::Completed, &trace).is_err());
        trace.outcomes.clear();
        assert_eq!(
            judge(Check::Completed, &trace),
            Err("no turn ended".to_owned())
        );
    }

    #[test]
    fn turns_must_share_one_thread() {
        let mut trace = answering("done");
        assert_eq!(judge(Check::OneThread, &trace), Ok(()));
        trace.threads = 2;
        assert!(judge(Check::OneThread, &trace).is_err());
    }
}
