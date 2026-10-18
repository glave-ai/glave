//! Command-line arguments.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use glave_harness::ApprovalMode;

use glave_config::policy::{On, SandboxChoice};

pub const USAGE: &str = "\
glave: run hope, or any model, on the project in this folder

Usage:
  glave [options] [prompt...]   run one turn with this prompt
  glave [options]               start a session; each line you type is a turn. With no
                                model set up yet, glave opens in your browser instead
  glave app                     open glave in your browser, signed in on this computer;
                                from a terminal, it starts glave in the background first
                                when it is not running (glave server --help)
  glave --goal <objective> [--goal-budget <tokens>] [options]
                                keep working across turns until the model has checked the
                                goal is met, or pauses it because it needs you. Say how to
                                check in the objective (\"... until cargo test passes\").
                                Stops when its token budget (default 1000000) runs out,
                                after three turns in a row without a tool call, or on Ctrl-C
  glave --background <prompt> [--budget <tokens>], or -b
                                run the turn as a task, with no one attached, in this
                                folder's task host, so it goes on when you leave; with
                                --goal, work toward the goal (glave tasks --help)
  glave tasks [stop <thread>]   list the tasks running and recently finished, or stop one
  glave schedule add|list|rm    run a prompt or a goal as a task at set times
                                (glave schedule --help)
  glave serve --stdio [options] serve the session API (JSON-RPC 2.0, one message per line)
                                to the program that started glave
  glave serve --socket [options]
                                be this folder's task host: serve the session API on a Unix
                                socket in GLAVE_HOME/run and run its tasks and schedules
                                until nothing is left (glave starts it when needed)
  glave serve [--http <address> [--issuer <url> --audience <name>]] [options]
                                run glave for you in the foreground, as a service or a
                                container does: one per GLAVE_HOME (a second one exits),
                                on the Unix socket GLAVE_HOME/run/server.sock and over
                                HTTP, on 127.0.0.1:4180 (any free port when that is taken)
                                unless --http says where. A browser on this computer signs
                                in by a link glave prints or opens, good once for five
                                minutes, and stays signed in; programs here use the token
                                in GLAVE_HOME/serve.token, made at every start; with
                                --issuer and --audience, clients elsewhere sign in with an
                                OIDC access token from that issuer for that audience.
                                The address is host:port, or a port on 127.0.0.1; without
                                --issuer it must be this computer's. The listener has no
                                TLS, so put it behind a proxy that has.
                                --issuer may be given more than once. The web app is
                                served at / too: from --web <folder> (the app's build),
                                else the one built into glave, else the web folder beside
                                the glave binary, if any. Anyone who reaches the listener
                                gets a folder's files without signing in, so it must hold
                                only the build and must not be writable by an agent
  glave server status|stop|restart
                                whether glave runs for you in the background, stop it, or
                                restart it from this terminal (glave server --help)
  glave gateway serve --http <address> --issuer <url> --audience <name>
                                serve the gateway to an organization: model calls and the
                                tool catalog, every call signed in (glave gateway --help)
  glave computer serve --stdio | --tcp <port>
                                serve this machine as a computer to a harness elsewhere
                                (glave computer --help)
  glave policy explain [options] show every setting and where it came from, and every
                                constraint and the policy file that set it
  glave skills                  list the skills the model is offered here
  glave threads [archive|unarchive|delete <id>]
                                list this folder's threads, or archive, restore or delete one
                                (glave threads --help)
  glave worktrees [prune]       list the worktrees threads made, or remove the merged ones
                                (glave worktrees --help)
  glave commands                list the custom commands you can type here as /name
  glave connect github | <address>
                                connect GitHub, or any git host with a token, so git and the
                                host's own tool sign in as you (glave connect --help)
  glave connections [remove <id>]
                                list the git hosts you connected, or remove one

Options:
  --model <name>     the model to use (default: $GLAVE_MODEL)
  --continue, -c     continue your last thread in this folder
  --approval <mode>  how much to ask before acting: auto (only reading secret files asks;
                     the default when commands run in the sandbox) or ask (every change
                     and command asks; the default when they cannot). With no one at the
                     keyboard, anything that would ask is declined
  --sandbox <level>  the wall around commands: project (the default wherever this computer
                     can run it: writes only the project and temp folders, hooks and
                     settings stay read-only), read-only, or off; never reads secrets
  --no-network       keep sandboxed commands off the network
  --network          let sandboxed commands use the network (the default)
  --worktree         work in a new worktree of this repository, on a branch glave/<id>,
                     kept under GLAVE_HOME/worktrees, rather than in this folder itself
  --on <place>       where threads work: here (the default), or cloud: a disposable worker
                     of their own, made from the project's devcontainer.json or
                     .glave/environment.toml, with this repository cloned in at its
                     current commit; work comes back as pushed branches
  --max-steps <n>    end each turn after n model calls, the last one a text report of
                     what is done and what is left (default: no limit)
  --events <file>    save the thread's events to this file as JSON Lines
  --server <url>     be a client of the session server at this URL (or [server] url in
                     config.toml): threads, turns and approvals run there and show here;
                     sign in with GLAVE_SERVER_TOKEN
  --log <file>       append glave's spans and metrics to this file: ids, names, counts and
                     timings, never prompts, answers, file contents or secrets
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
  GLAVE_SERVER_TOKEN an access token for the server --server names
  GLAVE_STORE_URL    a Postgres database to keep history in, as [store] url in config.toml
                     (default: the SQLite file in GLAVE_HOME)
  GLAVE_LOG          how much --log records: error, warn, info (the default), debug or trace
  GLAVE_JUDGE_MODEL     in auto mode, the model that decides whether an action a rule
                     flagged may run because you asked for it (default: the main model;
                     `off` to always ask)

Settings: config.toml in GLAVE_HOME may set model, judge_model, approval, sandbox,
worktree (true or false), on (here or cloud), [workers] (backend = \"docker\" and glave,
a Linux build of glave for workers), mcp (the servers in mcp.json to turn on) and
[telemetry] (otlp = an OpenTelemetry collector's OTLP/HTTP URL, for builds with the otlp
feature: spans and metrics, never content) and [store] (url = a postgres:// database, over TLS
off this computer); flags and the environment win over it. An
organization's policy at /etc/glave/policy.toml limits what may be chosen.

A new thread starts with your AGENTS.md (in GLAVE_HOME), then every AGENTS.md from the
repository's root down to this folder.

Skills (agentskills.io: <name>/SKILL.md with a name and a description) come from .glave/skills,
.agents/skills and .claude/skills in this folder and each one above it up to the repository's
root, then from skills in GLAVE_HOME and ~/.agents/skills; the nearest wins a name. The model
sees each name and description, and loads a skill's instructions when a task needs them.

Commands are <name>.md files in .glave/commands and .claude/commands (in this folder and each
one above it up to the repository's root), then in commands in GLAVE_HOME and
~/.claude/commands. Typing /name words sends the file's text, with $ARGUMENTS replaced by the
words and $1 to $9 by each one.

Hooks are commands config.toml runs at points of a turn: [[hooks.PreToolUse]] (before a tool
call; exit 2 stops it, and the model reads why), [[hooks.PostToolUse]], [[hooks.UserPromptSubmit]]
and [[hooks.Stop]], each with a command, and optionally a matcher (a regular expression over
the tool's name) and a timeout in seconds. A hook reads JSON on its input. A project's hooks
run only once you trust the project.

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
    /// List the custom commands.
    Commands,
    /// Open glave in the browser, signed in.
    App,
    Help,
    Version,
}

/// What the person asked for.
#[derive(Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent command-line flags, each a yes or no"
)]
pub struct Args {
    pub action: Action,
    pub model: Option<String>,
    /// How much to ask before acting; `auto` when not given.
    pub approval: Option<ApprovalMode>,
    /// Run commands in the sandbox at this level, or not at all.
    pub sandbox: Option<SandboxChoice>,
    /// Whether sandboxed commands may use the network; chosen elsewhere, or on, when not given.
    pub network: Option<bool>,
    /// Work in a worktree of the repository of its own, on a new branch.
    pub worktree: bool,
    /// Where new threads work: here, or on a worker (`--on cloud`).
    pub on: Option<On>,
    /// Model calls one turn may make; no limit when not given.
    pub max_steps: Option<u32>,
    /// Continue the most recent thread in this folder instead of starting one.
    pub resume: bool,
    pub events: Option<PathBuf>,
    /// Append glave's spans and metrics to this file.
    pub log: Option<PathBuf>,
    /// Be a client of the session server at this URL.
    pub server: Option<String>,
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
    /// The built web app to serve over HTTP, rather than the one built into glave or beside it.
    pub web: Option<PathBuf>,
    /// Run the turn or goal as a task in the folder's task host, with no one attached.
    pub background: bool,
    /// With `background`, tokens the turn may use.
    pub budget: Option<u64>,
    /// Serve as the folder's task host, on its Unix socket.
    pub socket: bool,
    /// Serve the program that started glave, over stdin and stdout.
    pub stdio: bool,
    /// As the person's server started in the background: a session of its own first
    /// (`crate::server::start` passes it).
    pub own_session: bool,
}

impl Args {
    /// Whether `glave serve` runs as the person's server: not a program's own over stdio, nor
    /// a folder's task host.
    pub fn person_s_server(&self) -> bool {
        self.action == Action::Serve && !self.stdio && !self.socket
    }
}

pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args::default();
    let mut words = Vec::new();
    let mut rest = arguments.into_iter().peekable();
    // `serve` is a subcommand only as the first word; a prompt starting with it goes after `--`.
    let serving = rest.next_if(|first| first == "serve").is_some();
    if !serving {
        args.action = command(&mut rest)?;
    }
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--model" => args.model = Some(rest.next().ok_or("--model needs a name")?),
            "--events" => {
                args.events = Some(PathBuf::from(rest.next().ok_or("--events needs a file")?));
            }
            "--server" => args.server = Some(rest.next().ok_or("--server needs a URL")?),
            "--log" => args.log = Some(PathBuf::from(rest.next().ok_or("--log needs a file")?)),
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
            "--network" => args.network = Some(true),
            "--no-network" => args.network = Some(false),
            "--worktree" => args.worktree = true,
            "--on" => args.on = Some(place(rest.next())?),
            "--max-steps" => args.max_steps = Some(steps(rest.next())?),
            "--goal" => args.goal = Some(rest.next().ok_or("--goal needs an objective")?),
            "--goal-budget" => args.goal_budget = Some(budget(rest.next())?),
            "--background" | "-b" => args.background = true,
            "--budget" => args.budget = Some(tokens("--budget", rest.next())?),
            "--socket" if serving => args.socket = true,
            "--stdio" if serving => args.stdio = true,
            "--own-session" if serving => args.own_session = true,
            "--http" if serving => args.http = Some(address(rest.next())?),
            "--issuer" if serving => args
                .issuers
                .push(rest.next().ok_or("--issuer needs a URL")?),
            "--audience" if serving => {
                args.audience = Some(rest.next().ok_or("--audience needs a name")?);
            }
            "--web" if serving => {
                args.web = Some(rest.next().ok_or("--web needs a folder")?.into());
            }
            "--continue" | "-c" => args.resume = true,
            "--help" | "-h" => args.action = Action::Help,
            "--version" | "-V" => args.action = Action::Version,
            "--" => words.extend(rest.by_ref()),
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            _ => words.push(argument),
        }
    }
    if args.network.is_some() && args.sandbox == Some(SandboxChoice::Off) {
        return Err("--network and --no-network only apply with a sandbox".into());
    }
    args.prompt = words.join(" ");
    alone(&args)?;
    if serving && args.action == Action::Run {
        transport(&args)?;
        args.action = Action::Serve;
    }
    goal(&args)?;
    background(&args)?;
    Ok(args)
}

