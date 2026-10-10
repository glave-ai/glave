//! Command-line arguments.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use glave_harness::ApprovalMode;

use crate::policy::SandboxChoice;

pub const USAGE: &str = "\
glave: run hope, or any model, on the project in this folder

Usage:
  glave [options] [prompt...]   run one turn with this prompt
  glave [options]               start a session; each line you type is a turn
  glave --goal <objective> [--goal-budget <tokens>] [options]
                                keep working across turns until the model has checked the
                                goal is met, or pauses it because it needs you. Say how to
                                check in the objective (\"... until cargo test passes\").
                                Stops when its token budget (default 1000000) runs out,
                                after three turns in a row without a tool call, or on Ctrl-C
  glave serve --stdio [options] serve the session API (JSON-RPC 2.0, one message per line)
                                to the program that started glave
  glave serve --http <address> --issuer <url> --audience <name> [options]
                                serve the session API over HTTP to clients signed in with
                                an OIDC access token from that issuer for that audience.
                                The address is host:port, or a port on 127.0.0.1; the
                                listener has no TLS, so put it behind a proxy that has.
                                --issuer may be given more than once
  glave computer serve --stdio | --tcp <port>
                                serve this machine as a computer to a harness elsewhere
                                (glave computer --help)
  glave policy explain [options] show every setting and where it came from, and every
                                constraint and the policy file that set it
  glave skills                  list the skills the model is offered here

Options:
  --model <name>     the model to use (default: $GLAVE_MODEL)
  --continue, -c     continue your last thread in this folder
  --approval <mode>  how much to ask before acting: auto (the default; only reading secret
                     files asks) or ask (every change and command asks). With no one at
                     the keyboard, anything that would ask is declined
  --sandbox <level>  run commands in the macOS sandbox: read-only, or project (writes only
                     the project and temp folder), or off; never reads secret locations
  --network          let sandboxed commands use the network
  --max-steps <n>    end each turn after n model calls, the last one a text report of
                     what is done and what is left (default: no limit)
  --events <file>    save the thread's events to this file as JSON Lines
  --help             show this help
  --version          show the version

Environment:
  GLAVE_BASE_URL     an OpenAI-compatible API (default: http://localhost:11434/v1, Ollama),
                     or https://api.anthropic.com for Anthropic's own Messages API
  GLAVE_MODEL        the model name, such as gemma4:12b, an OpenRouter model id, or a
                     Claude model id with Anthropic's API
  GLAVE_API_KEY      the API key, if the endpoint needs one
  GLAVE_PROVIDER     a short name recorded with each response (default: from the URL)
  GLAVE_HOME         where glave keeps its database, your own AGENTS.md and mcp.json
                     (default: ~/.glave)
  GLAVE_CONTEXT_WINDOW  the model's context window in tokens (default: 128000); glave
                     summarizes a thread near 80% of it
  GLAVE_IDLE_TIMEOUT    seconds the model may send nothing before a call fails (default:
                     300); failures that may pass are tried 4 more times
  GLAVE_JUDGE_MODEL     in auto mode, the model that decides whether an action a rule
                     flagged may run because you asked for it (default: the main model;
                     `off` to always ask)

Settings: config.toml in GLAVE_HOME may set model, judge_model, approval, sandbox and
mcp (the servers in mcp.json to turn on); flags and the environment win over it. An
organization's policy at /etc/glave/policy.toml limits what may be chosen.

A new thread starts with your AGENTS.md (in GLAVE_HOME), then every AGENTS.md from the
repository's root down to this folder.

Skills (agentskills.io: <name>/SKILL.md with a name and a description) come from .glave/skills,
.agents/skills and .claude/skills in this folder and each one above it up to the repository's
root, then from skills in GLAVE_HOME and ~/.agents/skills; the nearest wins a name. The model
sees each name and description, and loads a skill's instructions when a task needs them.

MCP servers in mcp.json, in the .mcp.json format ({\"mcpServers\": {\"name\": {\"command\": ...}}}
or {\"url\": ..., \"headers\": {...}}), start with glave; their tools reach the model as
mcp__<server>__<tool>. A program gets only the variables its settings name, so add keys
there: \"env\": {\"TOKEN\": \"${TOKEN}\"}.

A repository may bring its own: .mcp.json at its root (the same format) and .glave/config.toml.
glave asks once, only when they would run commands, and remembers the answer in trust.toml
in GLAVE_HOME. With no one to ask, those servers stay off.

In a session, /compact summarizes the thread now, and /exit leaves.";

/// What to do.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    #[default]
    Run,
    /// Serve the session API to another program over stdin and stdout.
    Serve,
    /// Explain the settings and the policy.
    Explain,
    /// List the skills on offer.
    Skills,
    Help,
    Version,
}

/// What the person asked for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub action: Action,
    pub model: Option<String>,
    /// How much to ask before acting; `auto` when not given.
    pub approval: Option<ApprovalMode>,
    /// Run commands in the sandbox at this level, or not at all.
    pub sandbox: Option<SandboxChoice>,
    /// Let sandboxed commands use the network.
    pub network: bool,
    /// Model calls one turn may make; no limit when not given.
    pub max_steps: Option<u32>,
    /// Continue the most recent thread in this folder instead of starting one.
    pub resume: bool,
    pub events: Option<PathBuf>,
    /// The one-shot prompt; empty starts an interactive session.
    pub prompt: String,
    /// Serve over HTTP on this address, rather than stdio.
    pub http: Option<SocketAddr>,
    /// Work toward this objective across turns, instead of running one turn.
    pub goal: Option<String>,
    /// Tokens the goal may use.
    pub goal_budget: Option<u64>,
    /// The identity providers whose tokens sign clients in over HTTP.
    pub issuers: Vec<String>,
    /// The audience those tokens must name.
    pub audience: Option<String>,
}

pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args::default();
    let mut words = Vec::new();
    let mut rest = arguments.into_iter().peekable();
    // `serve` is a subcommand only as the first word; a prompt starting with it goes after `--`.
    let serving = rest.next_if(|first| first == "serve").is_some();
    if !serving && rest.next_if(|first| first == "policy").is_some() {
        if rest.next_if(|second| second == "explain").is_none() {
            return Err("glave policy takes one command: explain".into());
        }
        args.action = Action::Explain;
    } else if !serving && rest.next_if(|first| first == "skills").is_some() {
        args.action = Action::Skills;
    }
    let mut stdio = false;
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--model" => args.model = Some(rest.next().ok_or("--model needs a name")?),
            "--events" => {
                args.events = Some(PathBuf::from(rest.next().ok_or("--events needs a file")?));
            }
            "--approval" => {
                let mode = rest.next().ok_or("--approval needs a mode: auto or ask")?;
                args.approval =
                    Some(ApprovalMode::named(&mode).ok_or_else(|| {
                        format!("unknown approval mode {mode:?}: use auto or ask")
                    })?);
            }
            "--sandbox" => {
                let level = rest
                    .next()
                    .ok_or("--sandbox needs a level: read-only, project or off")?;
                args.sandbox = Some(SandboxChoice::named(&level).ok_or_else(|| {
                    format!("unknown sandbox level {level:?}: use read-only, project or off")
                })?);
            }
            "--network" => args.network = true,
            "--max-steps" => args.max_steps = Some(steps(rest.next())?),
            "--goal" => args.goal = Some(rest.next().ok_or("--goal needs an objective")?),
            "--goal-budget" => args.goal_budget = Some(budget(rest.next())?),
            "--stdio" if serving => stdio = true,
            "--http" if serving => {
                args.http = Some(address(rest.next())?);
            }
            "--issuer" if serving => {
                args.issuers
                    .push(rest.next().ok_or("--issuer needs a URL")?);
            }
            "--audience" if serving => {
                args.audience = Some(rest.next().ok_or("--audience needs a name")?);
            }
            "--continue" | "-c" => args.resume = true,
            "--help" | "-h" => args.action = Action::Help,
            "--version" | "-V" => args.action = Action::Version,
            "--" => words.extend(rest.by_ref()),
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            _ => words.push(argument),
        }
    }
    if args.network && args.sandbox == Some(SandboxChoice::Off) {
        return Err("--network only applies with a sandbox".into());
    }
    args.prompt = words.join(" ");
    if args.action == Action::Explain && (!args.prompt.is_empty() || args.resume) {
        return Err("glave policy explain takes no prompt or --continue".into());
    }
    if args.action == Action::Skills && (!args.prompt.is_empty() || args.resume) {
        return Err("glave skills takes no prompt or --continue".into());
    }
    if serving && args.action == Action::Run {
        transport(&args, stdio)?;
        args.action = Action::Serve;
    }
    goal(&args)?;
    Ok(args)
}

