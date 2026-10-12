# Roadmap

What glave is building toward, what exists, and what comes next. The rules for building it are in [AGENTS.md](AGENTS.md); the contracts are in [specs/](specs/). Work is tracked in issues; this page is the map.

## What glave is

An open-source Claude.ai or ChatGPT that you run yourself, where any conversation can have a real computer behind it. Install it on your own machine and it just works, privately, with any model, including Hope, a free open model. A company runs the very same software as its own private AI cloud.

Glave is to OpenAI and Anthropic what GitLab is to GitHub. It's the same product for everyone, run wherever you choose:

- **People** run it on a laptop or a small server: your own AI, free, private and yours, with nothing to set up.
- **Companies** self-host it on their own infrastructure, under their sign-in, models, data and policy.
- **Glave Cloud** (glave.ai, later) is the same software, hosted by us for many organizations.

It's one binary, used in the browser. Everything that differs between a person and a company sits behind a seam: a built-in default that needs no setup, and a protocol for plugging in your own. A person never sees the enterprise side (AGENTS.md, "Keeping principle 4").

## Where we are

Built, tested, and evaluated on three models:

| Part | What exists |
| --- | --- |
| Web app | Threads streaming live, approvals and questions, a model picker and approval mode per thread, automatic titles, tool steps, a tools panel (tasks and changes), memory, settings; Glaze, our component library |
| Harness | The agent loop and its tools, an auto-mode judge, goals, subagents, skills, commands, hooks, compaction, memory |
| Computers | This machine in a sandbox (macOS, Linux), worktrees per thread, cloud workers on Docker and Kubernetes |
| Gateway | Routing, fallback, budgets and metering. Any OpenAI-compatible or Anthropic model, plus Azure, Bedrock and Vertex with keyless sign-in |
| Data | SQLite or Postgres, encrypted at rest with per-space keys |
| Session API | In-process, stdio, socket, and HTTP with OIDC sign-in; tasks and schedules |

## Next: the pilot

The first pilot is a company self-hosting glave on its own GKE cluster. Work alternates between what a person feels and what a company needs, so neither starves the other.

| | Piece | Issue |
| --- | --- | --- |
| Person | First run: `glave` opens the browser, add a model by pasting a key | #248 |
| Person | Credentials: Connect GitHub, any git host, tokens kept out of the computer | #249 |
| Person | Trust a project from the browser; guardrails that keep enterprise out of view | #250, #251 |
| Person | Markdown in replies, a model picker fix, attachments, reasoning effort | #226, #235, #243 |
| Company | A reference GKE cluster and a Helm chart | #220 |
| Company | Computers on the `agent-sandbox` operator: lazy, sized per group, sleeping when idle | #221 |
| Company | Control plane: org config as data, an admin API and an admin MCP | #222 |
| Company | Seams for a skills library, secrets and audit; a security pack for an enterprise review | #223, #224 |
| Both | Projects with a source (folder or repository); the project page, search, notifications | #239 |
| Both | Tool search, so many connected tools don't crowd the context | #247 |

## 1.0

The pilot's pieces, hardened. Then:

- Open source, public under `glave-ai`, Apache-2.0.
- Packaging: an install script and Homebrew, plus a container image.
- Docs in two tracks: using glave, and running it.

## After 1.0

| Piece | Why |
| --- | --- |
| Glave Cloud | The same software hosted for many organizations: multi-tenancy, sign-up, billing |
| Hope, trained and hosted | A free model tuned for glave. Training on rented GPUs; private hosted inference on confidential GPUs |
| The tools panel's terminal and live previews | After the credential proxy keeps secrets out of the computer (#225) |
| Voice: an assistant that's always ready, with a wake word, listening, speaking | #58 |
| The agent's browser, the knowledge pack | #95, #144 |
| SCIM, SAML, SIEM export, chargeback | When an organization asks |

## How we decide

Industry defaults first, then the simplest design that works, then evals on real models. Nothing optional is on by default unless it's invisible and harmless, and no enterprise feature adds a click for a person.
