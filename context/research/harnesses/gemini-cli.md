# Gemini CLI

**Identity:** Google's open-source terminal coding agent — a TypeScript/React (Ink) CLI wrapping a server-style "core" agent loop, distinguished by free-tier OAuth auth, Google Search grounding, multimodal input, and very large Gemini context windows.
**Source:** https://github.com/google-gemini/gemini-cli · Apache-2.0 · TypeScript (Node ≥20, React/Ink TUI) · snapshot `c82e2b5`
**Family:** terminal (with a cloud/ACP path via the experimental `a2a-server` and a separate GitHub Action product)
**Open/closed note:** The CLI harness is open source (Apache-2.0, the cloned code); the Gemini models it drives are closed/hosted by Google. Local routing can use an open **Gemma** model for routing decisions only.

## Overview

Gemini CLI is a polished, feature-maximalist terminal agent. The repo is an npm monorepo (`packages/*`): `cli` (Ink/React TUI + non-interactive runner), `core` (the agent engine — tools, scheduler, prompts, context management, policy, MCP, telemetry), `sdk` (programmatic API), `a2a-server` (experimental Agent-to-Agent / ACP remote-agent server), `vscode-ide-companion` (IDE integration), plus `devtools` and `test-utils`. The bin is `gemini`. Version at snapshot is a `0.45.0-nightly` build, so the surface is broad and actively churning.

The product positioning (README) is a generalist that happens to start with coding: a **free tier** (60 req/min, 1,000 req/day via personal Google OAuth), **Gemini 3 with a ~1M-token context window**, **built-in Google Search grounding**, **multimodal** input (generate apps "from PDFs, images, or sketches"), file/shell/web tools, MCP extensibility, conversation checkpointing, and `GEMINI.md` project context. It also ships an external **GitHub Action** (`google-github-actions/run-gemini-cli`) for PR review, issue triage, and on-demand help in CI.

Architecturally it is more than "model + tools." The `core` package contains a real control loop with a separate **scheduler** (parallel tool batching), a layered **context-management pipeline** (compression, tool-output masking/distillation, truncation), a **policy engine** with tiered priorities for permissions, a **hooks** system, **subagents**, **skills**, **plan mode**, **checkpointing/rewind** via a shadow git repo, and **loop detection** (both heuristic and LLM-based). The system prompt is assembled from modular "snippets," and the project's own comments warn engineers to re-run SWEBench before touching the "Context Efficiency" section — a sign cost/turn behavior is actively benchmarked.

Despite the server-like `core`, state is fundamentally **local and client-owned**: sessions, checkpoints, shadow-git history, and OAuth creds all live under `~/.gemini/`. The `a2a-server` is the (experimental) move toward a server-owned remote agent.

## Architecture

**Control loop.** The main loop lives in `packages/core/src/core/client.ts` (`GeminiClient`, ~1300 lines) over `geminiChat.ts` (the streaming chat/history abstraction, ~1470 lines) and `turn.ts` (a `Turn` emits a typed event stream: `Content`, `ToolCallRequest/Response/Confirmation`, `Thought`, `ChatCompressed`, `LoopDetected`, `MaxSessionTurns`, `ContextWindowWillOverflow`, `Citation`, `Retry`, `Finished`, etc.). The loop is model-driven (the model emits function calls) but wrapped in substantial deterministic machinery.

**Tool scheduling is its own subsystem** (`packages/core/src/scheduler/`, ~5k LOC incl. tests). `Scheduler.schedule()` validates calls, runs them through the policy engine, batches contiguous **parallelizable** calls, and executes. Parallelism is the default; a per-call `wait_for_previous` arg forces sequential execution when a tool depends on a prior tool's side effects. Separate files: `policy.ts` (decision), `tool-executor.ts`, `state-manager.ts`, `tool-modifier.ts`, `confirmation.ts`. State per scheduled call is a small state machine (validating → scheduled → executing → terminal).

**Context management is first-class** (`packages/core/src/context/`): a `contextManager.ts`, a `pipeline.ts` of `processors`, `chatCompressionService.ts`, `contextCompressionService.ts`, `toolOutputMaskingService.ts`, `toolDistillationService.ts`, `truncation.ts`, and a `tracer.ts`. Compression triggers when history exceeds `DEFAULT_COMPRESSION_TOKEN_THRESHOLD = 0.5` of the model token limit (overridable), preserves the most recent `COMPRESSION_PRESERVE_THRESHOLD = 0.3` fraction of history plus recent tool outputs in full, and emits a structured XML `<state_snapshot>` summary (compression prompt in `prompts/snippets.ts`). Compression is reconstructable so **rewind works across compression points**.

