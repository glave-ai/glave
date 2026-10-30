//! Command-line arguments.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use glave_harness::Permissions;

use glave_config::policy::On;

pub(crate) const USAGE: &str = "\
glave: run hope, or any model, on the project in this folder

Usage:
  glave                         open glave in your browser at this folder's project,
                                signed in on this computer (asking once to make a new
                                folder a project; in your home folder, your chats); with
                                --continue, at your last thread there. From a terminal, it
                                starts glave in the background first when it is not
                                running (glave server --help)
  glave [options] <prompt...>   run one turn with this prompt, answered in the terminal
  glave --goal <objective> [--goal-budget <tokens>] [options]
                                keep working across turns until the model has checked the
                                goal is met, or pauses it because it needs you. Say how to
                                check in the objective (\"... until cargo test passes\").
                                Stops when its token budget (default 1000000) runs out,
                                after three turns in a row without a tool call, or on Ctrl-C
  glave --background <prompt> [--budget <tokens>], or -b
                                run the turn as a task, with no one attached, in glave
                                running for you in the background, so it goes on when you
                                leave; with --goal, work toward the goal (glave tasks --help)
  glave tasks [stop <thread>]   list the tasks running and recently finished, or stop one
  glave schedule add|list|rm    run a prompt or a goal as a task at set times
                                (glave schedule --help)
  glave serve --stdio [options] serve the session API (JSON-RPC 2.0, one message per line)
                                to the program that started glave
  glave serve [--http <address> [--issuer <url> --audience <name>]] [options]
                                run glave for you in the foreground, as a service or a
                                container does: one per GLAVE_HOME (a second one exits),
                                on the Unix socket GLAVE_HOME/run/server.sock and over
                                HTTP, on 127.0.0.1:4180 and [::1]:4180 (another port free
                                at both when that is taken) unless --http says where. A
                                browser on this computer signs in by a link glave prints or
                                opens, good once for five minutes, and stays signed in;
                                programs here use the token in GLAVE_HOME/serve.token, made
                                at every start. Reached from elsewhere, at origin or domain
                                under [serve] in config.toml, a browser there signs in with
                                a passkey: the first made through a link glave server link
                                prints here. With --issuer and --audience, clients
                                elsewhere sign in with an OIDC access token from that
                                issuer for that audience. The address is host:port, or a
                                port on 127.0.0.1. The listener has no TLS: beyond this
                                computer, a tunnel or a proxy that serves HTTPS goes in
                                front, and origin under [serve] in config.toml
                                says where people reach it (https://glave.example.com); a
                                tunnel on this computer reaches 127.0.0.1 at that name too.
                                With domain under [serve] instead, glave serves HTTPS for
                                that name itself, on port 443 unless --https <host:port>
                                says where, its certificate from Let's Encrypt (or the ACME
                                directory acme names), renewed on its own; until it has
                                one, it answers there only the authority's check.
                                --issuer may be given more than once. The web app is
                                served at / too: from --web <folder> (the app's build),
                                else the one built into glave, else the web folder beside
                                the glave binary, if any. Anyone who reaches the listener
                                gets a folder's files without signing in, so it must hold
                                only the build and must not be writable by an agent
  glave server status|stop|restart|link
                                whether glave runs for you in the background, stop it,
                                restart it from this terminal, or print a link that makes
                                a passkey on a device elsewhere (glave server --help)
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
  glave connect <name> | github | <address>
                                sign in to one of your MCP servers in the browser; or connect
                                GitHub, or any git host with a token, so git and the host's own
                                tool sign in as you (glave connect --help)
  glave connections [remove <id> | <name>]
                                list your connections and git hosts, or remove one
  glave search [on | off]       whether glave may search the web for you, or turn it on or off
  glave credits [limit <dollars> | add <dollars>]
                                what is left to spend on models, where this glave keeps
                                credits, the monthly limit, or credits added by whoever runs it
                                (glave credits --help)
  glave doc <command>           read, make and edit spreadsheets, documents and decks,
                                merge, split, turn and fill in PDFs, and query data with
                                SQL, with nothing else installed (glave doc --help)

Options:
  --model <name>     the model to use: one glave runs (default: its default model)
  --continue, -c     continue your last thread in this folder
  --permissions <mode>
                     what the agent may do: auto (the default: it works on its own in
                     this project, its commands in the sandbox, and asks only before
                     something risky), ask (it asks before every change; its commands
                     change nothing) or full-access (no sandbox and no limits: it can
                     change anything on this computer and never asks). Where the sandbox
                     cannot run, every change and command asks in auto and ask alike.
                     With no one at the keyboard, anything that would ask is declined
  --worktree         work in a new worktree of this repository, on a branch glave/<id>,
                     kept under GLAVE_HOME/worktrees, rather than in this folder itself
  --events <file>    save the thread's events to this file as JSON Lines
  --server <url>     be a client of the session server at this URL (or [server] url in
                     config.toml): threads, turns and approvals run there and show here;
                     sign in with GLAVE_SERVER_TOKEN
  --log <file>       append glave's spans and metrics to this file: ids, names, counts and
                     timings, never prompts, answers, file contents or secrets
  --help             show this help
  --version          show the version

A turn, --continue and --goal run in glave running for you in the background, started from
a terminal when it is not running, and go on if the terminal closes; Ctrl-C stops the turn.
With no one at the terminal and glave not running, they run in this command alone, and
anything that would ask is declined.

How glave runs is its settings, set in config.toml or given to glave serve:
  --no-network       keep sandboxed commands off the network
  --network          let sandboxed commands use the network (the default)
  --on <place>       where threads work: here (the default), or cloud: a disposable worker
                     of their own, made from the project's devcontainer.json or
                     .glave/environment.toml, with this repository cloned in at its
                     current commit; work comes back as pushed branches
  --max-steps <n>    end each turn after n model calls, the last one a text report of
                     what is done and what is left (default: no limit)

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
  GLAVE_STORE_BUCKET an S3-compatible bucket (s3://name) to keep files in, as [store] bucket,
                     reached with the AWS_* settings (default: the blobs folder in GLAVE_HOME)
  GLAVE_LOG          how much --log records: error, warn, info (the default), debug or trace
  GLAVE_JUDGE_MODEL     where nothing else asks, the model that decides whether an action
                     a rule flagged may run because you asked for it (default: the main
                     model; `off` to always ask)

Settings: config.toml in GLAVE_HOME may set model, judge_model, permissions, network,
worktree (true or false), on (here or cloud), [workers] (backend = \"docker\" and glave,
a Linux build of glave for workers), mcp (the servers in mcp.json to turn on) and
[telemetry] (otlp = an OpenTelemetry collector's OTLP/HTTP URL, for builds with the otlp
feature: spans and metrics, never content), [store] (url = a postgres:// database, over TLS
off this computer) and [serve] (origin = the https address people reach glave at from
elsewhere, through a tunnel or a proxy in front of it; or domain = the name glave serves HTTPS
for itself); flags and the environment win over it. An
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
glave asks once, only when they would run commands, and remembers the answer. With no one
to ask, those servers stay off.";

/// What to do.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
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
    Help,
    Version,
}

/// What the person asked for.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent command-line flags, each a yes or no"
)]
pub(crate) struct Args {
    pub action: Action,
    pub model: Option<String>,
    /// What the agent may do; chosen elsewhere, or Workspace, when not given.
    pub permissions: Option<Permissions>,
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
    /// Serve HTTPS of glave's own on this address, rather than port 443 of every address.
    pub https: Option<SocketAddr>,
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
    /// Run the turn or goal as a task in the person's server, with no one attached.
    pub background: bool,
    /// With `background`, tokens the turn may use.
    pub budget: Option<u64>,
    /// Serve the program that started glave, over stdin and stdout.
    pub stdio: bool,
    /// As the person's server started in the background: a session of its own first
    /// (`crate::server::start` passes it).
    pub own_session: bool,
}

