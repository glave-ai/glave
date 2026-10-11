# Config and policy

How glave is set up: what a person or project chooses, and what an organization allows. Tracks issue #22, with the project trust of issue #83.

## Status

First cut built, in the CLI (`crates/cli/src/policy`):

- `~/.glave/config.toml` (in `GLAVE_HOME`) and `/etc/glave/policy.toml` are read as typed TOML; unknown keys, and a policy without `version = 1`, are errors naming the file and key.
- Choices resolve nearest-wins across flags (`--model`, `--approval`, `--sandbox`, which now takes `off`), the environment (`GLAVE_MODEL`, `GLAVE_JUDGE_MODEL`), the person's file and the defaults, each remembering its source. `mcp` turns on only the named servers of `mcp.json`. `worktree` (`true` or `false`, default `false`; also `--worktree`) gives each new thread a git worktree of its own instead of the folder itself (`computers.md`); it is a plain choice, needs no trust in a project's file, and `glave policy explain` shows it with its source. `on` (`here` or `cloud`, default `here`; also `--on`) puts each new thread on a worker of its own, made on the backend `[workers]` names (`backend = "docker"` or `"kubernetes"`, `glave`, the Linux build that goes into workers; for Docker optionally `socket`; for Kubernetes optionally `kubeconfig`, `namespace` and `runtime_class`; a key the backend does not use is an error); `--on cloud` without `[workers]` is an error saying so.
- Constraints only narrow: allow-lists intersect, deny lists add up, a requirement anywhere holds, limits take the smallest, and contradictions (no approval mode left, a judge both required and off) are errors.
- A choice someone made outside the constraints is an error naming the rule, its file and the choice's source. Only glave's own defaults give way, to what the policy leaves (shown as "default, narrowed by" the file).
- Enforced: `[models]` when the gateway's targets are built, `[approval] modes` and `judge` in the settings the harness runs with, `[sandbox] required` before the workspace exists, `[mcp] own = false` before any server starts, `[workers]` (cpu, memory_gb, max_minutes, per_person, network on or off, isolation) by the worker adapter: limits take the smallest, `off` wins over `on`, and `vm` over `container`; a policy requiring `vm` refuses every worker on a backend without a runtime class.
- Refused at load as not enforced yet, so glave will not run under them: `[budget]` (the gateway keeps budgets per run, not per day) and `[mcp] catalog` (glave cannot start servers an organization offers), and `[workers] network = "allowlist"` (no egress allow-list on workers yet).
- `glave policy explain` prints every setting with its source, every rule with its file, and why glave would refuse to run; it exits non-zero when it would.

Project trust (issue #83), in `crates/cli/src/trust`:

- A repository's `.mcp.json` servers and the loosening parts of its `.glave/config.toml` (`approval = "auto"`, `sandbox = "off"`, a `judge_model` other than `"off"`) run only after the person trusts the project. glave asks once, in the terminal, and only when the repository brings such things; a repository without them never asks, and the rest of its `config.toml` (a model, a tighter approval or sandbox) applies at once.
- The answer is kept in `trust.toml` in glave's folder (below). What is trusted is each server's definition, so a repository that changes a server, or adds one, is asked about again, and only that.
- `[projects] trusted` pre-trusts projects by path glob, with no question. `[mcp] own = false` forbids a project's servers whatever anyone says; the person's own servers in `mcp.json` stay refused loudly as before.
- With no one to ask (a pipe, a script, `glave serve`), untrusted servers stay off, glave says so, and the model is told which tools are off and why.
- `glave policy explain` lists the project's items and where each stands.

Hooks (issue #46; `approvals.md`, "Hooks"):

- `[[hooks.<Point>]]` in `config.toml` (`PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `Stop`; `command`, optional `matcher` and `timeout`), checked when read: an unknown point or key, a matcher that is not a regular expression, or a matcher on a point without a tool is an error naming the file.
- Hooks add up instead of the nearest winning: the person's, then a trusted project's. They are not choices, so flags and the environment have none.
- A project's hooks are trust items, each bound by its whole definition. `[hooks] own = false` forbids them, and makes glave refuse to start while the person's own `config.toml` has hooks. `glave policy explain` lists the person's hooks and the project's with where each stands.

Not built yet: a command to change a decision (edit `trust.toml`); signed bundles from an organization's server, macOS managed preferences and Group Policy; a space's own policy; recording policy changes as events (the admin console); and recording the policy version in each turn's configuration.

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
2. a space's own policy, which may only narrow the organization's (a client's sealed space, say);
3. none at all for a person on their own computer, whose personal organization allows everything.

A consumer never sees a policy. They set at most a few choices.

## The files

`config.toml` (choices; every key optional):

```toml
model = "anthropic/claude-sonnet-5-5"
judge_model = "anthropic/claude-haiku-5-5"   # or "off"
approval = "auto"                             # or "ask"
sandbox = "project"                           # or "off", "read-only"
mcp = ["github", "jira"]                      # servers to turn on, from the catalog
worktree = true                               # each thread in a worktree of its own; default false
on = "cloud"                                  # each thread on a worker of its own; default "here"

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

[mcp]
catalog = ["github", "jira"]                      # servers the organization offers
own = false                                       # may people add their own servers

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
| models, budgets | the gateway, which holds the keys |
| approval modes, the judge | the session API and the harness's tool gate |
| sandbox | the computer port, before any command runs |
| MCP servers | the harness, when it starts servers |
| hooks | the CLI, before the harness is built: forbidden hooks are never handed to it |
| workers | the worker adapter (`WorkerComputers`), before a worker starts and on every call to it |

A laptop's own config can never loosen a constraint, because the constraint arrives from the organization (a signed bundle or device management), not from a file the person edits.

## Explaining

`glave policy explain` prints every effective setting, which file or flag set it, and every constraint with its source. Every denial names the rule that denied it and where that rule came from. Each turn records the policy bundle version it ran under, in its configuration blob.

## Project trust

- A project's `.glave/config.toml` and its `.mcp.json` can run commands (MCP servers and hooks). So they are honoured only after the person trusts the project once. glave asks at most once per project, and only when the project brings something that runs commands.
- Trust decisions are kept in `trust.toml` in `~/.glave` (or `GLAVE_HOME`), one `[[project]]` per repository, keyed by its canonical root path. A clone that moved is recognised by its `origin` remote, read from the repository's config file without running anything. Each item records the exact definition that was answered, so trust is of the content:

  ```toml
  version = 1

  [[project]]
  path = "/Users/ada/code/app"
  remote = "git@example.com:acme/app.git"

  [project.items."mcp:docs"]
  definition = '{"args":["--fast"],"command":"docs-mcp"}'
  trusted = true          # false: declined, and not asked again until it changes
  ```
- What needs trust: each server in the project's `.mcp.json`, each hook in its `config.toml` (item `hook:<Point>:<n>`, its definition the hook's point, command, matcher and timeout as JSON), and `approval = "auto"` or `sandbox = "off"` in its `config.toml`, and a `judge_model` other than `"off"` (a repository must not choose who judges its own actions; `off` only makes glave ask more and applies at once). Nothing else asks. Servers in `~/.glave/mcp.json` are the person's own and always trusted; on a name clash the person's wins.
- An organization's policy can pre-trust projects (`[projects] trusted`), or forbid project MCP servers (`[mcp] own = false`) or hooks (`[hooks] own = false`) entirely. A forbid wins over a pre-trust and over the person's yes.
- With no one to answer, an untrusted project's servers stay off, and the model is told why: its instructions gain a "Project tools that are off" section listing what is off and why. Hooks that stay off are told to the person only: the model never calls a hook.