**Where state lives:** locally under `~/.gemini/` — sessions in `~/.gemini/tmp/<project_hash>/chats/`, checkpoints (shadow git commits) in `~/.gemini/history/<project_hash>`, OAuth creds via `oauth-credential-storage`. Config is layered `settings.json` (admin/user/workspace/system) plus env vars and flags.

**Deterministic vs model-driven:** model drives tool selection, planning, and verification decisions; deterministic layers handle scheduling/parallelism, the policy/permission engine, sandboxing, context compression & truncation, loop detection, checkpoint/rewind, model-fallback routing, and `complete_task` as the single forced finish gate (in non-interactive/subagent flows).

## Feature Inventory

**Project/context discovery (`GEMINI.md`).** Hierarchical context files concatenated and sent with every prompt (`docs/cli/gemini-md.md`): global `~/.gemini/GEMINI.md` → workspace `GEMINI.md` (and parents) → **just-in-time (JIT)** discovery (`tools/jit-context.ts`) that scans for `GEMINI.md` in a directory and its ancestors *when a tool touches that path*. Supports `@file.md` imports for modularization. Filename is configurable (`context.fileName`, e.g. `["AGENTS.md","CONTEXT.md","GEMINI.md"]`). `/memory show|reload` inspects/reloads. `.geminiignore` excludes paths from context.

**Tool / action grammar.** Canonical built-in tools (string names from `tools/definitions/coreTools.ts`):
- File: `read_file` (ranged via start/end line), `read_many_files`, `write_file`, `replace` (the edit tool — fails on ambiguous `old_string`, supports `allow_multiple`), `glob`, `grep_search` (rich: include/exclude patterns, context/before/after, max-matches caps — uses ripgrep, `ripGrep.ts`), `list_directory`.
- Shell: `run_shell_command` (with `is_background`, `additional_permissions`, background-task tools in `shellBackgroundTools.ts`, PTY via node-pty).
- Web: `google_web_search` (search grounding), `web_fetch` (URL fetch + model summarization; 250KB cap, private-IP block, 10 req/window rate limit).
- Planning/UX: `write_todos`, `enter_plan_mode`/`exit_plan_mode`, `ask_user`, `update_topic`, `complete_task` (forced finish).
- Knowledge/extensibility: `activate_skill`, `get_internal_docs`, `list_mcp_resources`, `read_mcp_resource`, the `memory` tool (`memoryTool.ts`, persistent facts), and the subagent invocation tool (`agent`).
- Tracker tools (`trackerTools.ts`) back a file-based task-tracking protocol.

**Search grounding & multimodal.** `google_web_search` returns Gemini `groundingMetadata` (grounding chunks + support segments) and renders inline citations (`web-search.ts`). Multimodal input (images/PDFs/sketches) is a headline capability of the closed Gemini models; the CLI passes media parts through `@google/genai`.

**Permissions & safety.** A real **policy engine** (`packages/core/src/policy/`) with TOML policy files loaded across tiers — `DEFAULT(1) < EXTENSION(2) < WORKSPACE(3) < USER(4) < ADMIN(5)` — and fractional priority offsets for flags (allowed/excluded tools, trusted MCP servers, confirmation-required). Decisions are `ALLOW | DENY | ASK_USER`; e.g. `run_shell_command` defaults to `ASK_USER`. Approval modes: `default`, `autoEdit`, `plan`, `yolo` (autonomous). A `message-bus` (`confirmation-bus/`) brokers confirmations; an `integrity.ts` guards policy tampering. **Sandboxing** (`sandbox/`, `docs/cli/sandbox.md`): macOS Seatbelt profiles or Docker/Podman containers (`GEMINI_SANDBOX`); the system prompt has sandbox-aware failure-recovery instructions. A separate `safety/` module runs built-in + pluggable safety **checkers** (incl. a `conseca` checker) with a context-builder/protocol. **Trusted folders** gate execution in untrusted dirs.

