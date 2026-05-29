# Harness Landscape — Intro

Date: 2026-05-29

This folder is Glave's tracked research on the live coding-agent **harness**
landscape — the control loops that wrap a model and turn it into an agent. It
exists to answer one question before Glave rebuilds Hope's harness from a clean
base:

> Of everything the field has already learned the hard way, what must Glave have
> parity with, what should it do differently, and what should it refuse to build?

The durable analysis lives here in `context/research/harnesses/` (tracked). The
raw material — shallow source clones of each open harness — lives in `refs/`
(gitignored). If `refs/` is ever wiped, this folder is the thing that survives.

## Ground rule

These notes treat every harness as a **design signal, not a source dependency**.
Do not clone, quote, port, or derive implementation from leaked Claude Code
source or leak-adjacent clean-room clones. The nine harnesses written up here are
all open source under permissive licenses, read for architecture and feature
ideas only. Glave's harness should be original, typed, server-owned, and
evaluated against its own traces.

## What we classify

We classify the **harness** — the control loop, tools, policy, and state — not
the company's model or hosted product. By that rule the nine cloned projects are
all genuinely open even when they wrap a closed model (e.g. Codex CLI is
Apache-2.0 even though GPT-5/Codex models and OpenAI's hosted "Codex Web" are
closed; same for Gemini CLI over closed Gemini models).

The **closed column** — Claude Code, Cursor, Windsurf, Devin, GitHub Copilot
agent mode, Amp, Replit Agent, Jules, Junie, Factory — has no readable harness,
so it is covered only at a high level in `executive-summary.md` from public
information (features + sentiment), never from source.

## The nine open harnesses

Each has a dedicated file with full feature inventory and pros/cons. They span
the four families the market clusters into.

| Harness | Family | Lang | License | One-line | Headline lever |
| --- | --- | --- | --- | --- | --- |
| [Aider](./aider.md) | Terminal pair-programmer | Python | Apache-2.0 | Edits your git repo directly via text edit-formats + repo maps | Structural token-budgeted **repo map**; measured edit-format/model routing |
| [Pi](./pi.md) | Minimal terminal | TypeScript | MIT | Deliberately tiny core (4 tools), everything else is an extension | **Subtraction discipline** + clean provider-portable transport (`pi-ai`) |
| [OpenCode](./opencode.md) | Terminal (TUI) | Go | MIT | Single-binary TUI agent with LSP + SQLite sessions | **LSP diagnostics** as cheap deterministic in-loop feedback |
| [Codex CLI](./codex.md) | Terminal + app-server | Rust | Apache-2.0 | OpenAI's Rust terminal agent over a typed app-server protocol | **Sandbox/permission as a typed contract**; Rust one-engine-many-surfaces |
| [Gemini CLI](./gemini-cli.md) | Terminal | TypeScript | Apache-2.0 | Google's terminal agent; free-tier auth, search grounding, huge context | **Benchmark-gated cost-aware prompting**; layered compaction |
| [Goose](./goose.md) | General-purpose (desktop/CLI/API) | Rust | Apache-2.0 | Task-general agent; coding is just one extension | **ACP subscription-reuse auth**; pluggable safety-inspector pipeline |
| [Cline](./cline.md) | IDE + terminal | TypeScript | Apache-2.0 | Human-in-the-loop agent across VS Code/JetBrains/CLI/SDK | **Shadow-git checkpoints**; Plan/Act; per-model-family prompt compiler |
| [OpenHands](./openhands.md) | Cloud/platform | Python | MIT (+enterprise carve-out) | Broadest open platform: sandboxed agents, SaaS, enterprise | **Action/Observation event model**; server-owned trust boundary |
| [SWE-agent](./swe-agent.md) | Research/benchmark | Python | MIT | The academic ACI reference, measured on SWE-bench | **Measured eval flywheel**; explicit `submit`; Mini-SWE-agent subtraction proof |

## The four families

1. **Developer-terminal agents** — Aider, Pi, OpenCode, Codex CLI, Gemini CLI
   (and Claude Code in the closed column). Fast, local, developer-in-the-loop.
2. **IDE / human-approval agents** — Cline (and closed Cursor/Windsurf). Approvals
   and visibility are part of the UX.
3. **Cloud / autonomous platforms** — OpenHands (and closed Devin-likes). Remote
   sandbox, API/SDK, integrations, multi-tenant.
4. **Research harnesses** — SWE-agent / Mini-SWE-agent. Benchmark-first; the ACI
   and the eval loop are the product.

Goose sits slightly outside the four as a deliberately **task-general** runtime,
which is why it matters to Glave: it's structural proof that "coding is one task
type on the same engine" is buildable.

## How to read this folder

- Start here, then read `executive-summary.md` for the compiled findings and the
  recommendations framed for Glave as an econo-frontier company.
- The per-harness files share a fixed structure: Overview → Architecture →
  Feature Inventory → Standout/Unique → Pros → Cons → **Lessons for Glave** →
  Source. If you only read one section per file, read *Lessons for Glave*.
- Every file is grounded in the actual cloned source (file paths, tool names,
  config keys cited), not from memory or marketing.

## What this folder is not

It is not a build spec and not a ranking of "best agent." It is a map of solved
problems and earned scars, filtered through one lens: **cost per correct
completed task on a server-owned typed harness.** The recommendations live in
`executive-summary.md`.
