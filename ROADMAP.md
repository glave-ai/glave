# Roadmap

What glave is building toward, what exists, and what comes next. The rules for building it are in [AGENTS.md](AGENTS.md); the contracts are in [specs/](specs/).

## What glave is

The open-source user agent for AI: one binary that runs anywhere, works with any model, and ships Hope as its free house model.

- **For people:** an affordable, private AI that gets things done (files, documents, errands, code), runs on your own machine, and is yours.
- **For companies:** own your AI. Your models, your data, your rules, your record: glave runs on your infrastructure, under your sign-in and policy, with every action attributed and auditable.

Both run the same code. People get simple defaults; companies plug in their own identity, storage, models and compute.

## Where we are

Built, tested and evaluated on three models:

| Part | What exists |
| --- | --- |
| Harness | the agent loop and its tools (search, glob, read, write, edit, bash, web fetch), approvals and an auto-mode judge, goals, subagents, skills, custom commands, hooks, compaction, retries |
| Computers | this machine, remote computers, cloud workers on Docker and Kubernetes (VM isolation), an optional worktree per thread, hardened file confinement, a fault-tolerant link |
| Sandbox | macOS (Seatbelt) and Linux (bubblewrap and seccomp, Landlock as a fallback) |
| Data store | SQLite and Postgres, encrypted at rest with per-space keys and erasure |
| Gateway | routes by purpose, fallback, metering and budgets; as a service with OpenAI- and Anthropic-compatible endpoints, sign-in and a remote tools catalog |
| Session API | in-process, stdio, Unix socket, and HTTP with OIDC sign-in; background tasks and schedules |
| Policy | config and policy files, project trust, `glave policy explain` |
| CLI | everything above, from the terminal |

## 1.0

The first release people and companies can adopt.

| Piece | Status |
| --- | --- |
| Web interface: threads, approvals, tasks and schedules, files handed back, run in the cloud, settings over the config files, and a read-only admin view (policy, usage, audit) | next |
| Memory: what glave remembers, where, and how you see, edit and forget it ([#93](https://github.com/seashell86/glave/issues/93)) | spec in progress |
| Hope: an adapter for better tool use, served through the gateway | not started |
| Packaging: one binary, install script and Homebrew, release builds | not started |
| Website and docs | not started |
| Open source: public under the `glave-ai` organization, Apache-2.0 | at 1.0 |

## After 1.0

| Piece | Why |
| --- | --- |
| Knowledge pack ([#144](https://github.com/seashell86/glave/issues/144)) | Wikipedia, docs and news offline, and a company's own documents through the same engine |
| Hope, trained | a full fine-tune for brevity, taste and calibrated reasoning, then a small distilled model, on glave's own inference server |
| Admin console | editable policy, the tool catalog, budgets and people, every change an event |
| Identity | SAML and SCIM: teams, groups, and spending caps per team |
| Plan mode ([#92](https://github.com/seashell86/glave/issues/92)), errands ([#95](https://github.com/seashell86/glave/issues/95)), voice ([#58](https://github.com/seashell86/glave/issues/58)) | the rest of the harness |
| Cloud homes | a persistent disk per person for loose files, that sleeps cheaply |
| Cheap code execution on Glave Cloud | a warm pool of small gVisor workers on the Kubernetes backend, snapshots if start time matters, WebAssembly Python only if measurements say most cloud chats are simple data work. Locally and on a company's own server, the sandbox already runs code for free |
| Desktop app | the web interface, packaged |

## How we decide

Industry defaults first, then the simplest design that works, then evals on real models. Nothing optional is on by default unless it is invisible and harmless, and no enterprise feature adds a click for a person.