/// The command the first words name, taking them: `policy explain`, `skills`, `commands` or
/// `app`; otherwise a run, taking nothing.
fn command(rest: &mut std::iter::Peekable<impl Iterator<Item = String>>) -> Result<Action, String> {
    let action = match rest.peek().map(String::as_str) {
        Some("policy") => Action::Explain,
        Some("skills") => Action::Skills,
        Some("commands") => Action::Commands,
        Some("app") => Action::App,
        _ => return Ok(Action::Run),
    };
    rest.next();
    if action == Action::Explain && rest.next_if(|second| second == "explain").is_none() {
        return Err("glave policy takes one command: explain".into());
    }
    Ok(action)
}

/// Checks that the commands that take no prompt got none.
fn alone(args: &Args) -> Result<(), String> {
    let name = match args.action {
        Action::Explain => "policy explain",
        Action::Skills => "skills",
        Action::Commands => "commands",
        Action::App => "app",
        _ => return Ok(()),
    };
    if !args.prompt.is_empty() || args.resume {
        return Err(format!("glave {name} takes no prompt or --continue"));
    }
    Ok(())
}

/// Checks that a task has something to do, and a budget only with a task's turn.
fn background(args: &Args) -> Result<(), String> {
    if args.budget.is_some() && (!args.background || args.goal.is_some()) {
        return Err(
            "--budget is for a turn run with --background; a goal takes --goal-budget".into(),
        );
    }
    if !args.background {
        return Ok(());
    }
    if args.action != Action::Run {
        return Err("--background runs a task: not with this command".into());
    }
    if args.prompt.is_empty() && args.goal.is_none() {
        return Err("--background needs a prompt, or --goal <objective>".into());
    }
    if args.resume || args.events.is_some() {
        return Err("--background starts a new thread: not with --continue or --events".into());
    }
    Ok(())
}

