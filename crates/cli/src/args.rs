//! Command-line arguments.

use std::path::PathBuf;

use glave_harness::{ApprovalMode, SandboxLevel};

pub const USAGE: &str = "\
glave: run hope, or any model, on the project in this folder

Usage:
  glave [options] [prompt...]   run one turn with this prompt
  glave [options]               start a session; each line you type is a turn
  glave serve --stdio [options] serve the session API (JSON-RPC 2.0, one message per line)
                                to the program that started glave

Options:
  --model <name>     the model to use (default: $GLAVE_MODEL)
  --continue, -c     continue your last thread in this folder
  --approval <mode>  how much to ask before acting: ask (every change and command), edits
                     (file changes go through, commands ask; the default), or auto
                     (nothing asks but secret files). With no one at the keyboard, anything
                     that would ask is declined
  --yes, -y          the same as --approval auto
  --sandbox <level>  run commands in the macOS sandbox: read-only, or project (writes only
                     the project and temp folder); never reads secret locations
  --network          let sandboxed commands use the network
  --events <file>    save the thread's events to this file as JSON Lines
  --help             show this help
  --version          show the version

Environment:
  GLAVE_BASE_URL     an OpenAI-compatible API (default: http://localhost:11434/v1, Ollama)
  GLAVE_MODEL        the model name, such as gemma4:12b or an OpenRouter model id
  GLAVE_API_KEY      the API key, if the endpoint needs one
  GLAVE_PROVIDER     a short name recorded with each response (default: from the URL)
  GLAVE_HOME         where glave keeps its database and your own AGENTS.md (default: ~/.glave)
  GLAVE_CONTEXT_WINDOW  the model's context window in tokens (default: 128000); glave
                     summarizes a thread near 80% of it
  GLAVE_IDLE_TIMEOUT    seconds the model may send nothing before a call fails (default:
                     300); failures that may pass are tried 4 more times

A new thread starts with your AGENTS.md (in GLAVE_HOME), then every AGENTS.md from the
repository's root down to this folder.

In a session, /compact summarizes the thread now, and /exit leaves.";

/// What to do.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    #[default]
    Run,
    /// Serve the session API to another program over stdin and stdout.
    Serve,
    Help,
    Version,
}

/// What the person asked for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub action: Action,
    pub model: Option<String>,
    /// How much to ask before acting; `edits` when not given.
    pub approval: Option<ApprovalMode>,
    /// Run commands in the sandbox at this level.
    pub sandbox: Option<SandboxLevel>,
    /// Let sandboxed commands use the network.
    pub network: bool,
    /// Continue the most recent thread in this folder instead of starting one.
    pub resume: bool,
    pub events: Option<PathBuf>,
    /// The one-shot prompt; empty starts an interactive session.
    pub prompt: String,
}

pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args::default();
    let mut words = Vec::new();
    let mut rest = arguments.into_iter().peekable();
    // `serve` is a subcommand only as the first word; a prompt starting with it goes after `--`.
    let serving = rest.next_if(|first| first == "serve").is_some();
    let mut stdio = false;
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--model" => args.model = Some(rest.next().ok_or("--model needs a name")?),
            "--events" => {
                args.events = Some(PathBuf::from(rest.next().ok_or("--events needs a file")?));
            }
            "--yes" | "-y" => args.approval = Some(ApprovalMode::Auto),
            "--approval" => {
                let mode = rest
                    .next()
                    .ok_or("--approval needs a mode: ask, edits or auto")?;
                args.approval = Some(ApprovalMode::named(&mode).ok_or_else(|| {
                    format!("unknown approval mode {mode:?}: use ask, edits or auto")
                })?);
            }
            "--sandbox" => {
                let level = rest
                    .next()
                    .ok_or("--sandbox needs a level: read-only or project")?;
                args.sandbox = Some(SandboxLevel::named(&level).ok_or_else(|| {
                    format!("unknown sandbox level {level:?}: use read-only or project")
                })?);
            }
            "--network" => args.network = true,
            "--stdio" if serving => stdio = true,
            "--continue" | "-c" => args.resume = true,
            "--help" | "-h" => args.action = Action::Help,
            "--version" | "-V" => args.action = Action::Version,
            "--" => words.extend(rest.by_ref()),
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            _ => words.push(argument),
        }
    }
    if args.network && args.sandbox.is_none() {
        return Err("--network only applies with --sandbox".into());
    }
    args.prompt = words.join(" ");
    if serving && args.action == Action::Run {
        if !stdio {
            return Err("glave serve needs a transport: --stdio".into());
        }
        if !args.prompt.is_empty() || args.resume || args.events.is_some() {
            return Err(
                "glave serve takes no prompt, --continue or --events: clients choose".into(),
            );
        }
        args.action = Action::Serve;
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(words: &[&str]) -> Result<Args, String> {
        parse(words.iter().map(|w| (*w).to_owned()))
    }

    #[test]
    fn prompt_words_join_and_flags_parse() {
        let args = parse_words(&["--yes", "fix", "the", "--model", "m", "test"]).unwrap();
        assert_eq!(args.approval, Some(ApprovalMode::Auto));
        assert_eq!(args.model.as_deref(), Some("m"));
        assert_eq!(args.prompt, "fix the test");
    }

    #[test]
    fn the_sandbox_takes_a_level_and_optional_network() {
        let args = parse_words(&["--sandbox", "project", "--network", "go"]).unwrap();
        assert_eq!(args.sandbox, Some(SandboxLevel::Project));
        assert!(args.network);
        assert!(parse_words(&["--sandbox", "everything"]).is_err());
        assert!(parse_words(&["--sandbox"]).is_err());
        assert!(
            parse_words(&["--network"]).is_err(),
            "network needs a sandbox"
        );
    }

    #[test]
    fn serve_is_a_subcommand_only_as_the_first_word() {
        let serve = parse_words(&["serve", "--stdio", "--approval", "auto"]).unwrap();
        assert_eq!(serve.action, Action::Serve);
        assert_eq!(serve.approval, Some(ApprovalMode::Auto));
        assert!(parse_words(&["serve"]).unwrap_err().contains("--stdio"));
        assert!(parse_words(&["serve", "--stdio", "hello"]).is_err());
        assert!(
            parse_words(&["--stdio"]).is_err(),
            "only serve takes --stdio"
        );
        let prompt = parse_words(&["--", "serve", "the", "page"]).unwrap();
        assert_eq!(
            (prompt.action, prompt.prompt.as_str()),
            (Action::Run, "serve the page")
        );
    }

    #[test]
    fn approval_modes_parse_and_yes_means_auto() {
        assert_eq!(parse_words(&["hi"]).unwrap().approval, None);
        let ask = parse_words(&["--approval", "ask", "hi"]).unwrap();
        assert_eq!(ask.approval, Some(ApprovalMode::Ask));
        assert!(parse_words(&["--approval", "bypass"]).is_err());
        assert!(parse_words(&["--approval"]).is_err());
    }

    #[test]
    fn errors_are_explained() {
        assert!(parse_words(&["--model"]).is_err());
        assert!(parse_words(&["--bogus"]).is_err());
        assert_eq!(parse_words(&["--", "--yes"]).unwrap().prompt, "--yes");
    }
}
