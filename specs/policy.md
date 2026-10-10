# Config and policy

How glave is set up: what a person or project chooses, and what an organization allows. Tracks issue #22, with the project trust of issue #83.

## Status

Draft, specified ahead of the code. Today the CLI reads environment variables and `~/.glave/mcp.json`; nothing below is enforced yet.

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
```

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
```

Unknown keys are errors that name the key and file, so a typo never silently does nothing.

## Enforcement

Each constraint is enforced where it cannot be bypassed:

| Constraint | Enforced by |
| --- | --- |
| models, budgets | the gateway, which holds the keys |
| approval modes, the judge | the session API and the harness's tool gate |
| sandbox | the computer port, before any command runs |
| MCP servers | the harness, when it starts servers |

A laptop's own config can never loosen a constraint, because the constraint arrives from the organization (a signed bundle or device management), not from a file the person edits.

## Explaining

`glave policy explain` prints every effective setting, which file or flag set it, and every constraint with its source. Every denial names the rule that denied it and where that rule came from. Each turn records the policy bundle version it ran under, in its configuration blob.

## Project trust

- A project's `.glave/config.toml` and its `.mcp.json` can run commands (MCP servers, later hooks). So they are honoured only after the person trusts the project once. glave asks at most once per project, and only when the project brings something that runs commands.
- Trust decisions are kept in `~/.glave`, keyed by the project's path and repository identity.
- An organization's policy can pre-trust projects, or forbid project MCP servers entirely.
- With no one to answer, an untrusted project's servers stay off, and the model is told why.