/// A number of tokens for `flag`: at least one.
fn tokens(flag: &str, value: Option<String>) -> Result<u64, String> {
    let value = value.ok_or_else(|| format!("{flag} needs a number of tokens"))?;
    match value.parse::<u64>() {
        Ok(tokens) if tokens >= 1 => Ok(tokens),
        _ => Err(format!(
            "{flag} {value:?} is not a number of tokens: use 1 or more"
        )),
    }
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

/// Checks that `glave serve` has at most one transport (none is the person's server, on its
/// socket and loopback HTTP), and sign-in for HTTP.
fn transport(args: &Args) -> Result<(), String> {
    let stdio = args.stdio;
    if [stdio, args.http.is_some(), args.socket]
        .iter()
        .filter(|chosen| **chosen)
        .count()
        > 1
    {
        return Err("glave serve takes one of --stdio, --socket or --http <address>".into());
    }
    if args.own_session && (stdio || args.socket) {
        return Err("--own-session is for the person's server, not --stdio or --socket".into());
    }
    if !args.prompt.is_empty() || args.resume || args.events.is_some() {
        return Err("glave serve takes no prompt, --continue or --events: clients choose".into());
    }
    if args.web.is_some() && (stdio || args.socket) {
        return Err("--web is for HTTP: the web app is served over HTTP".into());
    }
    let signs_in = !args.issuers.is_empty() || args.audience.is_some();
    if (stdio || args.socket) && signs_in {
        return Err(
            "--issuer and --audience are for --http; stdio and the socket need no sign-in".into(),
        );
    }
    if let Some(address) = args.http {
        if args.issuers.is_empty() != args.audience.is_none() {
            return Err("--issuer and --audience go together: OIDC needs both".into());
        }
        if args.issuers.is_empty() && !address.ip().is_loopback() {
            return Err(format!(
                "glave serve --http {address} would listen beyond this computer: without \
                 --issuer <url> and --audience <name> only a browser here can sign in, so \
                 listen on 127.0.0.1 or ::1"
            ));
        }
    }
    Ok(())
}

/// The value of `--http`: `host:port`, or a port alone on 127.0.0.1.
pub fn http_address(value: Option<String>) -> Result<SocketAddr, String> {
    address(value)
}

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
/// Where threads work, as `--on` names it.
fn place(value: Option<String>) -> Result<On, String> {
    let place = value.ok_or("--on needs a place: here or cloud")?;
    On::named(&place).ok_or_else(|| format!("unknown place {place:?}: use here or cloud"))
}

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
        assert_eq!(args.network, Some(true));
        let offline = parse_words(&["--no-network", "go"]).unwrap();
        assert_eq!(offline.network, Some(false));
        assert_eq!(parse_words(&["go"]).unwrap().network, None);
        let off = parse_words(&["--sandbox", "off", "go"]).unwrap();
        assert_eq!(off.sandbox, Some(SandboxChoice::Off));
        assert!(parse_words(&["--sandbox", "everything"]).is_err());
        assert!(parse_words(&["--sandbox"]).is_err());
        assert!(
            parse_words(&["--sandbox", "off", "--network"]).is_err(),
            "network needs a sandbox"
        );
        assert!(
            parse_words(&["--sandbox", "off", "--no-network"]).is_err(),
            "no network needs a sandbox"
        );
        assert!(
            parse_words(&["--network"]).is_ok(),
            "a config file may choose the sandbox"
        );
    }

    #[test]
    fn a_worktree_is_asked_for_by_flag_and_off_otherwise() {
        assert!(parse_words(&["--worktree", "go"]).unwrap().worktree);
        assert!(!parse_words(&["go"]).unwrap().worktree);
    }

    #[test]
    fn serve_is_a_subcommand_only_as_the_first_word() {
        let serve = parse_words(&["serve", "--stdio", "--approval", "auto"]).unwrap();
        assert_eq!(serve.action, Action::Serve);
        assert_eq!(serve.approval, Some(ApprovalMode::Auto));
        let person = parse_words(&["serve"]).unwrap();
        assert!(person.person_s_server() && !person.own_session);
        assert!(
            parse_words(&["serve", "--own-session"])
                .unwrap()
                .own_session
        );
        assert!(parse_words(&["serve", "--stdio", "--own-session"]).is_err());
        assert!(!serve.person_s_server(), "a program's own server");
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
        // Without OIDC, only a browser on this computer signs in, with the local token.
        let web = parse_words(&["serve", "--http", "8080", "--web", "dist/app"]).unwrap();
        assert_eq!(web.web, Some(PathBuf::from("dist/app")));
        assert!(parse_words(&["serve", "--stdio", "--web", "dist/app"]).is_err());
        assert!(parse_words(&["serve", "--http", "8080", "--web"]).is_err());
        let local = parse_words(&["serve", "--http", "8080"]).unwrap();
        assert_eq!(local.http, Some("127.0.0.1:8080".parse().unwrap()));
        assert!(parse_words(&["serve", "--http", "[::1]:8080"]).is_ok());
        let exposed = parse_words(&["serve", "--http", "0.0.0.0:9"]).unwrap_err();
        assert!(exposed.contains("--issuer"), "{exposed}");
        let half = parse_words(&["serve", "--http", "8080", "--issuer", "i"]).unwrap_err();
        assert!(half.contains("go together"), "{half}");
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
        assert_eq!(parse_words(&["commands"]).unwrap().action, Action::Commands);
        assert_eq!(parse_words(&["app"]).unwrap().action, Action::App);
        assert!(parse_words(&["app", "now"]).is_err());
        assert_eq!(
            parse_words(&["open", "the", "app"]).unwrap().action,
            Action::Run
        );
        assert!(parse_words(&["commands", "now"]).is_err());
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
    fn background_takes_a_prompt_or_a_goal_and_a_budget_only_for_a_turn() {
        let task = parse_words(&["-b", "--budget", "5000", "tidy", "up"]).unwrap();
        assert!(task.background);
        assert_eq!((task.budget, task.prompt.as_str()), (Some(5000), "tidy up"));
        let goal = parse_words(&["--background", "--goal", "pass", "--goal-budget", "9"]).unwrap();
        assert!(goal.background && goal.goal.is_some());
        assert!(parse_words(&["-b"]).is_err(), "nothing to do");
        assert!(
            parse_words(&["--budget", "5", "hi"]).is_err(),
            "only for a task"
        );
        assert!(parse_words(&["-b", "--goal", "x", "--budget", "5"]).is_err());
        assert!(parse_words(&["-b", "--continue", "hi"]).is_err());
        assert!(parse_words(&["-b", "--budget", "0", "hi"]).is_err());
        let host = parse_words(&["serve", "--socket", "--model", "m"]).unwrap();
        assert!(host.socket && host.action == Action::Serve);
        assert!(parse_words(&["serve", "--socket", "--stdio"]).is_err());
        assert!(
            parse_words(&["--socket"]).is_err(),
            "only serve takes --socket"
        );
    }

    #[test]
    fn errors_are_explained() {
        assert!(parse_words(&["--model"]).is_err());
        assert!(parse_words(&["--bogus"]).is_err());
        assert_eq!(parse_words(&["--", "--yes"]).unwrap().prompt, "--yes");
    }
}
