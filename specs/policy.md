# Config and policy

How glave is set up: what a person or project chooses, and what an organization allows. Tracks issue #22, with the project trust of issue #83.

## Status

First cut built, in the `glave-config` library (`crates/config/src/policy`), which the CLI and the servers share:

- `~/.glave/config.toml` (in `GLAVE_HOME`) and `/etc/glave/policy.toml` are read as typed TOML; unknown keys, and a policy without `version = 1`, are errors naming the file and key.
- Choices resolve nearest-wins across flags (`--model`, `--permissions`, `--network` and `--no-network`), the environment (`GLAVE_MODEL`, `GLAVE_JUDGE_MODEL`), the person's file and the defaults, each remembering its source. `mcp` turns on only the named servers of `mcp.json`. `worktree` (`true` or `false`, default `false`; also `--worktree`) gives each new thread in a project with a folder a git worktree of its own instead of the folder itself (`computers.md`); it is a plain choice, needs no trust in a project's file, and `glave policy explain` shows it with its source. `titles` (`true` or `false`, default `true`) has a model title each thread after its first exchange (`session.md`, "Titles"); off, a thread's title is the first line of its first message. `[computers]` (the server's own file only) gives each person a computer of their own on the Kubernetes cluster `[workers]` names (`glave`, the Linux build that goes into them; optionally `kubeconfig`, `namespace` and `runtime_class`), where all their threads work (`computers.md`, "A person's computer in the cloud"): `image`, pinned by digest, and `disk_gb`, each person's disk (50 unless set); without `[workers]` it is an error saying so. `on`, `--on` and `[placement]`, which put threads on per-thread workers, were removed before v0.1 with them (owner, 2026-11-08).
- Constraints only narrow: allow-lists intersect, deny lists add up, a requirement anywhere holds, limits take the smallest, and contradictions (a judge both required and off) are errors.
- A choice someone made outside the constraints is an error naming the rule, its file and the choice's source. Only glave's own defaults give way, to what the policy leaves (shown as "default, narrowed by" the file).
- Safe by default (`sandbox.md`, `approvals.md`): `permissions` defaults to `auto` and `network` to on. Where the sandbox cannot run, the mode stays and what it comes to gives way: Auto and Ask first run commands without the sandbox and every change and command asks, said once at startup and by `server/discover`. A policy that leaves no full access requires the sandbox: there glave refuses to start. `glave policy explain` checks whether the sandbox can run here and shows the result beside the mode.
- Enforced: `[models]` when the gateway's targets are built, `[permissions] highest` and `[approval] judge` in the settings the harness runs with and in what the session API offers, the sandbox it requires before the workspace exists, `[mcp] own = false` before any server starts, `[mcp] catalog` (below), `[workers]` (cpu, memory_gb, network `open` or `offline`, isolation) by the worker adapter, for people's computers: limits take the smallest, `offline` wins over `open`, and `vm` over `container`; a policy requiring `vm` refuses every computer on a cluster without a runtime class.
- `[sandbox] network = false` takes the network from commands (#641): glave's default `network` gives way to off ("default, narrowed by" the file), a choice of on is an error naming the rule, the rule leaves no full access and so requires the sandbox, and it narrows `[workers] network` to `offline`. `[sandbox] ask_network = false` forbids a command without the network to ask for it, or to run outside the sandbox, which has it ("The network").
- `[notifications] push = false` (#333) pushes people's browsers nothing: the server is wired with nothing to push with, `server/discover` says no `push`, and `push/get` says why (`session.md`, "Push notifications"). `glave policy explain` lists the rule. It is the server's: on a server shared by many organizations, one can't yet turn push off for itself alone.
- `[mcp] catalog` names the servers the organization offers; several catalogs narrow one another. The servers of the person's `mcp.json` the catalog names count as the catalog's, so `[mcp] own = false` leaves them on and refuses only the others, and Connect offers the connectors it names (below).
- `[provider]` (a choice: `name = "azure"` with `resource` and optionally `scope`; `name = "bedrock"` with `region` and optionally `endpoint` and `api`; or `name = "vertex"` with `project` and optionally `location` and `api`) takes a cloud contract's models in place of `GLAVE_BASE_URL`'s server (`gateway.md`, "Cloud contracts"); `[models]` rules then see `azure/<deployment>`, `bedrock/<model>` or `vertex/<model>`. At Bedrock and Vertex, Claude goes by Anthropic's Messages API and other models by Chat Completions, unless `api = "anthropic"` or `"openai"` sends every model one way. `glave policy explain` shows the provider, where it is and which API. Only the person's or the server's own `config.toml` sets it: a project's is ignored, since the organization's credentials go there. Its keys come only from the environment.
- `[github]` (a choice: `client_id`; optionally `app`, the app's name in its URL, for the page where people choose its repositories; and optionally `url` for a GitHub Enterprise Server, its origin: `https://` and its host, or `http://` only to this computer) names the GitHub App people sign in to GitHub through in the browser (`session.md`, "Git hosts"). The app needs device flow turned on; its user tokens may expire, as GitHub's default has them, since glave renews them. The client id is public, never a secret. Only the person's or the server's own `config.toml` sets it: a project's is ignored, since the person's GitHub sign-in goes there. Without it, people sign in to github.com through glave's own app, `glave-ai` (`session.md`, "Git hosts"); a GitHub Enterprise Server, like any host, connects with a token. Policy cannot yet limit which hosts may be connected: that comes with an organization's own source of credentials (`computers.md`, rule 8).
- **Model accounts** (`session.md`, "Model accounts") are the consumer's default: a key the person connects from a client, sealed in the store, needs no file and no variable. Whatever the flags, the environment or `config.toml` choose still comes first: their model, at their endpoint, is the default, and an account's models are choices beside it. On a server people sign in to with `--issuer`, accounts are not used and cannot be connected: the organization sets up the models. An account's models are checked against `[models]` as the default is; one the policy refuses is left out rather than refusing to start.
- `glave policy explain` prints every setting with its source, every rule with its file, and why glave would refuse to run; it exits non-zero when it would.
- A server that runs for days reads the person's `config.toml`, the policy and `mcp.json` again at the start of a turn when their content's hash changed, and every choice a turn runs with follows from the next turn, the judge with the models it names; a running turn keeps what it started with. Tools a change adds wait for a new thread or a compaction, and the model is told so, since a thread's prompt and tools never change mid-thread (`invariants.md`, 27). It cannot refuse to start, so a choice a constraint added meanwhile refuses gives way as glave's default would, and the log names the rule: constraints still only narrow, whatever the person's or a project's file says (`one-server.md`, "Its settings").

Project trust (issue #83), in `crates/config/src/trust`:

- A repository's `.mcp.json` servers and the loosening parts of its `.glave/config.toml` (`permissions = "full-access"`, a `judge_model` other than `"off"`) run only after the person trusts the project. glave asks once, in the terminal or in an app (issue #250, below), and only when the repository brings such things; a repository without them never asks, and the rest of its `config.toml` (a model, a narrower mode) applies at once.
- The answer is kept in the store, in the person's trust ledger (below). What is trusted is each server's definition, so a repository that changes a server, or adds one, is asked about again, and only that.
- `[projects] trusted` pre-trusts projects by path glob, with no question. `[mcp] own = false` forbids a project's servers whatever anyone says; the person's own servers in `mcp.json` stay refused loudly as before.
- With no one to ask (a pipe, a script), untrusted servers stay off, glave says so, and the model is told which tools are off and why. A session server (`glave serve`, which the terminal's commands are clients of) asks the person on this computer in an app, or in the terminal through it, instead (`session.md`, "Project trust"); until they answer, the same holds, and the model is told the person has not trusted the project yet.
- In a session server, every project's trusted servers and hooks run for the threads that work in it, and only for them, the folder the server started in as much as a project made in the browser: no project lends its servers, hooks or settings to the whole server, whose own are the person's. A hook runs the project's own files, so it never runs in another project's folder. What is trusted is read again at the start of each turn, so a yes is on from the next one. A project's servers start in its folder the first time one of its turns needs them, and stay up for its later turns until 30 minutes pass with no turn of the project using them, for 16 projects at once (the one used least recently is closed first); a closed one starts again when it is next needed. The person's own servers start once, in their home folder, for their threads everywhere and never a thread of anyone signed in from elsewhere. A project's server whose name one of the person's own servers has stays off: the person's wins, and the model and the person are told.
- A project's mode applies to its threads that chose none under the rule the terminal settles it by: one no wider than what they would run with otherwise (the server's own, the person's file) applies at once, a wider Auto is ignored, so a repository never widens what the person chose, and full access applies once trusted. A flag is nearer than the project, and the policy's cap (`[permissions] highest`, or the network rule) holds. A thread's own choice is nearer still. Trust is the person's decision about their own computer, so what a project turns on runs only for the threads of the person on this computer: a thread of anyone signed in from elsewhere gets none of it, trusted or not (no servers, no hooks, no settings), and its model is told they are off.
- Every other choice in a project's `config.toml` is its threads' alone too, read at the start of each turn as its trust is: its model, `network = false`, a narrower mode, `worktree`, `titles` and `project_memory` at once; its `judge_model` and a wider `[web]` once trusted (without trust its `[web]` keeps only `fetch` and `deny`). The nearest choice wins: the thread's own, a flag or variable the server started with, the project's, the person's file, glave's defaults. A project's model, and its judge's, must be ones the server offers: another is set aside with a sentence, and its threads use the server's. Settings the constraints refuse are set aside whole, with why. A project never sets what only the person's or the server's own file sets (store, provider, GitHub, telemetry, server, serve, computers), nor turns on the person's own servers (`mcp` picks among its own). Not yet per project: `[workers]`, which stays the server's.
- `glave policy explain` lists the project's items and where each stands.

Web and untrusted servers (issue #94; `approvals.md`, "Untrusted content"):

- `[web]` in `config.toml` (`fetch`, `allow`, `deny`, `private_addresses`) is one choice, the nearest table winning; patterns are checked when read, and read as the outbound proxy reads its hosts allowed (`sandbox.md`, "The outbound proxy"): `docs.rs` at any port, `localhost:3000` at that port alone, `*.example.com` never `example.com` itself; a deny naming a port denies that port alone. A host `allow` names exactly may be private, and is then fetched only at the ports its exact entries name, whatever `*` allows: a lookup sees no port, so `*` beside `localhost:3000` must not reach `localhost:6379`. Without trust a project's `[web]` keeps only `fetch` and `deny`. `glave policy explain` shows it.
- What an untrusted MCP server returns is labelled untrusted. A project's servers and remote (HTTP) servers are untrusted by default; the person's own stdio servers are trusted. The person's `mcp.json` may mark a server of theirs either way (`"untrusted": true` or `false`); a project's `.mcp.json` may only add `"untrusted": true`. `[mcp] untrusted` in a policy names servers that are untrusted whatever else says; `[mcp] trusted` vouches for remote and project servers left unmarked, and, like `[projects] trusted`, any file may add to it, but it never lifts an `untrusted` mark (`approvals.md`, "Untrusted content").
- A remote MCP server is reached under the same list of addresses as `web_fetch` (`crates/ports/src/host`), checked at each connection after its name is looked up, in sign-in's discovery too, and no redirect is followed off its origin (#1132). Cloud metadata is refused everywhere. Private addresses are refused where people sign in from elsewhere (a public origin, or an identity provider) for what a person's or a project's file names; on a person's own computer they work as before, and an operator's own file keeps them, since no person or repository writes it: `mcp.json` on a server with an identity provider, whose servers run only for the person on that computer. A setting naming ranges a company may reach waits for a server that needs it.
- Signing in to a remote server goes where its metadata says (its authorization server, where glave registers, where it trades a code or renews a token), so where private addresses are reached a sign-in narrows them by the server (#1330): one whose own address, as its URL writes it, is this computer or a private address signs in under the same rule as its calls; any other, a server on the internet, reaches a private address only at its own host, which its calls reach anyway, so its metadata never sends glave into the person's network, at every request and redirect, after each lookup. rmcp's own client decides alike, from the URL as written, since what the server's own name looks up to is the server's to say. Such a sign-in takes no proxy, as `web_fetch` takes none, since a proxy looks names up where glave can't see. Not yet, each waiting for someone who needs it: a server on the internet whose authorization server is inside a company's network (an internal sign-in reached through split DNS), which rmcp allows with `new_allowing_private_addresses`; a server inside the network by name whose authorization server is another host inside; signing in through a proxy people must use.
- Not built yet: `[web]` rules in `policy.toml` (an organization's allow and deny lists), and fetches leaving through the gateway.

Hooks (issue #46; `approvals.md`, "Hooks"):

- `[[hooks.<Point>]]` in `config.toml` (`PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `Stop`; `command`, optional `matcher` and `timeout`), checked when read: an unknown point or key, a matcher that is not a regular expression, or a matcher on a point without a tool is an error naming the file.
- Hooks add up instead of the nearest winning: the person's, then a trusted project's. They are not choices, so flags and the environment have none. The person's own run for their threads everywhere, in every project and in Chats, and never for a thread of anyone signed in from elsewhere: they run with that person's sign-ins.
- A project's hooks are trust items, each bound by its whole definition. `[hooks] own = false` forbids them, and makes glave refuse to start while the person's own `config.toml` has hooks. `glave policy explain` lists the person's hooks and the project's with where each stands.

Memory (issue #93; `memory.md`):

- `memory = "on"` or `"off"` in `config.toml` (default `on`, `memory.md`) is the person's own choice: a project's file cannot set it. `project_memory` (`shared`, the default, `isolated` or `off`) is the project's setting, which a project's file may choose without trust, since it can only narrow.
- `[memory]` in a policy (`enabled`, `personal`, `projects`, `retention_days`) only narrows: `enabled = false` turns memory off (an explicit `memory = "on"` is then an error naming the rule), `personal = false` leaves projects only `isolated` or `off` (glave's default `shared` gives way to `isolated`), `projects` lists the settings allowed, and the smallest `retention_days` holds. `glave policy explain` shows `memory` and `project_memory` with their sources.

Not built yet: a command to change a decision; signed bundles from an organization's server, macOS managed preferences and Group Policy; a project's own policy; recording policy changes as events (the admin console); and recording the policy version in each turn's configuration.

## Two kinds of setting, two rules

- **Choices** say what is used: the model, the judge model, the mode (what the agent may do), the network, which MCP servers are on. **The nearest level that sets a choice wins.**
- **Constraints** say what is allowed: which models and MCP servers, the highest mode, budgets. **Constraints only narrow on the way down, and a deny always wins.**
- A choice outside the constraints is an error that names the rule and where it came from. It is never silently replaced.

A "requirement" (the sandbox must be on) is a constraint that allows one value. There is nothing else: no per-field merge strategies, no priorities.

## Where settings come from

**Choices**, nearest first:

1. command-line flags;
2. environment variables (`GLAVE_MODEL`, `GLAVE_JUDGE_MODEL`, …);
3. the project's `.glave/config.toml` (once the project is trusted);
4. the person's `~/.glave/config.toml`;
5. glave's built-in defaults.

**Constraints**, all of which apply at once:

1. the organization's policy, kept as data in its scope and edited from its admin screens (`model.md`, "Configuration is files"); and, beside it, a file the server's operator or device management installs at `/etc/glave/policy.toml` (macOS: managed preferences; Windows: Group Policy), which narrows like any other;
2. a project's own policy, which may only narrow the organization's (a client's sealed project, say);
3. none at all for a person on their own computer, whose personal organization allows everything.

A consumer never sees a policy. They set at most a few choices.

## The files

`config.toml` (choices; every key optional):

```toml
model = "anthropic/claude-sonnet-5-5"
judge_model = "anthropic/claude-haiku-5-5"   # or "off"
permissions = "ask"                           # or "auto", the default; "plan"; or "full-access"
network = false                               # sandboxed commands' network; default true
mcp = ["github", "jira"]                      # servers to turn on, from the catalog
worktree = true                               # each thread in a worktree of its own; default false
titles = false                                # no model-written titles; default true

[[models]]                                    # models a person may choose for a thread besides
id = "hope-12b"                               # `model` (session.md, "Choosing a model…"), nearest list wins;
name = "Hope"                                 # each is checked against [models] like `model`

[web]                                         # what web_fetch may reach (approvals.md), nearest wins
fetch = true                                  # default true; false turns web_fetch off
allow = ["docs.rs", "*.example.com"]          # hosts it may fetch; every host when absent
deny = ["*.internal.example.com"]             # hosts it never fetches; a deny wins
private_addresses = false                     # default false; a host allow names exactly may be private anyway; cloud metadata never

[server]                                      # the person's file only: a session server elsewhere
url = "https://glave.example.com"             # glave runs threads there (glave --server); sign in with GLAVE_SERVER_TOKEN

[serve]                                       # the person's or server's file only: how people reach this glave
origin = "https://glave.example.com"          # its public origin, served over HTTPS by a tunnel or proxy in front;
                                              # without it glave answers on this computer alone
# domain = "glave.example.com"                # or a name glave serves HTTPS for itself (not with origin), on
                                              # port 443 unless glave serve --https says where
# acme = "https://acme-staging-v02.api.letsencrypt.org/directory"  # where its certificate comes
                                              # from, with domain; Let's Encrypt when absent
# forwarded = 2                               # how many addresses the proxies in front add to
                                              # X-Forwarded-For, the client's first: 2 behind
                                              # Google Cloud's load balancer, glave reached only
                                              # through them; unset, only the last a proxy here or
                                              # on a private network names

[store]                                       # the person's or server's file only; also GLAVE_STORE_URL, GLAVE_STORE_BUCKET
url = "postgres://glave:...@db.example.com/glave"  # the one data store; the local SQLite file when absent.
                                              # TLS off this computer unless sslmode=disable; shown without its password
bucket = "s3://glave-blobs"                   # where files' bytes go; the blobs folder in glave's folder when absent.
                                              # Reached with the AWS_* settings or workload identity, never keys here

[provider]                                    # the person's or server's file only (gateway.md, "Cloud contracts")
name = "azure"                                # a cloud contract's models instead of GLAVE_BASE_URL; model = a deployment
resource = "acme-openai"                      # or its endpoint, https://acme.services.ai.azure.com
# scope = "https://ai.azure.com/.default"     # the Entra scope, when not this default
# name = "bedrock"                            # or Amazon Bedrock; model = a Bedrock model id
# region = "us-east-1"
# endpoint = "mantle"                         # default "runtime"; "mantle" for models only it serves
# name = "vertex"                            # or Google Vertex AI; model = google/gemini-3.5-flash, claude-opus-5-5
# project = "acme-ai"
# location = "global"                         # the default; or us, eu, or a region such as us-east5
# api = "anthropic"                           # bedrock and vertex: every model by this API; by model family when absent
                                              # keys from the environment, else the workload's identity
                                              # (vertex: Google Application Default Credentials)

[telemetry]                                   # the person's or server's file only; off by default
otlp = "http://localhost:4318"                # spans and metrics (never content) to this OTLP/HTTP collector

[github]                                      # the person's or server's file only (session.md, "Git hosts")
client_id = "Iv23li0123456789abcd"            # the GitHub App sign-ins in the browser go through; public
app = "glave"                                 # its name in https://github.com/apps/glave, to choose repositories
# url = "https://github.example.com"          # a GitHub Enterprise Server; github.com when absent

[[hooks.PreToolUse]]                          # commands run at points of a turn (approvals.md)
matcher = "bash"
command = "./scripts/guard.sh"

[workers]                                     # the cluster people's computers run on (computers.md)
glave = "/opt/glave/linux-arm64/glave"         # a Linux build of glave for the cluster's processor
# kubeconfig = "/etc/glave/kubeconfig"        # default KUBECONFIG, ~/.kube/config, or in-cluster
# namespace = "glave-workers"                 # default the kubeconfig's
# runtime_class = "gvisor"                    # gVisor's runsc or Kata: a kernel or VM for each
```

Hooks are the one thing in `config.toml` that adds up across files rather than the nearest winning: the person's and a trusted project's all run.

`policy.toml` (constraints; every section optional):

```toml
version = 1

[models]
allow = ["acme-vllm/*", "bedrock/anthropic.*"]   # globs over provider/model
deny = []

[permissions]
highest = "auto"                                  # the most anyone may choose; below full access,
                                                  # the sandbox is required

[approval]
judge = "required"                                # or "allowed", "off"

[sandbox]
network = false                                   # commands go without the network ("The network")
ask_network = false                               # nor may one ask for it; asking is allowed when absent

[mcp]
catalog = ["github", "jira"]                      # the organization's approved connections
own = false                                       # may people add their own connections, local or remote
untrusted = ["search"]                            # servers whose output is untrusted (approvals.md)
trusted = ["docs"]                                # remote or project servers vouched for


[projects]
trusted = ["/srv/repos/*"]                        # projects at these paths (globs) need no question

[hooks]
own = false                                       # may people and projects run hooks

[workers]                                         # what people's computers may be (computers.md)
cpu = 4                                           # the most CPUs; the size when none is named
memory_gb = 8
network = "open"                                  # or "offline"; "guarded" is refused when read until it holds there
isolation = "vm"                                  # or "container": "vm" needs a runtime class

[tasks]                                           # threads that run with no one attached (session.md)
max_running = 3                                   # tasks one person may have running at once
max_schedules = 10                                # schedules one person may keep

[notifications]
push = false                                      # people's browsers are pushed nothing (session.md, "Push notifications")
```

Unknown keys are errors that name the key and file, so a typo never silently does nothing.

An organization's policy is data in its scope, in the format above, and its admin screens edit it: each change is an event with its author, so every change has a history. A file at `/etc/glave/policy.toml` stays for what has no organization's screens to edit it: a laptop under device management, and a server's operator narrowing every organization on it.

## Enforcement

Each constraint is enforced where it cannot be bypassed:

| Constraint | Enforced by |
| --- | --- |
| models, credits | the gateway in glave, which holds the keys |
| the highest mode, the judge | the session API, which offers and accepts only modes within it, and the harness's tool gate |
| sandbox, and its network | the computer port, before any command runs |
| asking for the network | the harness's tool gate, before anyone is asked |
| MCP servers | glave, when it starts servers |
| hooks | the CLI, before the harness is built: forbidden hooks are never handed to it |
| people's computers (`[workers]`) | the worker adapter (`WorkerComputers`), before a computer wakes |
| push notifications | the CLI, before the server is built: a server under `[notifications] push = false` is wired with nothing to push with, and says so |

A laptop's own config can never loosen a constraint, because the constraint arrives from the organization's server or device management, not from a file the person edits.

## Explaining

`glave policy explain` prints every effective setting, which file or flag set it, and every constraint with its source. Every denial names the rule that denied it and where that rule came from. Each turn records the policy bundle version it ran under, in its configuration blob.

## Connections

A connection (`model.md`, "The hierarchy") reaches its service through a tool server started on the person's computer or run remotely. Decided on 2026-10-23 (#633); what is built is `[mcp]` ("Status"), and signing a person in to a remote server of their own `mcp.json` (`session.md`, "Connections").

- **One list per organization: its approved connections,** kept in its policy as `[mcp] catalog`. Connect offers them to its people, and each signs in once as themselves.
- **Connections are called from the server,** the person's sign-in sealed in the data store, so using one never wakes a computer and no token is on one. Where glave runs the server, a network rule lets it reach the internet and nothing of ours, as a person's computer's does (`computers.md`); a company's own servers name the internal addresses theirs may reach, and a hosted company lets our outbound address in (#1132).
- **Services that hold documents** (Google Drive, SharePoint, Confluence) are connections like any other. glave's own knowledge bases are a separate thing, planned in #661: curated documents searched by meaning, each with an owner, granted to groups, and private to whoever made it by default.
- **Its default follows who the organization is, never the deployment.** A company's is "only from our list": it runs no other tool server, whatever a file says (`[mcp] own = false`). An organization of one's is "anyone can add".
- **"May add their own" is one grant,** given to groups (#645). It covers servers started on the person's computer and remote ones alike: both reach data with that person's sign-ins, so one answer fits both.
- **Connect**, the page that lists connections, connects one of glave's connectors (`session.md`, "Connectors") with a record in the person's accounts ledger and their sign-in, and writes no file; `[mcp] catalog` names the connectors an organization offers, and with `[mcp] own = false` and no catalog none is. An approved connection needs only the sign-in.
- **Each is used everywhere, or only in one place** (owner, 2026-11-06; #1219): "Everywhere, including chats", or "Only in" one project of the person's or their Chats, which is their personal project, so one mechanism serves both. Added from Settings or a chat, it starts on everywhere; from inside a project, on only that project (the client's choice of default, the server's of what holds). Built for the connectors of glave's catalog (`session.md`, "Connectors"); a server the person's `mcp.json` names is used everywhere, as a file has no place.
- **The server keeps each to its place** (`invariants.md`, 29): a thread is offered the tools of only the connections used where it works, so one used elsewhere is not listed, not found by a search of tools, and refused if called, in a plain sentence the model passes on. A subagent has no more than its parent, whose project it works in.
- **Changes apply at once.** A connection turned on, or moved into a thread's place, comes with the thread's next turn, which makes the thread's prefix again, once (`invariants.md`, 27); one turned off, or moved out, is refused from the thread's next turn on, its tools still listed so the prefix stays as it was, and a call to it is told it is not connected here, for the model to pass on. A turn under way keeps the tools it started with, each call going to the server it listed.
- **Two accounts of one service** (#1219): a second account of a service the person has takes a name, unique to them for that service ("Glave", "Acme"; the first stays unnamed until they name it), which left empty is glave's (`Stripe 2`), shown as who it is signed in as where the service says (#680), is signed in to apart, and is used in a place no other account of that service is: "another Stripe account is used everywhere already" otherwise. A place's own account replaces the everywhere one of its service there, so a project with "Stripe: Acme" sees only Acme's, and a thread never has two of one service; a call's line, and so its approval, names the account (`mcp__stripe__refund (Acme) {…}`). Both in one place waits until someone asks.
- **Everyone signs in as themselves.** Shared keys, where an organization needs one, live only in its gateway, never on a person's computer.
- **A tool server from a repository's file asks "run this?"** before its first run, and again when its definition changes, for each person ("Project trust", as built). An approved connection is never asked about.
- **Approved is not vouched for.** Whether what a connection returns is labelled untrusted follows `approvals.md` ("Untrusted content"), with `[mcp] untrusted` and `[mcp] trusted`, whether it is approved or not; a tool that reaches the open world is labelled anyway.
- **An organization's say is unchanged:** its approved list and "may add their own" decide which connections exist for its people; where each is used is the person's. No connection is forced on anyone.
- **Later, for organizations:** offboarding revokes every sign-in of the person who leaves at once, with a test that proves it; work a team shares (a team's project, or its connections) is its own feature, after this.
- **Safety is the industry's baseline today:** the approved connections, each person's own sign-in, approvals (`approvals.md`), the sandbox around every command (`sandbox.md`), untrusted content labelled, and every tool call recorded (`events.md`). A stronger rule waits for its own decision and measurement: checking a write through a connection after untrusted content is #634.

## Permissions

Decided on 2026-10-23 and built (#649): what the agent may do is one choice, its mode (`approvals.md`, "Modes"; named Auto, Ask first and Full access with #739, and Plan with #92), not separate approval and sandbox switches.

- **One choice, the nearest wins:** `permissions` in a `config.toml`, `--permissions`, a thread's own (`session.md`, "Choosing a model and permissions"). A project's file only narrows it, or gives full access once trusted ("Project trust").
- **The policy caps the highest:** `[permissions] highest = "ask"` or `"auto"` (`"full-access"` caps nothing), the modes ordered by caution, Plan lowest, then Ask first; every cap leaves Plan, which changes nothing, and `"plan"` leaves only it, where no plan can be built (`approvals.md`, "Plan mode"). Several caps: the lowest holds. glave's default gives way to the cap ("default, narrowed by" the file); a person's wider choice is an error naming the rule, and on a server already running it gives way, said in a sentence. What a thread may choose (`server/discover`'s `modes`) is what the cap leaves.
- **Below full access, the sandbox is required:** without it commands would run unconfined somewhere, so where it cannot run glave refuses to start. The network rule (`[sandbox] network = false`) caps at Auto for the same reason.
- **Today's constraints, re-expressed:** `[approval] modes = ["ask"]` is `highest = "ask"`; `[sandbox] required = true` is `highest = "auto"`. Both old keys are gone (an unknown key is an error naming it); forbidding the cautious modes has no rule, and nobody needs it.

## The network

Decided on 2026-10-23 and built (#641). Web search, the tool beside `web_fetch` the issue named, is a connection under the hood, which the person turns on as a setting (#677, `session.md`, "Web search").

- **On by default for a person on their own** (a laptop, their own server, Glave Cloud's personal plan): glave's own default, an organization of one's. **Off by default in an organization:** its policy file says `[sandbox] network = false`, a constraint that narrows the `network` choice as every constraint does. glave's default gives way to off ("default, narrowed by" the file); a person's `network = true` or `--network` is an error naming the rule, and on a server already running it gives way, said in a sentence. Glave Cloud's enterprise plan writes it into the policy each organization starts with (there is no template file yet; the admin console, #222, edits the policy).
- **The switch always exists:** `network` (`network = false`, `--no-network`) in every deployment. A person on their own turns it off; an organization's policy keeps it off; a project's file can only take it away.
- **Only the sandbox keeps the network from commands on the person's computer,** so the rule leaves no full access and requires the sandbox: where it cannot run, glave refuses to start, and full access, a person's or a trusted project's, is refused (a project's is narrowed to Auto).
- **A command that needs the internet asks** (`npm install`, a build fetching dependencies, `git push`). The model runs it again with `network: true` and a reason, and the person is asked to allow that one command with the network, in the sandbox otherwise: "It can reach the internet, which commands here otherwise cannot." It is the request to run outside the sandbox narrowed to the network, and asks as that one does (`approvals.md`, "Running outside the sandbox"): in every mode, each time, and with no one there it never runs. Where commands have the network, nothing asks, and the model is not offered it. A command that needs more than the network (a build writing the person's own package cache) still asks to run outside the sandbox.
- **Untrusted content changes nothing here:** a thread that has read some keeps the network its setting gives (`approvals.md`, "Untrusted content").
- **The policy says whether asking is allowed;** it is unless `[sandbox] ask_network = false`. Then no command without the network asks for it, nor to run outside the sandbox, which has it: the model is offered neither and told so, a call that asks anyway is refused, and the person is never asked. An answer never widens what the policy allows: a command whose question was answered before the sandbox changed is refused, not run, if it would now leave the sandbox otherwise than the person was asked.
- **People's computers go without it too:** the rule narrows `[workers] network` to `offline`, which a person's computer is woken with. Like every `[workers]` rule, it reaches a computer from its next wake.
- **glave's own web tools stay,** as the decision has it: `web_fetch` reads the web for the model, labelled untrusted and limited by `[web]`. A URL it fetches can still carry what the model read; `[web]` rules in a policy, to narrow its hosts, are not built yet. Web search is a connection, not a tool of glave's (#677): a setting of the person's, off until they turn it on, its results labelled untrusted; an organization's is one of its approved connections.
- **The list governs connections; the network governs the machine.** What a command reaches is the network's to say, whatever the list says: the outbound proxy allows, blocks or holds the machine's traffic (#643), and beyond glave an organization restricts its machines with its firewall, or network policy in the Helm chart (#220).
- **Later:** an allow-list of package registries and git hosts, through that proxy, so the common installs need no ask.

## Project trust

- A project's `.glave/config.toml` and its `.mcp.json` can run commands (MCP servers and hooks). So they are honoured only after the person trusts the project once. glave asks at most once per project, and only when the project brings something that runs commands.
- Trust decisions are kept in the store, in the trust ledger in the person's personal project (`events.md`, "The trust ledger"), one `trust.decided` per decision about a project, keyed by its canonical root path; the latest about a project holds, so every server on one store reads the same. A clone that moved is recognised by its `origin` remote, read from the repository's config file without running anything. Each item records the exact definition that was answered, so trust is of the content:

  ```json
  {"path": "/Users/ada/code/app", "remote": "git@example.com:acme/app.git",
   "items": {"mcp:docs": {"definition": "{\"args\":[\"--fast\"],\"command\":\"docs-mcp\"}",
                          "trusted": true}}}
  ```

  `trusted: false` is declined, and not asked again until it changes.
- A project's `network = false` applies at once; its `network = true` is ignored, so a project can take the network from sandboxed commands but never give it back.
- A project's file sends nothing from the environment to a remote server (#1132). `${VAR}` in a local program's `command`, `args` and `env` is filled from glave's environment once the project is trusted, as the person's own `mcp.json` is; in a remote server's `url` and `headers` it reads empty, or as its default, since the repository chose the address. Where people sign in from another computer (a public origin other than this computer's `localhost`, or an identity provider) a project's file reads none of the server's environment at all: a local program's `${VAR}` reads as a remote server's does, since that environment is what the server runs on, its store and mail passwords among them. This is decided apart from the addresses a server may reach, so a setting for those changes nothing here. A server that names a variable with no default where it can't be read stays off, and says which server, never which variable, and why: to the model, in what it is told stays off, and so to the person through it, and on the terminal when glave starts in that folder. The trust question does not say so yet. The person's own `mcp.json` fills both, from the environment glave started with, which on an installed server holds only what was given to glave, so there the value is written in. This keeps a file from naming a secret, not a program from finding one: a program the person trusted runs outside the sandbox as glave's own user (`invariants.md`, 30). Claude Code reads named credentials as empty there too.
- What needs trust: each server in the project's `.mcp.json`, each hook in its `config.toml` (item `hook:<Point>:<n>`, its definition the hook's point, command, matcher and timeout as JSON), and `permissions = "full-access"` in its `config.toml`, `[web] private_addresses = true` or a `[web] allow` list (a host it names exactly may be private), and a `judge_model` other than `"off"` (a repository must not choose who judges its own actions; `off` only makes glave ask more and applies at once). Nothing else asks. Servers in `~/.glave/mcp.json` are the person's own and always trusted to start; on a name clash the person's wins. Trusting a project lets its servers start; what they return is still labelled untrusted unless vouched for (above).
- An organization's policy can pre-trust projects (`[projects] trusted`), or forbid project MCP servers (`[mcp] own = false`) or hooks (`[hooks] own = false`) entirely. A forbid wins over a pre-trust and over the person's yes.
- With no one to answer, an untrusted project's servers stay off, and the model is told why: its instructions gain a "Project tools that are off" section listing what is off and why. Hooks that stay off are told to the person only: the model never calls a hook.
- **On a person's own computer in the cloud** (`computers.md`), they are asked the same, from any client, since what a project runs there runs on their machine and no one else's. What's asked is read from the project's `config.toml` as last read there and kept, so asking and answering never wake the computer, and a change is asked about from the next read. Their yes is kept in that project's own trust ledger: it goes with the project, and another project at the same path, whose paths come from its name, is asked about again. A rule of the policy's about a path doesn't trust it, since that computer's paths are every person's, though a forbid holds. Hooks are asked about and, once trusted, run on that computer through the computer port, in its sandbox where it has one. Under `[computers] sandbox = false` commands aren't kept from `.glave/` (`sandbox.md`), so a command could write what the next ask shows: it still asks, but the words may be the command's. A `[web]` in its file isn't asked about and stays off: `web_fetch` runs on the server, whose network a person's project may not widen. Tools aren't asked about yet: its `.mcp.json` isn't read there.
- In an app the person answers "Trust this project" or "Not now" (`session.md`, "Project trust"). A trust records exactly what a yes in the terminal records, per item and definition, so the terminal never asks about it again and a changed or added item is asked about again, and only it. "Not now" records nothing: like a terminal no one answered, it is asked again next time, on a new thread. Nothing is ever trusted without one of these answers from the person asked: there is no default yes, and an answer names exactly what the person was shown, so a project that changed in between trusts nothing and is asked again.