impl Args {
    /// Whether `glave serve` runs as the person's server: not a program's own over stdio.
    pub(crate) fn person_s_server(&self) -> bool {
        self.action == Action::Serve && !self.stdio
    }

    /// Whether a turn or a goal was asked for.
    pub(crate) fn turn(&self) -> bool {
        !self.prompt.is_empty() || self.goal.is_some()
    }
}

pub(crate) fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Args, String> {
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
            "--permissions" => args.permissions = Some(mode(rest.next())?),
            "--network" => args.network = Some(true),
            "--no-network" => args.network = Some(false),
            "--worktree" => args.worktree = true,
            "--on" => args.on = Some(place(rest.next())?),
            "--max-steps" => args.max_steps = Some(steps(rest.next())?),
            "--goal" => args.goal = Some(rest.next().ok_or("--goal needs an objective")?),
            "--goal-budget" => args.goal_budget = Some(budget(rest.next())?),
            "--background" | "-b" => args.background = true,
            "--budget" => args.budget = Some(tokens("--budget", rest.next())?),
            "--stdio" if serving => args.stdio = true,
            "--own-session" if serving => args.own_session = true,
            "--http" if serving => args.http = Some(address(rest.next())?),
            "--https" if serving => {
                let value = rest.next().ok_or("--https needs an address: host:port")?;
                args.https = Some(
                    value
                        .parse()
                        .map_err(|_| format!("--https {value:?} is not host:port"))?,
                );
            }
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
    args.prompt = words.join(" ");
    alone(&args)?;
    if serving && args.action == Action::Run {
        transport(&args)?;
        args.action = Action::Serve;
    }
    goal(&args)?;
    background(&args)?;
    settings_only(&args)?;
    Ok(args)
}

