# Config and policy

How glave is set up: what a person or project chooses, and what an organization allows. Tracks issue #22, with the project trust of issue #83.

## Status

First cut built, in the `glave-config` library (`crates/config/src/policy`), which the CLI and the servers share:

- `~/.glave/config.toml` (in `GLAVE_HOME`) and `/etc/glave/policy.toml` are read as typed TOML; unknown keys, and a policy without `version = 1`, are errors naming the file and key.
- Choices resolve nearest-wins across flags (`--model`, `--approval`, `--sandbox`, which takes `off`, `--network` and `--no-network`), the environment (`GLAVE_MODEL`, `GLAVE_JUDGE_MODEL`), the person's file and the defaults, each remembering its source. `mcp` turns on only the named servers of `mcp.json`. `worktree` (`true` or `false`, default `false`; also `--worktree`) gives each new thread in a project with a folder a git worktree of its own instead of the folder itself (`computers.md`); it is a plain choice, needs no trust in a project's file, and `glave policy explain` shows it with its source. `titles` (`true` or `false`, default `true`) has a model title each thread after its first exchange (`session.md`, "Titles"); off, a thread's title is the first line of its first message. `on` (`here` or `cloud`, default `here`; also `--on`) puts each new thread on a worker of its own, made on the backend `[workers]` names (`backend = "docker"` or `"kubernetes"`, `glave`, the Linux build that goes into workers; for Docker optionally `socket`; for Kubernetes optionally `kubeconfig`, `namespace` and `runtime_class`; a key the backend does not use is an error); `--on cloud` without `[workers]` is an error saying so.
- Constraints only narrow: allow-lists intersect, deny lists add up, a requirement anywhere holds, limits take the smallest, and contradictions (no approval mode left, a judge both required and off) are errors.
- A choice someone made outside the constraints is an error naming the rule, its file and the choice's source. Only glave's own defaults give way, to what the policy leaves (shown as "default, narrowed by" the file).
- Safe by default (`sandbox.md`, `approvals.md`): `sandbox` defaults to `project` and `network` to on. Where glave's default sandbox cannot run, it gives way: `sandbox` becomes off ("default, but it cannot run here: …") and approval `ask` ("default when commands run without a sandbox"), said once at startup; with `sandbox = "off"` chosen, approval is `ask` unless `approval = "auto"` was chosen too. A sandbox someone chose, or one `[sandbox] required` keeps (shown as "default, narrowed by" the policy), never gives way: glave refuses to start. `glave policy explain` checks whether the sandbox can run here and shows the result.
- Enforced: `[models]` when the gateway's targets are built (and by a gateway serving many, on every call, for each caller), `[approval] modes` and `judge` in the settings the harness runs with, `[sandbox] required` before the workspace exists, `[mcp] own = false` before any server starts, `[mcp] catalog` (below), `[budget] person_per_day_usd` by a gateway serving many, for each person each UTC day (`gateway.md`), `[workers]` (cpu, memory_gb, max_minutes, per_person, network on or off, isolation) by the worker adapter: limits take the smallest, `off` wins over `on`, and `vm` over `container`; a policy requiring `vm` refuses every worker on a backend without a runtime class.
- `[sandbox] network = false` takes the network from commands (#641): glave's default `network` gives way to off ("default, narrowed by" the file), a choice of on is an error naming the rule, the rule requires the sandbox as `required` does, and it narrows `[workers] network` to off. `[sandbox] ask_network = false` forbids a command without the network to ask for it, or to run outside the sandbox, which has it ("The network").
- Refused at load as not enforced yet, so glave will not run under them: `[budget]` in glave itself (only a gateway serving many keeps a daily budget; glave keeps budgets for one run), and `[workers] network = "allowlist"` (no egress allow-list on workers yet).
- `[mcp] catalog` names the servers the organization offers; several catalogs narrow one another. A gateway serving many offers exactly those, from its own `mcp.json`, at `/mcp`, and refuses to start when one has no settings there (`gateway.md`, "Remote tools"). glave with `[gateway] url` takes the catalog from that gateway, and every server of the person's `mcp.json` is then their own. Without a gateway, the servers of the person's `mcp.json` the catalog names count as the catalog's, so `[mcp] own = false` leaves them on and refuses only the others; an organization that needs the catalog's settings out of people's hands serves it from a gateway.
- `[gateway] url` (a choice, `https`, or `http` only to this computer) sends model calls to that gateway and adds its catalog (`gateway.md`, "Client mode"). Only the person's own `config.toml` sets it: a project's is ignored, since the person's sign-in goes there.
- `[provider]` (a choice: `name = "azure"` with `resource` and optionally `scope`; `name = "bedrock"` with `region` and optionally `endpoint` and `api`; or `name = "vertex"` with `project` and optionally `location` and `api`) takes a cloud contract's models in place of `GLAVE_BASE_URL`'s server (`gateway.md`, "Cloud contracts"); `[models]` rules then see `azure/<deployment>`, `bedrock/<model>` or `vertex/<model>`. At Bedrock and Vertex, Claude goes by Anthropic's Messages API and other models by Chat Completions, unless `api = "anthropic"` or `"openai"` sends every model one way. `glave policy explain` shows the provider, where it is and which API. Only the person's or the server's own `config.toml` sets it: a project's is ignored, since the organization's credentials go there. Its keys come only from the environment.
- `[github]` (a choice: `client_id`; optionally `app`, the app's name in its URL, for the page where people choose its repositories; and optionally `url` for a GitHub Enterprise Server, its origin: `https://` and its host, or `http://` only to this computer) names the GitHub App people sign in to GitHub through in the browser (`session.md`, "Git hosts"). The app needs device flow turned on; its user tokens may expire, as GitHub's default has them, since glave renews them. The client id is public, never a secret. Only the person's or the server's own `config.toml` sets it: a project's is ignored, since the person's GitHub sign-in goes there. Without it, GitHub is connected with a token like any git host. Policy cannot yet limit which hosts may be connected: that comes with an organization's own source of credentials (`computers.md`, rule 8).
- **Model accounts** (`session.md`, "Model accounts") are the consumer's default: a key the person connects from a client, sealed in the store, needs no file and no variable. Whatever the flags, the environment or `config.toml` choose still comes first: their model, at their endpoint, is the default, and an account's models are choices beside it. Under `[gateway] url`, or on a server people sign in to with `--issuer`, accounts are not used and cannot be connected: the organization sets up the models. An account's models are checked against `[models]` as the default is; one the policy refuses is left out rather than refusing to start.
- `glave policy explain` prints every setting with its source, every rule with its file, and why glave would refuse to run; it exits non-zero when it would.
- A server that runs for days reads the person's `config.toml`, the policy and `mcp.json` again at the start of a turn when their content's hash changed, and every choice a turn runs with follows from the next turn, the judge with the models it names; a running turn keeps what it started with. It cannot refuse to start, so a choice a constraint added meanwhile refuses gives way as glave's default would, and the log names the rule: constraints still only narrow, whatever the person's or a project's file says (`one-server.md`, "Its settings").

Project trust (issue #83), in `crates/config/src/trust`:

- A repository's `.mcp.json` servers and the loosening parts of its `.glave/config.toml` (`approval = "auto"`, `sandbox = "off"`, a `judge_model` other than `"off"`) run only after the person trusts the project. glave asks once, in the terminal or in an app (issue #250, below), and only when the repository brings such things; a repository without them never asks, and the rest of its `config.toml` (a model, a tighter approval or sandbox) applies at once.
- The answer is kept in the store, in the person's trust ledger (below). What is trusted is each server's definition, so a repository that changes a server, or adds one, is asked about again, and only that.
- `[projects] trusted` pre-trusts projects by path glob, with no question. `[mcp] own = false` forbids a project's servers whatever anyone says; the person's own servers in `mcp.json` stay refused loudly as before.
- With no one to ask (a pipe, a script), untrusted servers stay off, glave says so, and the model is told which tools are off and why. A session server (`glave serve`, which the terminal's commands are clients of) asks the person on this computer in an app, or in the terminal through it, instead (`session.md`, "Project trust"); until they answer, the same holds, and the model is told the person has not trusted the project yet.
- In a session server, every project's trusted servers and hooks run for the threads that work in it, and only for them, the folder the server started in as much as a project made in the browser: no project lends its servers, hooks or settings to the whole server, whose own are the person's. A hook runs the project's own files, so it never runs in another project's folder. Threads that work on workers run the person's own hooks alone. What is trusted is read again at the start of each turn, so a yes is on from the next one. A project's servers start in its folder the first time one of its turns needs them, and stay up for its later turns until 30 minutes pass with no turn of the project using them, for 16 projects at once (the one used least recently is closed first); a closed one starts again when it is next needed. The person's own servers start once, in their home folder, for their threads everywhere and never a thread of anyone signed in from elsewhere. A project's server whose name one of the person's own servers has stays off: the person's wins, and the model and the person are told.
- Of a project's trusted settings, `approval = "auto"` and `sandbox = "off"` apply to its threads under the rules the terminal settles them by: a flag is nearer than the project, `[sandbox] required` and `[approval] modes` win, and commands run unconfined with nothing asked only when the sandbox off and auto are both chosen; a project that turns the sandbox off, with auto chosen nowhere, makes its threads ask. In a thread whose project runs its commands unconfined with auto chosen nowhere, the thread asks whatever mode it chose: its auto was chosen behind the sandbox. Trust is the person's decision about their own computer, so what a project turns on runs only for the threads of the person on this computer: a thread of anyone signed in from elsewhere gets none of it, trusted or not (no servers, no hooks, no settings), and its model is told they are off.
- Every other choice in a project's `config.toml` is its threads' alone too, read at the start of each turn as its trust is: its model, `network = false`, `approval = "ask"`, `sandbox = "read-only"`, `worktree`, `titles` and `project_memory` at once; its `judge_model` and a wider `[web]` once trusted (without trust its `[web]` keeps only `fetch` and `deny`). The nearest choice wins: the thread's own, a flag or variable the server started with, the project's, the person's file, glave's defaults. A project's model, and its judge's, must be ones the server offers: another is set aside with a sentence, and its threads use the server's. Settings the constraints refuse are set aside whole, with why. A project never sets what only the person's or the server's own file sets (store, gateway, provider, GitHub, telemetry, server, serve), nor turns on the person's own servers (`mcp` picks among its own). Not yet per project: `on` and `[workers]`, which stay the server's.
- `glave policy explain` lists the project's items and where each stands.

Web and untrusted servers (issue #94; `approvals.md`, "Untrusted content"):

- `[web]` in `config.toml` (`fetch`, `allow`, `deny`, `private_addresses`) is one choice, the nearest table winning; patterns are checked when read. Without trust a project's `[web]` keeps only `fetch` and `deny`. `glave policy explain` shows it.
- What an untrusted MCP server returns is labelled untrusted. A project's servers and remote (HTTP) servers are untrusted by default; the person's own stdio servers are trusted. The person's `mcp.json` may mark a server of theirs either way (`"untrusted": true` or `false`); a project's `.mcp.json` may only add `"untrusted": true`. `[mcp] untrusted` in a policy names servers that are untrusted whatever else says; `[mcp] trusted` vouches for remote and project servers left unmarked, and, like `[projects] trusted`, any file may add to it, but it never lifts an `untrusted` mark (`approvals.md`, "Untrusted content").
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

- **Choices** say what is used: the model, the judge model, the approval mode, the sandbox level, which MCP servers are on. **The nearest level that sets a choice wins.**
- **Constraints** say what is allowed: which models and MCP servers, which approval modes, whether the sandbox is required, budgets. **Constraints only narrow on the way down, and a deny always wins.**
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

1. the organization's policy: served by the organization's glave server as a signed, versioned bundle, or installed by device management at `/etc/glave/policy.toml` (macOS: managed preferences; Windows: Group Policy);
2. a project's own policy, which may only narrow the organization's (a client's sealed project, say);
3. none at all for a person on their own computer, whose personal organization allows everything.

A consumer never sees a policy. They set at most a few choices.

## The files

`config.toml` (choices; every key optional):

```toml
model = "anthropic/claude-sonnet-5-5"
judge_model = "anthropic/claude-haiku-5-5"   # or "off"
approval = "auto"                             # or "ask"; default auto in the sandbox, ask without
sandbox = "project"                           # the default where it runs; or "read-only", "off"
network = false                               # sandboxed commands' network; default true
mcp = ["github", "jira"]                      # servers to turn on, from the catalog
worktree = true                               # each thread in a worktree of its own; default false
titles = false                                # no model-written titles; default true
on = "cloud"                                  # each thread on a worker of its own; default "here"

[[models]]                                    # models a person may choose for a thread besides
id = "hope-12b"                               # `model` (session.md, "Choosing a model…"), nearest list wins;
name = "Hope"                                 # each is checked against [models] like `model`

[web]                                         # what web_fetch may reach (approvals.md), nearest wins
fetch = true                                  # default true; false turns web_fetch off
allow = ["docs.rs", "*.example.com"]          # hosts it may fetch; every host when absent
deny = ["*.internal.example.com"]             # hosts it never fetches; a deny wins
private_addresses = false                     # default false; a host allow names exactly may be private anyway

[gateway]                                     # the person's file only: the organization's gateway
url = "https://gateway.example.com"           # model calls and the tool catalog go through it

[server]                                      # the person's file only: a session server elsewhere
url = "https://glave.example.com"             # glave runs threads there (glave --server); sign in with GLAVE_SERVER_TOKEN

[serve]                                       # the person's or server's file only: how people reach this glave
origin = "https://glave.example.com"          # its public origin, served over HTTPS by a tunnel or proxy in front;
                                              # without it glave answers on this computer alone
# domain = "glave.example.com"                # or a name glave serves HTTPS for itself (not with origin), on
                                              # port 443 unless glave serve --https says where
# acme = "https://acme-staging-v02.api.letsencrypt.org/directory"  # where its certificate comes
                                              # from, with domain; Let's Encrypt when absent

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

[workers]                                     # where workers run (computers.md), nearest wins
backend = "docker"
glave = "/opt/glave/linux-arm64/glave"         # a Linux build of glave for the backend's processor
socket = "/var/run/docker.sock"              # optional: Docker's socket
# backend = "kubernetes"                      # or a cluster, with optionally:
# kubeconfig = "/etc/glave/kubeconfig"        # default KUBECONFIG, ~/.kube/config, or in-cluster
# namespace = "glave-workers"                 # default the kubeconfig's
# runtime_class = "gvisor"                    # gVisor's runsc or Kata: a kernel or VM per worker
```

Hooks are the one thing in `config.toml` that adds up across files rather than the nearest winning: the person's and a trusted project's all run.

`policy.toml` (constraints; every section optional):

```toml
version = 1

[models]
allow = ["acme-vllm/*", "bedrock/anthropic.*"]   # globs over provider/model
deny = []

[approval]
modes = ["ask", "auto"]
judge = "required"                                # or "allowed", "off"

[sandbox]
required = true
network = false                                   # commands go without the network ("The network")
ask_network = false                               # nor may one ask for it; asking is allowed when absent

[mcp]
catalog = ["github", "jira"]                      # the organization's approved connections
own = false                                       # may people add their own connections, local or remote
untrusted = ["search"]                            # servers whose output is untrusted (approvals.md)
trusted = ["docs"]                                # remote or project servers vouched for

[placement]
allowed = ["here", "worker", "laptop"]            # where threads may work; every place when absent.
                                                  # "laptop" (a person's computer dialled in) is not available yet

[budget]
person_per_day_usd = 20

[projects]
trusted = ["/srv/repos/*"]                        # projects at these paths (globs) need no question

[hooks]
own = false                                       # may people and projects run hooks

[workers]                                         # what workers may be (computers.md)
cpu = 4                                           # the most CPUs; the size when none is named
memory_gb = 8
max_minutes = 120                                 # after this, a worker takes no new work
per_person = 3                                    # workers one person may have at once
network = "on"                                    # or "off"; "allowlist" is refused for now
isolation = "vm"                                  # or "container": "vm" needs a runtime class

[tasks]                                           # threads that run with no one attached (session.md)
max_running = 3                                   # tasks one person may have running at once
max_schedules = 10                                # schedules one person may keep
```

Unknown keys are errors that name the key and file, so a typo never silently does nothing.

The policy file is the source of truth, and an organization's admin console is an editor for it: changes made in the console are written to the same policy and recorded as events, so every change has a history and an author. Admins who prefer configuration as code edit the file; both reach the same result.

## Enforcement

Each constraint is enforced where it cannot be bypassed:

| Constraint | Enforced by |
| --- | --- |
| models, budgets | the gateway, which holds the keys: in glave, or as a service for each caller |
| approval modes, the judge | the session API and the harness's tool gate |
| sandbox, and its network | the computer port, before any command runs |
| asking for the network | the harness's tool gate, before anyone is asked |
| MCP servers | glave, when it starts servers; the gateway, which serves only the catalog's |
| hooks | the CLI, before the harness is built: forbidden hooks are never handed to it |
| workers | the worker adapter (`WorkerComputers`), before a worker starts and on every call to it |

A laptop's own config can never loosen a constraint, because the constraint arrives from the organization (a signed bundle or device management), not from a file the person edits.

## Explaining

`glave policy explain` prints every effective setting, which file or flag set it, and every constraint with its source. Every denial names the rule that denied it and where that rule came from. Each turn records the policy bundle version it ran under, in its configuration blob.

## Connections

A connection (`model.md`, "The hierarchy") reaches its service through a tool server started on the person's computer or run remotely. Decided on 2026-10-23 (#633); what is built is `[mcp]` ("Status"), the gateway's catalog (`gateway.md`, "Remote tools: the catalog"), and signing a person in to a remote server of their own `mcp.json` (`session.md`, "Connections").

- **One list per organization: its approved connections,** kept in its policy file as `[mcp] catalog`.
- **Services that hold documents** (Google Drive, SharePoint, Confluence) are connections like any other. glave's own knowledge bases are a separate thing, planned in #661: curated documents searched by meaning, each with an owner, granted to groups, and private to whoever made it by default.
- **Its default follows who the organization is, never the deployment.** A company's is "only from our list": it runs no other tool server, whatever a file says (`[mcp] own = false`). An organization of one's is "anyone can add".
- **"May add their own" is one grant,** given to groups (#645). It covers servers started on the person's computer and remote ones alike: both reach data with that person's sign-ins, so one answer fits both.
- **Connect**, the page that lists connections, writes one line into the person's own `mcp.json` and signs them in. An approved connection needs only the sign-in, and no file is written.
- **Everyone signs in as themselves.** Shared keys, where an organization needs one, live only in its gateway, never on a person's computer.
- **A tool server from a repository's file asks "run this?"** before its first run, and again when its definition changes, for each person ("Project trust", as built). An approved connection is never asked about.
- **Approved is not vouched for.** Whether what a connection returns is labelled untrusted follows `approvals.md` ("Untrusted content"), with `[mcp] untrusted` and `[mcp] trusted`, whether it is approved or not; a tool that reaches the open world is labelled anyway.
- **Safety is the industry's baseline today:** the approved connections, each person's own sign-in, approvals (`approvals.md`), the sandbox around every command (`sandbox.md`), untrusted content labelled, and every tool call recorded (`events.md`). A stronger rule waits for its own decision and measurement: checking a write through a connection after untrusted content is #634.

## The network

Decided on 2026-10-23 and built (#641). Web search, the tool beside `web_fetch` the issue named, waits for its search provider to be chosen (#677).

- **On by default for a person on their own** (a laptop, their own server, Glave Cloud's personal plan): glave's own default, an organization of one's. **Off by default in an organization:** its policy file says `[sandbox] network = false`, a constraint that narrows the `network` choice as every constraint does. glave's default gives way to off ("default, narrowed by" the file); a person's `network = true` or `--network` is an error naming the rule, and on a server already running it gives way, said in a sentence. Glave Cloud's enterprise plan writes it into the policy each organization starts with (there is no template file yet; the admin console, #222, edits the policy).
- **The switch always exists:** `network` (`network = false`, `--no-network`) in every deployment. A person on their own turns it off; an organization's policy keeps it off; a project's file can only take it away.
- **Only the sandbox keeps the network from commands on the person's computer,** so the rule requires the sandbox, as `[sandbox] required` does: where it cannot run, glave refuses to start, and `sandbox = "off"`, a person's or a trusted project's, is refused.
- **A command that needs the internet asks** (`npm install`, a build fetching dependencies, `git push`). The model runs it again with `network: true` and a reason, and the person is asked to allow that one command with the network, in the sandbox otherwise: "It can reach the internet, which commands here otherwise cannot." It is the request to run outside the sandbox narrowed to the network, and asks as that one does (`approvals.md`, "Running outside the sandbox"): in every mode, each time, and with no one there it never runs. Where commands have the network, nothing asks, and the model is not offered it. A command that needs more than the network (a build writing the person's own package cache) still asks to run outside the sandbox.
- **The policy says whether asking is allowed;** it is unless `[sandbox] ask_network = false`. Then no command without the network asks for it, nor to run outside the sandbox, which has it: the model is offered neither and told so, a call that asks anyway is refused, and the person is never asked. An answer never widens what the policy allows: a command whose question was answered before the sandbox changed is refused, not run, if it would now leave the sandbox otherwise than the person was asked.
- **Workers go without it too:** the rule narrows `[workers] network` to off. A worker's network is its container's, set when it starts, so no command on one asks.
- **glave's own web tools stay,** as the decision has it: `web_fetch` reads the web for the model, labelled untrusted and limited by `[web]`, and so will web search (#677). A URL it fetches can still carry what the model read; `[web]` rules in a policy, to narrow its hosts, are not built yet.
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
- What needs trust: each server in the project's `.mcp.json`, each hook in its `config.toml` (item `hook:<Point>:<n>`, its definition the hook's point, command, matcher and timeout as JSON), and `approval = "auto"` or `sandbox = "off"` in its `config.toml`, `[web] private_addresses = true` or a `[web] allow` list (a host it names exactly may be private), and a `judge_model` other than `"off"` (a repository must not choose who judges its own actions; `off` only makes glave ask more and applies at once). Nothing else asks. Servers in `~/.glave/mcp.json` are the person's own and always trusted to start; on a name clash the person's wins. Trusting a project lets its servers start; what they return is still labelled untrusted unless vouched for (above).
- An organization's policy can pre-trust projects (`[projects] trusted`), or forbid project MCP servers (`[mcp] own = false`) or hooks (`[hooks] own = false`) entirely. A forbid wins over a pre-trust and over the person's yes.
- With no one to answer, an untrusted project's servers stay off, and the model is told why: its instructions gain a "Project tools that are off" section listing what is off and why. Hooks that stay off are told to the person only: the model never calls a hook.
- In an app the person answers "Trust this project" or "Not now" (`session.md`, "Project trust"). A trust records exactly what a yes in the terminal records, per item and definition, so the terminal never asks about it again and a changed or added item is asked about again, and only it. "Not now" records nothing: like a terminal no one answered, it is asked again next time, on a new thread. Nothing is ever trusted without one of these answers from the person on this computer: there is no default yes, and an answer names exactly what the person was shown, so a project that changed in between trusts nothing and is asked again.
