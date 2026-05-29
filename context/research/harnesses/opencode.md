# OpenCode

**Identity:** A Go terminal coding agent with a Bubble Tea TUI, multi-provider model routing, SQLite-backed sessions, and LSP-driven deterministic feedback — minimal control loop, batteries-included tool surface.
**Source:** https://github.com/opencode-ai/opencode (Kujtim Hoxha) · MIT (`LICENSE`: "Copyright (c) 2025 Kujtim Hoxha") · **Go** (`go.mod`: `module github.com/opencode-ai/opencode`, `go 1.24.0`; entry `main.go` → `cmd.Execute()`; no `package.json`) · snapshot `73ee493`
**Family:** terminal (developer-terminal agent)

> Disambiguation: there are two projects named "opencode." This clone is the **Go** `opencode-ai/opencode` (the original by Kujtim Hoxha), confirmed by `go.mod` module path, `go.sum`, `.goreleaser.yml`, `main.go`, and `git remote -v` → `https://github.com/opencode-ai/opencode.git`. It is **not** the later TypeScript SST/`opencode` rewrite. Evidence is unambiguous: there is no `package.json`/`tsconfig` anywhere in the tree, the whole codebase is `internal/*.go`, and the build is GoReleaser-driven.

## Overview

OpenCode is a self-described "early development" terminal coding assistant written entirely in Go. It pairs a Charm/Bubble Tea TUI with a compact agent loop and a fixed, hand-curated tool surface aimed at the classic coding workbench: search (glob/grep/ls), read (view), write/edit/patch, run (bash), web fetch, public-code search (Sourcegraph), LSP diagnostics, and a single read-only sub-agent ("agent" tool). The README markets it as "AI coding agent, built for the terminal."

The product's distinguishing bet is **LSP integration as cheap deterministic feedback**: language servers run in the background, and the `view`, `edit`, `write`, `patch`, and `diagnostics` tools all surface compiler/linter diagnostics inline (in `<file_diagnostics>` / `<project_diagnostics>` tags) right after a file is read or modified. The system prompt explicitly tells the model these diagnostics will appear automatically and to "take necessary actions to fix the issues." This gives the loop a non-model signal about correctness without spending a model turn to run a build.

State is real and persistent: a SQLite database (sessions, messages, files) lives under `.opencode/` with append-oriented message storage and full file-version history for change tracking. Multi-provider routing is first-class — Anthropic, OpenAI, Gemini, Groq, OpenRouter, xAI, Azure, AWS Bedrock, GCP Vertex, and GitHub Copilot — selected automatically by which API keys/credentials are present, with per-role agents (coder / task / title / summarizer) each independently model-configurable.

Architecturally it is a **local-first, single-process app**: the TUI, agent loop, tools, LSP clients, and DB all run in one Go binary on the developer's machine. There is no server boundary; local CLI state is the source of truth. It also supports a **non-interactive mode** (`opencode -p "..." -f json -q`) for scripting/piping, which is the closest thing to a programmatic harness surface.

## Architecture

**Control loop.** The core is `internal/llm/agent/agent.go`. `Run()` spawns a goroutine; `processGeneration()` runs the canonical agentic `for {}` loop: build message history → `streamAndHandleEvents()` streams provider output and executes any tool calls → if `FinishReason == ToolUse`, append assistant message + tool results and continue; otherwise return. Tool execution is sequential within a turn (`for i, toolCall := range toolCalls`), each dispatched by name-match against the registered `[]tools.BaseTool`. Cancellation is cooperative via `context.Context` checked before each iteration and before each tool call, with an `activeRequests sync.Map` of cancel funcs keyed by session ID (plus a `<id>-summarize` key). The loop is mostly **model-driven** (the model decides what to do next); the only deterministic interjections are LSP diagnostics appended to tool output and the auto-compaction trigger.