**Checkpointing.** `docs/cli/checkpointing.md` — before any file-mutating tool, the CLI commits to a **shadow git repo** (`~/.gemini/history/<project_hash>`) that doesn't touch the user's repo, and stores conversation + the pending tool call. `/restore [file]` reverts files + history and re-proposes the tool call. Disabled by default (`general.checkpointing.enabled`). **`/rewind`** (Esc Esc) is the richer UX: pick a past interaction and choose to revert conversation, code, or both; works across compression points.

**Sessions & state / persistence.** Auto-saved conversation history (prompts, responses, tool I/O, token stats, thoughts) in `~/.gemini/tmp/<project_hash>/chats/` (`docs/cli/session-management.md`). Resume/branch supported. `logger.ts` + `recordingContentGenerator.ts` record traces.

**Context management & huge-window posture.** Despite a ~1M-token window, Gemini CLI does **not** rely on brute force: compression at 50% of limit, recent-30% preservation, tool-output **masking** and **distillation**, truncation thresholds (`getTruncateToolOutputThreshold`), a processor pipeline, and a `ContextWindowWillOverflow` event. The "Context Efficiency" prompt section explicitly coaches the model to combine turns, use parallel scoped grep/ranged reads, and minimize both turns and output size (with the comment that **unnecessary turns are more expensive than other wasted context**).

**Verification / completion.** System prompt enforces a **Research → Strategy → Execution** lifecycle with a per-subtask **Plan → Act → Validate** cycle; "Validation is the only path to finality," mandatory tests for bug fixes (reproduce first), and a "strategic re-evaluation" rule after 3 failed fix attempts. `complete_task` is the single forced finish in non-interactive/subagent runs. **Loop detection** (`services/loopDetectionService.ts`): heuristic thresholds (`TOOL_CALL_LOOP_THRESHOLD=5`, `CONTENT_LOOP_THRESHOLD=10`) plus periodic **LLM-based** loop checks over recent turns with confidence-adaptive intervals.

**Provider & model routing / auth.** `AuthType`: `LOGIN_WITH_GOOGLE` (oauth-personal, the free tier), `USE_GEMINI` (API key), `USE_VERTEX_AI`, `LEGACY_CLOUD_SHELL`, plus a `GATEWAY` and compute-ADC path; env detection in `contentGenerator.ts`. Free tier via Google Code Assist (`code_assist/`). Model precedence: `--model` > `GEMINI_MODEL` env > `model.name` setting > local Gemma router > default `auto`. **Model routing** (`routing/`, `ModelAvailabilityService`, `fallback/`): on quota/server failure, prompts (or silently for utility calls) falls back along a Flash-lite → Flash → Pro chain. **Local routing**: an experimental local **Gemma** model makes routing decisions to cut cost. **Token caching** auto-enabled for API-key/Vertex (not OAuth).

**Headless scripting / automation.** `docs/cli/headless.md` + `cli/src/nonInteractiveCli.ts`. Triggered by `-p/--prompt` or non-TTY. `--output-format json` (response + stats + error) or **streaming JSONL** events (`init`, `message`, `tool_use`, `tool_result`, `error`, `result` with per-model token breakdowns). Distinct exit codes: `0` ok, `1` error, `42` input error, `53` turn-limit exceeded. Non-interactive system-prompt branch tells the model not to ask the user questions.

**Extensibility (MCP, custom context).** Full **MCP** client (`tools/mcp-client*.ts`, `mcp-client-manager.ts`, compliance transport, `mcp-prompts.ts`, MCP resources tools) configured via `mcpServers` in `settings.json`; trusted-MCP priority in the policy engine. **Extensions** (`docs/extensions/`) package tools/commands/context. **Custom slash commands** (`docs/cli/custom-commands.md`). **Skills** (`skills/`, `activate_skill`, skill-extraction agent) — Anthropic-style progressive-disclosure skill files. **Hooks** (`hooks/`, `scheduler/hook-utils.ts`, before/after-agent and tool hooks) with a prompt section instructing the model to treat `<hook_context>` as read-only data, not instructions.