/// Refuses, for a turn, the flags that choose how glave runs rather than one turn: they are
/// the server's settings (`specs/one-server.md`, "The flags"), which `glave serve` and
/// `glave policy explain` still take.
fn settings_only(args: &Args) -> Result<(), String> {
    if !matches!(args.action, Action::Run | Action::Help | Action::Version) {
        return Ok(());
    }
    let given = [
        (args.network == Some(true), "--network"),
        (args.network == Some(false), "--no-network"),
        (args.on.is_some(), "--on"),
        (args.max_steps.is_some(), "--max-steps"),
    ];
    match given
        .into_iter()
        .find_map(|(given, flag)| given.then_some(flag))
    {
        Some(flag) => Err(format!(
            "{flag} chooses how glave runs, not one turn: set it in config.toml, or give it to \
             glave serve"
        )),
        None => Ok(()),
    }
}

/// The command the first words name, taking them: `policy explain`, `skills` or `commands`;
/// otherwise a run, taking nothing.
fn command(rest: &mut std::iter::Peekable<impl Iterator<Item = String>>) -> Result<Action, String> {
    let action = match rest.peek().map(String::as_str) {
        Some("policy") => Action::Explain,
        Some("skills") => Action::Skills,
        Some("commands") => Action::Commands,
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
    if stdio && (args.http.is_some() || args.https.is_some()) {
        return Err("glave serve takes one of --stdio or --http <address>".into());
    }
    if args.own_session && stdio {
        return Err("--own-session is for the person's server, not --stdio".into());
    }
    if !args.prompt.is_empty() || args.resume || args.events.is_some() {
        return Err("glave serve takes no prompt, --continue or --events: clients choose".into());
    }
    if args.web.is_some() && stdio {
        return Err("--web is for HTTP: the web app is served over HTTP".into());
    }
    let signs_in = !args.issuers.is_empty() || args.audience.is_some();
    if stdio && signs_in {
        return Err("--issuer and --audience are for --http; stdio needs no sign-in".into());
    }
    if args.http.is_some() && args.issuers.is_empty() != args.audience.is_none() {
        return Err("--issuer and --audience go together: OIDC needs both".into());
    }
    Ok(())
}

/// The value of `--http`: `host:port`, or a port alone on 127.0.0.1.
pub(crate) fn http_address(value: Option<String>) -> Result<SocketAddr, String> {
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

/// The mode `--permissions` names.
fn mode(value: Option<String>) -> Result<Permissions, String> {
    let name = value.ok_or("--permissions needs a mode: auto, plan, ask or full-access")?;
    Permissions::named(&name)
        .ok_or_else(|| format!("unknown mode {name:?}: use auto, plan, ask or full-access"))
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
mod tests;