**Key components (all under `internal/`):**
- `app/app.go` — composition root: wires `Sessions`, `Messages`, `History`, `Permissions`, `LSPClients`, and the `CoderAgent`. LSP clients init in the background (`go app.initLSPClients`).
- `llm/agent/` — agent service, tool registries (`tools.go`), the `agent` sub-task tool (`agent-tool.go`), MCP tool wrapper (`mcp-tools.go`).
- `llm/tools/` — the built-in tool implementations.
- `llm/provider/` — per-provider streaming clients behind a `Provider` interface with a normalized `ProviderEvent` stream (`EventContentDelta`, `EventToolUseStart/Stop`, `EventThinkingDelta`, `EventComplete`, `EventError`, …).
- `llm/models/` — static model catalog with per-model `ContextWindow`, `DefaultMaxTokens`, cost-per-1M (in/out/cached) and capability flags (`CanReason`, `SupportsAttachments`).
- `llm/prompt/` — role prompts (coder/task/title/summarizer) + project-context loader.
- `lsp/` — LSP client, transport, protocol types, and a workspace file watcher.
- `db/`, `session/`, `history/`, `message/` — SQLite persistence via `sqlc`-generated code.
- `config/` — Viper-based JSON config + env.
- `tui/` — Bubble Tea UI (pages, dialogs, components, themes).
- `permission/`, `pubsub/`, `diff/` — policy, an in-process event broker, and diff rendering.

**TUI.** Built on Bubble Tea (`internal/tui/`), with pages and dialogs for chat, sessions, model selection, theme, permission prompts, custom commands, and logs. The status bar (`components/core/status.go`) renders live token usage as a percentage of the model context window and running session cost. Auto-compaction is wired in the TUI update loop (`tui.go`): after a response completes, it computes `tokens >= ContextWindow*0.95` and, if `AutoCompact` is on, fires `startCompactSessionMsg`.

**LSP.** `lsp.NewClient` spawns a language server per configured language (e.g. `gopls`), registers a `textDocument/publishDiagnostics` notification handler, opens files, and maintains a diagnostics map. The `diagnostics.go` helper `getDiagnostics()` formats results by severity (Error/Warn/Hint), sorts errors first, caps at 10 per scope, and emits `<file_diagnostics>`/`<project_diagnostics>`/`<diagnostic_summary>` blocks. `waitForLspDiagnostics()` blocks up to 5s for fresh diagnostics after a change. A workspace watcher (`lsp/watcher/`) keeps the server's view of the tree current.

**SQLite state / where state lives.** Persistence is a SQLite DB under the `.opencode` data directory (`config.go` `defaultDataDirectory = ".opencode"`). Schema (`db/migrations/20250424200609_initial.sql`): `sessions` (id, parent_session_id, title, message_count, prompt_tokens, completion_tokens, cost, timestamps), `messages` (role, `parts` JSON, model, finished_at), and `files` (path/content/version, UNIQUE(path, session_id, version)). DB triggers maintain `message_count` and `updated_at`. A later migration adds `summary_message_id` to support compaction. Cost and token usage are accumulated **per session in the DB** by `TrackUsage()` after every `EventComplete`.

**Deterministic vs model-driven.** Deterministic: LSP diagnostics injection, the 95%-context auto-compact trigger, the bash banned-command list, file-version history, and per-session cost accounting. Model-driven: essentially everything else — tool selection, ordering, when to verify, when to stop. There is no programmatic verification/submit gate.

## Feature Inventory