**UX surfaces.** Interactive Ink/React TUI (themes, notifications, `/memory`, `/restore`, `/rewind`, `/stats`, `/help`, `/bug`); non-interactive CLI; **VS Code companion** extension (workspace access, ACP mode); **ACP / A2A** remote-agent path (`a2a-server`, experimental — Agent-to-Agent protocol, HTTP server, persistence). External **GitHub Action** (`google-github-actions/run-gemini-cli`) for PR review, issue triage, and on-demand `@gemini` help. (The clone's own `.github/workflows/*.yml` like `gemini-automated-issue-triage.yml` are the project dogfooding the action.)

**Anything unique.** "**Topic model**" (`update_topic` tool, `topicTool.ts`) — narrate work as evolving topics instead of per-tool chatter, tied to a prompt mode. **Subagents** as compression: codebase-investigator, generalist, cli-help, skill-extraction (`agents/`), invoked by name or `@agent` syntax, each with isolated context/tools, collapsing to a single summary in the parent history. **Plan mode** writes a plan doc and gates execution behind approval, with an explicit "state transition override" prompt that lifts read-only constraints once a plan is approved.

## Standout / Unique

- **Cost-aware prompting that is benchmark-gated.** The system prompt has a dedicated, SWEBench-guarded "Context Efficiency" section that teaches the model the actual economics ("the agent passes full history each turn"; "unnecessary turns are more expensive than other wasted context") and pushes parallel scoped grep + ranged reads. This is the rare harness that treats cost-per-task as a prompt-engineering target, not an afterthought.
- **Layered, multi-strategy context management** beyond summary-at-95%: compression at 50% with recent-30% preservation, tool-output masking, tool-output distillation, truncation, and a processor pipeline — and rewind that reconstructs across compression points.
- **Search grounding + citations as a first-class built-in tool** with real grounding-metadata rendering — table stakes for a generalist, not bolted on.
- **Tiered TOML policy engine** with admin/user/workspace/extension priorities and fractional offsets — a genuine policy model rather than just per-prompt confirmation.
- **Subagents framed explicitly as a context-compression mechanism**, plus LLM-assisted loop detection with confidence-adaptive cadence.

## Pros

- Broad, generalist-ready surface: search grounding, multimodal, MCP, skills, hooks, subagents — coding is clearly "task type one," matching Glave's framing.
- Real engineering in the loop: typed event stream, dedicated scheduler with safe parallelism (`wait_for_previous`), policy engine, sandboxing (Seatbelt/Docker/Podman), checkpoint/rewind, loop detection.
- Cost levers are explicit: token caching, model-fallback routing, local Gemma router, and cost-aware prompting that is empirically benchmarked.
- Clean headless contract (JSON/JSONL events, meaningful exit codes, per-model token stats) — easy to script and to harvest traces.
- Strong free-tier on-ramp (OAuth, no API key) lowers adoption friction.

## Cons / Limitations

- **Local/client-owned state** (sessions, shadow-git, creds under `~/.gemini/`) — the trusted orchestrator is the user's machine, contrary to Glave's server-owned doctrine. The `a2a-server` server path is explicitly experimental.
- **Huge-window posture can mask weak management** (the README concern): a 1M window invites lazy context loading; the good compression machinery exists but the default still ships a lot of concatenated `GEMINI.md` + history each turn.
- **Heavy and sprawling.** TypeScript/Ink monorepo, nightly versioning, large core (`client.ts` 1.3k LOC, `geminiChat.ts` 1.5k, scheduler ~5k). Lots of surface to maintain, and feature-maximalist — the opposite of minimal.
- **Vendor lock-in to closed Gemini models**; free tier and token caching are tied to Google auth (token caching unavailable for OAuth). Routing/fallback chains are Gemini-specific.
- Permission UX still leans on interactive confirmation modals for the default mode; full autonomy is `yolo`.

## Lessons for Glave (econo-frontier, feature-parity lens)

- **Copy:**
  - The **cost-aware prompt discipline** and, more importantly, **gating prompt changes on a measured benchmark** (their SWEBench warning on the Context Efficiency section). This is exactly Glave's "decide harness design by fast measured evals, not vibes."
  - **`complete_task` as the single forced finish gate** in autonomous runs — a clean, typed submit boundary that pairs with a verification policy.
  - **Loop detection** (cheap heuristic thresholds + occasional LLM check) — directly protects cost-per-correct-task by killing runaway turns.
  - **Safe-parallel tool scheduling with an explicit `wait_for_previous`/dependency flag** — fewer turns is the dominant cost lever, and parallel batching is the deterministic way to get it.
  - **Headless JSONL event contract + per-model token stats + distinct exit codes** — ideal for trace harvesting and the flywheel.
- **Adapt:**
  - The **layered context pipeline** (compress / mask / distill / truncate) is the right shape, but make it a **server-owned, budgeted** context manager keyed on measured token cost, not a 50%-threshold default. Keep the rewind-across-compression idea.
  - The **tiered policy engine** is a good model; reimplement as **deterministic server policy** (typed Rust), not user-editable TOML that the client loads, and drop interactive confirmation as the primary mechanism.
  - **Subagents-as-context-compression** and **JIT context discovery** are strong primitives; in Glave terms, child task nodes with explicit budgets/inputs/outputs, and lazy context fetch instead of concatenating everything up front.
  - **Checkpointing via shadow git** is a clean artifact boundary — adopt the diff/rollback discipline, but anchor the source of truth server-side.
- **Skip:**
  - The Ink/React **TUI and the bulk of terminal UX** — not Glave's moat.
  - **Gemini-specific auth/routing/free-tier** plumbing and the closed-model lock-in.
  - Feature-maximalism for its own sake (themes, notifications, voice dir, IDE companion) — these dilute a minimal evaluable core.
  - Client as trusted orchestrator; local `~/.gemini/` as source of truth.
- **Cost angle:** Gemini CLI is the clearest cautionary-and-instructive case for Glave's cost axis. Its own README concern — "huge context can hide poor context management" — is the thesis: a 1M window tempts brute-force context, but the *good* parts of this harness (50%-threshold compression, output masking/distillation, subagent summary-collapse, the "unnecessary turns are the most expensive context" prompt rule, token caching, and a cheaper local router for routing decisions) are all **cost-per-correct-task** moves. Glave should take those compression/selection mechanisms and the benchmark-gated tuning, and reject the large-window posture as the default — prefer compressed, selected, budgeted context with cost measured per turn.

## Source

Clone root: `refs/harnesses/gemini-cli/` (snapshot `c82e2b5`). Key paths:
- Loop/engine: `packages/core/src/core/client.ts`, `geminiChat.ts`, `turn.ts`, `contentGenerator.ts`
- Scheduler/parallelism/policy: `packages/core/src/scheduler/{scheduler,policy,tool-executor,state-manager,confirmation}.ts`
- Tools: `packages/core/src/tools/` (`tool-names.ts`, `definitions/coreTools.ts`, `edit.ts`, `grep.ts`/`ripGrep.ts`, `shell.ts`, `web-search.ts`, `web-fetch.ts`, `complete-task.ts`, `enter-plan-mode.ts`, `memoryTool.ts`, `activate-skill.ts`, `jit-context.ts`)
- Prompts: `packages/core/src/prompts/{promptProvider,snippets}.ts`, `packages/core/src/core/prompts.ts`
- Context mgmt: `packages/core/src/context/` (`contextManager.ts`, `chatCompressionService.ts`, `toolOutputMaskingService.ts`, `toolDistillationService.ts`, `truncation.ts`, `pipeline.ts`)
- Permissions/safety: `packages/core/src/policy/`, `safety/`, `sandbox/`, `confirmation-bus/`
- Auth/routing: `packages/core/src/code_assist/`, `routing/`, `fallback/`, `availability/`
- MCP/extensibility: `packages/core/src/tools/mcp-*.ts`, `prompts/mcp-prompts.ts`, `hooks/`, `skills/`, `agents/` (subagents)
- Loop detection: `packages/core/src/services/loopDetectionService.ts`
- CLI / headless: `packages/cli/src/{gemini.tsx,nonInteractiveCli.ts}`; cloud/ACP: `packages/a2a-server/`, `packages/cli/src/acp/`; SDK: `packages/sdk/`; IDE: `packages/vscode-ide-companion/`
- Config schema: `schemas/settings.schema.json` (top-level keys incl. `mcpServers`, `policyPaths`, `model`, `agents`, `context`, `tools`, `mcp`, `security`, `hooks`, `contextManagement`, `skills`, `experimental`)
- Docs: `docs/cli/{gemini-md,checkpointing,rewind,headless,model-routing,session-management,sandbox,token-caching,plan-mode}.md`, `docs/core/{subagents,local-model-routing,remote-agents}.md`, `docs/tools/`, `docs/issue-and-pr-automation.md`

Upstream: https://github.com/google-gemini/gemini-cli · GitHub Action: https://github.com/google-github-actions/run-gemini-cli · Docs: https://www.geminicli.com/docs/