/// Checks that a goal's flags come with `--goal`, and a goal with no prompt beside it.
fn goal(args: &Args) -> Result<(), String> {
    match &args.goal {
        None if args.goal_budget.is_some() => Err("--goal-budget needs --goal <objective>".into()),
        Some(objective) if objective.trim().is_empty() => Err("--goal needs an objective".into()),
        Some(_) if !args.prompt.is_empty() => {
            Err("give the goal's objective with --goal, and no prompt beside it".into())
        }
        Some(_) if args.action != Action::Run => Err("--goal runs a goal: not with serve".into()),
        _ => Ok(()),
    }
}

/// The value of `--goal-budget`: a whole number of tokens, at least one.
fn budget(value: Option<String>) -> Result<u64, String> {
    let value = value.ok_or("--goal-budget needs a number of tokens")?;
    match value.parse::<u64>() {
        Ok(tokens) if tokens >= 1 => Ok(tokens),
        _ => Err(format!(
            "--goal-budget {value:?} is not a number of tokens: use 1 or more"
        )),
    }
}

/// Checks that `glave serve` has one transport, and sign-in for HTTP.
fn transport(args: &Args, stdio: bool) -> Result<(), String> {
    if stdio == args.http.is_some() {
        return Err("glave serve needs one transport: --stdio or --http <address>".into());
    }
    if !args.prompt.is_empty() || args.resume || args.events.is_some() {
        return Err("glave serve takes no prompt, --continue or --events: clients choose".into());
    }
    let signs_in = !args.issuers.is_empty() || args.audience.is_some();
    if stdio && signs_in {
        return Err("--issuer and --audience are for --http; stdio needs no sign-in".into());
    }
    if args.http.is_some() && (args.issuers.is_empty() || args.audience.is_none()) {
        return Err(
            "glave serve --http needs --issuer <url> and --audience <name>: a network \
             listener never runs without sign-in"
                .into(),
        );
    }
    Ok(())
}