### Project / context discovery
- **Context files** (`config.go` `defaultContextPaths`, loaded in `prompt/prompt.go`): on startup the coder and task agents glob a fixed list and concatenate matches into a `# Project-Specific Context` block: `.github/copilot-instructions.md`, `.cursorrules`, `.cursor/rules/`, `CLAUDE.md`, `CLAUDE.local.md`, and the `opencode.md`/`OpenCode.md`/`OPENCODE.md` (+ `.local`) variants. Loaded once via `sync.Once`, deduped case-insensitively, walked concurrently. This is broad cross-tool compatibility (it deliberately reads competitors' instruction files).
- **Environment block** (`prompt/coder.go` `getEnvironmentInfo`): injects cwd, is-git-repo, OS platform, date, and a `ls .` listing of the project into the system prompt.
- **Memory convention**: the Anthropic coder prompt tells the model that `OpenCode.md` is auto-loaded and to ask the user before recording build/test/lint commands and style preferences there.

### Tool / action grammar (built-in tools)
Registered in `agent/tools.go` `CoderAgentTools`:
- `bash` (`bash.go`) — persistent shell session; `MaxOutputLength = 30000` truncation; timeout up to 600000ms (default 30min); a hard-coded `bannedCommands` list (`curl`, `wget`, `nc`, `telnet`, browsers, etc. — prompt-injection / exfiltration guard) and a `safeReadOnlyCommands` allowlist; prompt steers the model away from `find`/`grep`/`cat` toward the dedicated tools; long embedded git-commit / gh-PR playbooks.
- `edit` (`edit.go`) — exact `old_string`→`new_string` single-occurrence replace; create (empty old) / delete (empty new) modes; requires permission; writes a file-history version; **appends LSP diagnostics** after edit.
- `write` (`write.go`) — full-file overwrite; permission + history version + diagnostics.
- `patch` (`patch.go`) — multi-file coordinated patch in a `*** Begin Patch` envelope (Codex-style apply_patch grammar); permission + history + LSP.
- `view` (`view.go`) — ranged file read; opens file in LSP and **appends diagnostics**.
- `glob` (`glob.go`) — fast file globbing.
- `grep` (`grep.go`) — content search.
- `ls` (`ls.go`) — directory listing.
- `fetch` (`fetch.go`) — HTTP fetch with `format` ∈ {text, markdown, html} (uses `html-to-markdown`); permission-gated.
- `sourcegraph` (`sourcegraph.go`) — search **public** code across repos via Sourcegraph's GraphQL API (cross-repo discovery without cloning).
- `diagnostics` (`diagnostics.go`) — on-demand LSP diagnostics for a file or whole project.
- `agent` (`agent-tool.go`) — the sub-task tool (below).
Tool contract (`tools/tools.go`): `BaseTool` = `Info() ToolInfo` + `Run(ctx, ToolCall) (ToolResponse, error)`; responses carry `Content`, optional JSON `Metadata`, and `IsError`. Parameters are hand-written JSON-schema-ish maps, not generated from typed structs.

### The `agent` subtask tool
A single, **read-only** sub-agent (`agent-tool.go`). Spawns a fresh `AgentTask` agent with only `glob, grep, ls, sourcegraph, view` (`TaskAgentTools`) — explicitly **cannot** bash/edit/write. Each invocation is stateless, runs in its own child session (`CreateTaskSession` with `parent_session_id`), returns one final message, and **rolls its cost up into the parent session**. The description encourages concurrent launches for search to "reduce context usage." This is a context-isolation/search-offload primitive, not a general delegation system.

### LSP diagnostics integration
The standout feature. Diagnostics are surfaced (a) automatically inline after `view`/`edit`/`write`/`patch`, and (b) on demand via the `diagnostics` tool. Severity-grouped, errors-first, capped at 10 per scope with a summary line. The prompt's `# LSP Information` section instructs the model to fix issues it caused and ignore unrelated diagnostics. Servers are configured per language in `.opencode.json` (the clone configures `gopls`).

### Permissions & safety
- `permission/permission.go`: every mutating/external tool (`bash`, `edit`, `write`, `patch`, `fetch`, and **every MCP tool**) calls `permissions.Request(...)`, which blocks on a channel until the TUI dialog answers. Decisions: `Grant` (once), `GrantPersistant` (remembered for the session by tool+action+path), `Deny`. `AutoApproveSession` whitelists a whole session (used by non-interactive mode and the `--dangerously-...`/yolo path).
- Denial propagates: in the agent loop, a `ErrorPermissionDenied` marks the current tool result as error and cancels all subsequent tool calls in that turn (`FinishReasonPermissionDenied`).
- `bash` adds defense-in-depth via banned-command list + output truncation + timeouts.
- There is **no sandbox**: tools run with the user's full local privileges; safety is prompt + per-call approval, not isolation. Read-only search tools (glob/grep/ls/view/sourcegraph) require no permission.

### Sessions & state / SQLite persistence
SQLite under `.opencode/` (see Architecture). Sessions form a tree via `parent_session_id` (sub-agent sessions are children). Messages stored append-style with a JSON `parts` column; file versions stored in `files`. Cost + token counts persisted per session and shown live in the TUI. Title generation is a separate cheap-model agent run async on first message.

### File-change tracking
`internal/history/file.go` — every edit/write/patch records a versioned snapshot (`InitialVersion` = "initial", then `CreateVersion`) keyed by `(path, session_id, version)`. This gives a per-session audit trail of file states (the basis for diffs and potential rollback). Note: the repo's own diffing/review surface is git-agnostic at the storage layer — it tracks content versions in SQLite rather than relying on git.

### Context management & auto-compaction (the 95% threshold)
`AutoCompact` defaults **on** (`config.go` `setDefaults`). After each completed response the TUI checks `tokens >= ContextWindow*0.95` and triggers `Summarize()` (`agent.go`). Summarization runs a dedicated `summarizer` agent over the full history with a fixed prompt ("detailed but concise summary … what we did, what we're doing, which files, what's next"), stores the summary as an assistant message, and sets `session.SummaryMessageID`. On the next turn the loop slices history from the summary message forward (and relabels it `User`), effectively compacting. It is a **last-minute, single-shot summary**, not a continuous budgeted context manager. `maxTokens` is also clamped to ≤ half the context window during validation.

### Custom commands
`tui/components/dialog/custom_commands.go`: Markdown files become slash-style commands, loaded from `$XDG_CONFIG_HOME/opencode/commands`, `~/.opencode/commands` (prefix `user:`), and the project data dir `.opencode/commands` (prefix `project:`). Nested dirs become `:`-namespaced IDs. Files support **named arguments** via `$UPPER_CASE` placeholders; the TUI prompts for each via a multi-argument dialog, then substitutes and runs the resulting prompt. They are raw prompt macros — no typing or validation beyond placeholder substitution.

### Verification / completion
There is **no deterministic verification or submit gate**. Completion = the model stops emitting tool calls (`FinishReason != ToolUse`). Verification is purely advisory in the prompt: "run the lint and typecheck commands … if they were provided," "verify the solution if possible with tests." LSP diagnostics are the only built-in non-model signal. No regression-gate, no test-must-pass requirement.

### Provider & model routing (multi-provider)
`config.go` `setProviderDefaults` auto-selects a provider by credential presence, in a fixed priority order: Copilot → Anthropic → OpenAI → Gemini → Groq → OpenRouter → xAI → Bedrock → Azure → Vertex. Four role agents (coder/summarizer/task/title) each get a model; `task`/`title` default to cheaper models (e.g. GPT-4.1-mini, Gemini Flash, Claude Haiku) — an explicit cost lever. Reasoning effort (low/medium/high) is plumbed for OpenAI/local reasoning models; Anthropic "thinking" enabled for the coder agent. Model catalog carries cost-per-1M fields used for live cost accounting. `Update()` lets the user hot-swap the coder model (refuses while busy).

### Extensibility (MCP, custom commands)
- **MCP** (`agent/mcp-tools.go`): supports `stdio` and `sse` MCP servers from config (`mcpServers`). Tools are discovered via `ListTools`, namespaced `<server>_<tool>`, and exposed as ordinary `BaseTool`s. **Every MCP tool call is permission-gated** with `action: execute` and the raw params shown to the user. MCP tool list is cached after first discovery.
- **Custom commands** (above).
- **LSP servers**, **providers**, **themes** are all config-driven.
- No hooks system, no skills system, no plugin API beyond MCP/commands.

### UX surfaces
Primary surface is the Bubble Tea TUI (chat, session/model/theme/permission/command dialogs, logs page, live token%/cost status bar, themes incl. an `opencode` default). Secondary surface is **non-interactive CLI** (`-p/--prompt`, `-f/--output-format text|json`, `-q/--quiet`, `-c/--cwd`, `-d/--debug`) via `app.RunNonInteractive`, plus a JSON-schema generator (`cmd/schema`) for the config.

### Anything unique
- Reads competitors' context files (Copilot/Cursor/Claude) for drop-in compatibility.
- Sourcegraph public-code search as a first-class tool.
- Cost accounting baked into core state (per session, per sub-agent, rolled up), not bolted on.
- `pubsub` in-process broker decouples agent events from the TUI (clean event-sourced UI).

## Standout / Unique

**LSP diagnostics as cheap, deterministic feedback** is the headline. Instead of asking the model to run a build or trusting it to "look correct," OpenCode pipes real compiler/linter output back into the tool result the instant a file is read or written, formatted and capped, with prompt guidance to fix self-caused errors and ignore the rest. This is the cheapest possible correctness signal in the loop: zero extra model turns, no bash invocation, milliseconds of latency (5s bounded wait). It directly attacks the failure mode where an agent confidently writes broken code and only discovers it turns later. Combined with per-session cost tracking and cheaper models for ancillary roles, OpenCode shows a coherent (if shallow) cost-consciousness throughout.

Secondary standouts: the read-only search sub-agent as a deliberate context-budget tool (not a general delegate), versioned file history in SQLite as a storage-level change ledger, and credential-presence-driven provider auto-routing with role-specialized cheap models.

## Pros
- LSP diagnostics give a real, deterministic, near-free correctness signal mid-loop.
- Cost + token usage is a first-class, persisted concept (per session and per sub-agent), exposed live.
- Minimal, legible single-binary Go architecture; the agent loop fits in one readable file.
- Broad multi-provider support with sensible auto-selection and per-role model assignment (cost lever).
- Sub-agent is deliberately scoped (read-only, stateless, cost-rolled-up) — a clean context-isolation primitive.
- Permission model uniformly gates mutating + MCP tools; denial cleanly aborts the rest of a turn.
- File-version history + diff rendering give human-reviewable change tracking independent of git.
- Cross-tool context-file compatibility lowers adoption friction.
- Honest non-interactive mode with text/JSON output for scripting.

## Cons / Limitations
- **No sandbox / isolation** — tools run with full local user privileges; safety is prompt + per-call approval. Bad fit for untrusted code/images.
- **No deterministic verification or submit gate** — "done" = model stops; no enforced test/lint pass. LSP is advisory.
- **Local CLI is the source of truth** — no server boundary, no audit/budget enforcement layer; client orchestrates everything.
- **Compaction is a crude 95% last-minute single summary** — no continuous context budgeting, retrieval, or measured eviction; one summarizer model call.
- Tool schemas are hand-written `map[string]any`, not generated from typed structs — weak typing, drift risk.
- Permissions are coarse (tool+action+path string match); no fine-grained policy engine.
- Sub-agent is search-only and stateless — no general delegation, budgets, or structured I/O contracts.
- Self-described early/not-production; some monkey-patches (e.g. Copilot Sonnet tool-name repetition) hint at provider fragility.
- MCP tool cache + global state make multi-session isolation imperfect.

## Lessons for Glave (econo-frontier, feature-parity lens)

- **Copy:** The **LSP-diagnostics-as-deterministic-feedback** pattern — surface compiler/linter results inline in the tool result immediately after any read/edit/write, severity-sorted and capped, with a bounded wait. This is the single highest-leverage idea here for `$/correct`: a near-zero-cost non-model signal that catches breakage before the model burns turns "verifying." In Glave terms, make it a typed action post-condition, not advisory prose. Also copy **first-class per-task cost/token accounting** (here it's already in the session/sub-agent model) and **role-specialized cheap models** for title/summary/search sub-tasks.
- **Adapt:** The **read-only search sub-agent** → Glave's child task node, but with explicit budgets, typed inputs/outputs, and a verification contract (OpenCode's version is untyped and stateless-only). **Custom commands** → compile Markdown/`$ARG` macros into typed run specs rather than raw prompt strings. **Context-file discovery** → keep the multi-file project-instruction primitive, but load it through the typed server harness, deduped and budgeted, not via a one-shot `sync.Once` glob. **File-version history in a store** → good idea, but Glave should make the artifact boundary explicit (git or a typed diff ledger) and server-owned.
- **Skip:** The **Bubble Tea TUI** and terminal-first UX (not Glave's moat; clients are actuators). The **95%-last-minute single-summary auto-compaction** — replace with a measured, continuously-budgeted context manager + retrieval, decided by evals not a fixed percentage. **Banned-command lists as security** — they are prompt-injection hygiene, not a boundary; Glave's SWE-Pro constraint demands real runtime isolation (no-net, drop-caps, ephemeral/disposable VM). **Local-CLI-as-source-of-truth** and **per-call permission popups** — Glave's server must own policy, audit, and budget regardless of where execution happens; popups can't stand in for a policy model (and zero-HIL is a hard constraint). **Hand-written `map[string]any` tool schemas** — Glave wants typed (Rust) tool/action definitions with validation.
- **Cost angle:** LSP diagnostics are the clearest `$/correct` win in the whole harness — they convert "model writes broken code → discovers it 3 turns later → re-reads → re-edits" (4+ wasted model turns) into a single corrective signal delivered free with the edit's own tool result. OpenCode also demonstrates the cheaper-model-for-ancillary-roles lever (title/summarize/search on Haiku/Flash/mini) and persists cost per task/sub-task so it can actually be optimized. Glave should make the diagnostic/verification signal mandatory and deterministic (a typed post-condition gate), since OpenCode leaves it advisory and thereby leaves the headline cost saving partly on the table.

## Source

Key paths in the clone (`refs/harnesses/opencode/`):
- Agent loop: `internal/llm/agent/agent.go`; tool registries `internal/llm/agent/tools.go`; sub-agent `internal/llm/agent/agent-tool.go`; MCP wrapper `internal/llm/agent/mcp-tools.go`.
- Tools: `internal/llm/tools/{bash,edit,write,patch,view,glob,grep,ls,fetch,sourcegraph,diagnostics}.go`; contract `internal/llm/tools/tools.go`.
- LSP: `internal/lsp/client.go`, `internal/lsp/watcher/watcher.go`; diagnostics formatting in `internal/llm/tools/diagnostics.go`.
- Prompts: `internal/llm/prompt/{coder,prompt,summarizer,task,title}.go`.
- Config / providers / models: `internal/config/config.go`; `internal/llm/provider/*.go`; `internal/llm/models/*.go`.
- State: `internal/db/migrations/20250424200609_initial.sql`, `internal/session/session.go`, `internal/history/file.go`.
- Permissions: `internal/permission/permission.go`.
- Auto-compaction trigger: `internal/tui/tui.go` (`tokens >= ContextWindow*0.95`); status/cost `internal/tui/components/core/status.go`.
- Custom commands: `internal/tui/components/dialog/custom_commands.go`.
- CLI / non-interactive: `cmd/root.go` (`-p/-f/-q/-c/-d`), `internal/app/app.go`, `main.go`.
- Manifests confirming Go: `go.mod`, `go.sum`, `.goreleaser.yml`, `opencode-schema.json`, `.opencode.json`.

Upstream: https://github.com/opencode-ai/opencode · License `LICENSE` (MIT, © 2025 Kujtim Hoxha) · snapshot `73ee493` (per `refs/harnesses/README.md`).
