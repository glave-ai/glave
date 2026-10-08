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
    /// The file still holds every line it started with, in order: new lines may be added (a
    /// regression test), but nothing may be changed or removed (a weakened test).
    Kept(&'static str),
    /// The file does not exist.
    Missing(&'static str),
    /// The file exists.
    Exists(&'static str),
    /// This shell command succeeds in the project afterwards.
    Succeeds(&'static str),
    /// Every turn stayed in one thread.
    OneThread,
    /// This text never appears in the thread's events, so it never reached the model.
    NeverSent(&'static str),
    /// This text appears in the thread's events: the model was told it.
    Sent(&'static str),
    /// An event of this type was recorded, such as `thread.compacted`.
    Recorded(&'static str),
}

/// What a check can look at.
pub struct Context<'a> {
    pub trace: &'a Trace,
    /// The thread's events exactly as exported.
    pub events: &'a str,
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
            Self::Kept(path) => kept(cx, path),
            Self::Missing(path) => holds(!cx.project.join(path).exists(), || {
                format!("{path} was created")
            }),
            Self::Succeeds(command) => succeeds(cx, command),
            Self::OneThread => holds(cx.trace.threads == 1, || {
                format!("the turns used {} threads", cx.trace.threads)
            }),
            Self::Exists(path) => holds(cx.project.join(path).exists(), || {
                format!("{path} was not created")
            }),
            Self::Recorded(kind) => {
                holds(cx.events.contains(&format!("\"type\":\"{kind}\"")), || {
                    format!("no {kind} event was recorded")
                })
            }
            Self::Sent(text) => holds(cx.events.contains(text), || {
                format!("{text:?} was never sent to the model")
            }),
            Self::NeverSent(text) => holds(!cx.events.contains(text), || {
                format!("{text:?} reached the model")
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

fn kept(cx: &Context<'_>, path: &str) -> Result<(), String> {
    let original = cx
        .files
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, contents)| *contents)
        .ok_or_else(|| format!("{path} is not one of the task's files"))?;
    let now = std::fs::read_to_string(cx.project.join(path))
        .map_err(|err| format!("could not read {path}: {err}"))?;
    let mut remaining = now.lines();
    match original
        .lines()
        .find(|line| !remaining.any(|candidate| candidate == *line))
    {
        Some(line) => Err(format!("{path} lost or changed the line {:?}", line.trim())),
        None => Ok(()),
    }
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
            events: "",
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
    fn a_kept_file_may_grow_but_not_change() {
        let dir = std::env::temp_dir().join(format!("glave-kept-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files = [("t.rs", "fn a() {}\nfn b() {}\n")];
        let trace = answering("done");
        let judge_with = |contents: &str| {
            std::fs::write(dir.join("t.rs"), contents).unwrap();
            Check::Kept("t.rs").judge(&Context {
                trace: &trace,
                events: "",
                project: &dir,
                model: "m",
                files: &files,
                cargo_target: &dir,
            })
        };
        assert_eq!(judge_with("fn a() {}\nfn new() {}\nfn b() {}\n"), Ok(()));
        assert!(judge_with("fn a() {}\n").is_err(), "a test removed");
        assert!(judge_with("fn b() {}\nfn a() {}\n").is_err(), "reordered");
        assert!(
            judge_with("fn a() { ignore }\nfn b() {}\n").is_err(),
            "a test changed"
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