/// The value of `--http`: `host:port`, or a port alone on 127.0.0.1.
fn address(value: Option<String>) -> Result<SocketAddr, String> {
    let value = value.ok_or("--http needs an address: host:port, or a port")?;
    if let Ok(port) = value.parse::<u16>() {
        return Ok(SocketAddr::from((Ipv4Addr::LOCALHOST, port)));
    }
    value
        .parse()
        .map_err(|_| format!("--http {value:?} is not host:port or a port"))
}

/// The value of `--max-steps`: a whole number of model calls, at least one.
fn steps(value: Option<String>) -> Result<u32, String> {
    let value = value.ok_or("--max-steps needs a number of model calls")?;
    match value.parse::<u32>() {
        Ok(steps) if steps >= 1 => Ok(steps),
        _ => Err(format!(
            "--max-steps {value:?} is not a number of model calls: use 1 or more"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(words: &[&str]) -> Result<Args, String> {
        parse(words.iter().map(|w| (*w).to_owned()))
    }

    #[test]
    fn prompt_words_join_and_flags_parse() {
        let args =
            parse_words(&["--approval", "ask", "fix", "the", "--model", "m", "test"]).unwrap();
        assert_eq!(args.approval, Some(ApprovalMode::Ask));
        assert_eq!(args.model.as_deref(), Some("m"));
        assert_eq!(args.prompt, "fix the test");
    }

    #[test]
    fn the_sandbox_takes_a_level_and_optional_network() {
        let args = parse_words(&["--sandbox", "project", "--network", "go"]).unwrap();
        assert_eq!(args.sandbox, Some(SandboxChoice::Project));
        assert!(args.network);
        let off = parse_words(&["--sandbox", "off", "go"]).unwrap();
        assert_eq!(off.sandbox, Some(SandboxChoice::Off));
        assert!(parse_words(&["--sandbox", "everything"]).is_err());
        assert!(parse_words(&["--sandbox"]).is_err());
        assert!(
            parse_words(&["--sandbox", "off", "--network"]).is_err(),
            "network needs a sandbox"
        );
        assert!(
            parse_words(&["--network"]).is_ok(),
            "a config file may choose the sandbox"
        );
    }

    #[test]
    fn serve_is_a_subcommand_only_as_the_first_word() {
        let serve = parse_words(&["serve", "--stdio", "--approval", "auto"]).unwrap();
        assert_eq!(serve.action, Action::Serve);
        assert_eq!(serve.approval, Some(ApprovalMode::Auto));
        assert!(parse_words(&["serve"]).unwrap_err().contains("--stdio"));
        assert!(
            parse_words(&["--http", "8080"]).is_err(),
            "only serve takes --http"
        );
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
    fn serving_over_http_needs_sign_in() {
        let https = [
            "serve",
            "--http",
            "8080",
            "--issuer",
            "https://id.example.com",
            "--audience",
            "glave",
        ];
        let serve = parse_words(&https).unwrap();
        assert_eq!(serve.action, Action::Serve);
        assert_eq!(serve.http, Some("127.0.0.1:8080".parse().unwrap()));
        assert_eq!(serve.issuers, ["https://id.example.com"]);
        assert_eq!(serve.audience.as_deref(), Some("glave"));
        let anywhere = parse_words(&[
            "serve",
            "--http",
            "0.0.0.0:9",
            "--issuer",
            "i",
            "--audience",
            "a",
        ]);
        assert_eq!(anywhere.unwrap().http, Some("0.0.0.0:9".parse().unwrap()));
        let unsigned = parse_words(&["serve", "--http", "8080"]).unwrap_err();
        assert!(unsigned.contains("--issuer"), "{unsigned}");
        assert!(parse_words(&["serve", "--http", "8080", "--stdio"]).is_err());
        assert!(parse_words(&["serve", "--stdio", "--audience", "a"]).is_err());
        assert!(parse_words(&["serve", "--http", "nowhere"]).is_err());
    }

    #[test]
    fn approval_modes_parse() {
        assert_eq!(parse_words(&["hi"]).unwrap().approval, None);
        let ask = parse_words(&["--approval", "ask", "hi"]).unwrap();
        assert_eq!(ask.approval, Some(ApprovalMode::Ask));
        assert!(parse_words(&["--approval", "bypass"]).is_err());
        assert!(parse_words(&["--approval", "edits"]).is_err(), "retired");
        assert!(
            parse_words(&["--yes", "hi"]).is_err(),
            "auto is the default"
        );
        assert!(parse_words(&["--approval"]).is_err());
    }

    #[test]
    fn max_steps_takes_a_count_of_at_least_one() {
        assert_eq!(parse_words(&["hi"]).unwrap().max_steps, None);
        let capped = parse_words(&["--max-steps", "80", "hi"]).unwrap();
        assert_eq!((capped.max_steps, capped.prompt.as_str()), (Some(80), "hi"));
        let serve = parse_words(&["serve", "--stdio", "--max-steps", "1"]).unwrap();
        assert_eq!((serve.action, serve.max_steps), (Action::Serve, Some(1)));
        for wrong in ["0", "-3", "ten", "2.5", ""] {
            let error = parse_words(&["--max-steps", wrong]).unwrap_err();
            assert!(error.contains("use 1 or more"), "{wrong}: {error}");
        }
        assert!(
            parse_words(&["--max-steps"])
                .unwrap_err()
                .contains("needs a number")
        );
    }

    #[test]
    fn skills_is_a_command_only_as_the_first_word() {
        assert_eq!(parse_words(&["skills"]).unwrap().action, Action::Skills);
        assert!(parse_words(&["skills", "now"]).is_err());
        let prompt = parse_words(&["list", "skills"]).unwrap();
        assert_eq!(prompt.action, Action::Run);
        assert_eq!(prompt.prompt, "list skills");
    }

    #[test]
    fn policy_explain_is_a_command_that_takes_the_choice_flags() {
        let explain = parse_words(&["policy", "explain", "--approval", "ask"]).unwrap();
        assert_eq!(explain.action, Action::Explain);
        assert_eq!(explain.approval, Some(ApprovalMode::Ask));
        assert!(parse_words(&["policy"]).is_err());
        assert!(parse_words(&["policy", "show"]).is_err());
        assert!(parse_words(&["policy", "explain", "why"]).is_err());
        let prompt = parse_words(&["explain", "policy"]).unwrap();
        assert_eq!(prompt.action, Action::Run);
    }

    #[test]
    fn a_goal_takes_an_objective_and_optionally_a_budget() {
        let goal = parse_words(&["--goal", "make cargo test pass"]).unwrap();
        assert_eq!(goal.goal.as_deref(), Some("make cargo test pass"));
        assert_eq!(goal.goal_budget, None);
        let budgeted = parse_words(&["--goal", "x", "--goal-budget", "50000"]).unwrap();
        assert_eq!(budgeted.goal_budget, Some(50_000));
        let check = parse_words(&["--goal", "x", "--check", "cargo test"]);
        assert!(check.is_err(), "there is no check command");
        assert!(parse_words(&["--goal-budget", "5", "hi"]).is_err());
        assert!(parse_words(&["--goal", "x", "--goal-budget", "0"]).is_err());
        assert!(parse_words(&["--goal", "x", "and", "a", "prompt"]).is_err());
        assert!(parse_words(&["--goal", " "]).is_err());
        assert!(parse_words(&["serve", "--stdio", "--goal", "x"]).is_err());
    }

    #[test]
    fn errors_are_explained() {
        assert!(parse_words(&["--model"]).is_err());
        assert!(parse_words(&["--bogus"]).is_err());
        assert_eq!(parse_words(&["--", "--yes"]).unwrap().prompt, "--yes");
    }
}
