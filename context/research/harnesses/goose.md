# Goose

**Identity:** A Rust-built, task-general open-source AI agent (desktop + CLI + API/server) under Linux Foundation / AAIF governance, distinguished by very broad provider support (30+ registered, incl. subscription-auth via ACP) and an MCP-centric extension ecosystem ("70+ extensions").
**Source:** https://github.com/block/goose · Apache-2.0 · Rust (core/CLI/server) + TypeScript/Electron (desktop UI) + minijinja prompt templates · snapshot `1cb5cb0`
**Family:** general-purpose (with strong terminal + desktop surfaces; explicitly *not* coding-specific)

## Overview

Goose markets itself as "your native open source AI agent — for code, workflows, and everything in between." The README is emphatic that it is **not just for code**: "use it for research, writing, automation, data analysis, or anything you need to get done." This matches Glave's "coding is one task type on the same engine" stance almost exactly — Goose's core loop, tool grammar, and extension model are task-agnostic, and "coding" is delivered as one built-in platform extension (`developer`) plus optional MCP servers, not as a hardcoded center of gravity.

It ships three surfaces over one Rust core: a native Electron **desktop** app (backed by a Rust server binary `goosed`), a full **CLI** (`crates/goose-cli`, binary `goose`), and an **HTTP/WS API** (`crates/goose-server`, axum). All three are thin actuators over the `goose` core crate; the desktop UI talks to the server via a generated OpenAPI client (`just generate-openapi`). There is also an **ACP server** mode (`crates/goose/src/acp/`) so Goose can act as an Agent-Client-Protocol agent for editors, and a **gateway** (`src/gateway/`, including a Telegram bridge) for remote chat front-ends.

Architecturally it is a mature, large codebase (the core `Agent` in `agents/agent.rs` alone is ~3000 lines; `extension_manager.rs` ~2700). It leans heavily on the official Rust MCP SDK (`rmcp`) for both consuming MCP servers and exposing built-in tools, and on `sqlx`/SQLite for session persistence. Recent additions (e.g. an LLM-graded permission "judge," an adversary/prompt-injection inspector, egress inspection, a "code mode" that lets the model write code to call tools, and a Theory-of-Mind extension) show it is actively chasing safety and tool-scaling problems that directly bear on Glave's tool-sprawl concern.

It is now part of the **Agentic AI Foundation (AAIF)** at the Linux Foundation (the repo has migrated `block/goose` → `aaif-goose/goose`), so it is "extension-rich local runtime under neutral foundation governance" — a useful neutral structural reference rather than a single-vendor product.

## Architecture

### Control loop

The heart is `Agent::reply` → `reply_internal` in `crates/goose/src/agents/agent.rs`. `reply` is a pre-pass that handles slash/command interception (`execute_command`), `UserPromptSubmit` hooks, elicitation responses, persists the user message via `SessionManager`, and runs **auto-compaction** *before* the turn loop if `check_if_compaction_needed` says context exceeds threshold. It returns a `BoxStream<Result<AgentEvent>>`; the whole agent is **streaming-first** (async-stream + tokio). `AgentEvent` is a small enum: `Message(Message)`, `McpNotification((String, ServerNotification))`, `HistoryReplaced(Conversation)`.

`reply_internal` builds a `ReplyContext` (conversation, tools, toolshim_tools, system_prompt, goose_mode, `tool_call_cut_off`, initial_messages), appends project instructions (goosehints/AGENTS.md) to the system prompt, then runs the turn loop:

1. Cancellation + `final_output_tool` short-circuit checks (structured completion — see below).
2. `turns_taken += 1`; bail if over `max_turns` (`SessionConfig.max_turns` → `GOOSE_MAX_TURNS` → `DEFAULT_MAX_TURNS`) with a "reached maximum number of actions… continue?" message.
3. `moim::inject_moim` — "message of the iteration moment": mid-loop conversation mutation/injection (e.g. new hints, fix-ups via `fix_conversation`).
4. `stream_response_from_provider` — provider call, streamed.
5. For each chunk: update session token/cost metrics; `categorize_tools` splits into **frontend** tool requests (executed by the client/UI) vs **remaining** (executed in-process/MCP); thinking/reasoning content is preserved and echoed back (explicit handling for Gemini/Kimi/DeepSeek requirements).
6. If `goose_mode == Chat`, all tool calls are skipped with a canned response. Otherwise tools go through the **inspection pipeline** (`tool_inspection_manager.inspect_tools`) producing approve / needs-approval / deny buckets; approved+denied run via `handle_approved_and_denied_tools`, needs-approval via `handle_approval_tool_requests` (elicitation/confirmation), then results stream back, interleaved with elicitation draining on a 100ms tick.
7. Tool results appended to conversation; loop continues until no tool calls, max turns, cancellation, or final-output.

`SuccessCheck`/`RetryConfig` + `RetryManager` (`agents/retry.rs`) give a **verification + retry** layer for recipes: after a run, shell `SuccessCheck`s are executed; on failure an optional `on_failure` cleanup command runs and the agent retries up to `max_retries` (with separate timeouts).

### Crate / workspace layout

`Cargo.toml` is a workspace (`crates/*` + vendored `v8`), edition 2021, version 1.36.0, rust 1.91.1. Crates:

- `crates/goose` — core: agent loop, providers, extensions, context mgmt, sessions, recipes, security, ACP, gateway, skills, hooks, scheduler.
- `crates/goose-cli` — CLI (`main.rs`, `cli.rs`); subcommands: `configure`, `session`, `run`, `recipe`, `schedule`, `project`, `plugin`, `skills`, `info`, `doctor`, `gateway`, `tui`, `update`, `review`.
- `crates/goose-server` — `goosed` axum server; routes for `agent`, `reply`, `session`, `recipe`, `schedule`, `config_management`, `action_required` (elicitation), `sampling`, `mcp_app_proxy`, `mcp_ui_proxy`, `dictation`, `tunnel`, `telemetry`.
- `crates/goose-mcp` — built-in MCP servers spawned as in-process duplex-stream services: `autovisualiser`, `computercontroller`, `memory`, `tutorial`, `peekaboo` (+ `BUILTIN_EXTENSIONS` registry).
- `crates/goose-acp-macros` — proc macros for ACP.
- `crates/goose-sdk`, `goose-test`, `goose-test-support`.

Feature flags are extensive: `local-inference` (candle + llama-cpp), `cuda`/`vulkan`, `aws-providers` (Bedrock/SageMaker), `otel` (OpenTelemetry), `code-mode` (pctx), `system-keyring`, `nostr` (session sharing over Nostr), `rustls-tls`/`native-tls`, `portable-default`.

### Extension / MCP system

Extensions are the universal capability unit. `ExtensionConfig` (`agents/extension.rs`) is a tagged enum: `Sse`, `Stdio`, `StreamableHttp`, `Builtin` (in-process MCP server from `goose-mcp`), and `Platform` (runs *in the agent process with direct access to agent state*). `ExtensionManager` (~2700 lines) manages lifecycle, tool listing/namespacing, malware checks (`extension_malware_check.rs`), and per-session enable/disable state persisted to the session.

**Platform extensions** (`agents/platform_extensions/`, registered in `PLATFORM_EXTENSIONS`) are the "first-party tools": `developer` (write/edit/shell/tree), `todo`, `analyze` (tree-sitter symbol/call-graph analysis), `summarize`, `summon`, `orchestrator` (multi-agent), `chatrecall`, `code_execution` (code-mode), `apps`, `tom` (theory-of-mind). MCP is consumed via `rmcp` clients (`agents/mcp_client.rs`) over child-process / streamable-http / unix-socket transports. Tools from MCP and platform extensions share the same `rmcp::model::Tool` shape with **annotations** (read-only / destructive / open-world), which the permission inspector reads.

Notably the **system prompt itself** (`prompts/system.md`) warns the model when too many extensions/tools are loaded ("exceeding recommended limits… Consider asking if they'd like to disable some extensions to improve tool selection accuracy") — an explicit acknowledgment of tool sprawl as an accuracy problem.

### Provider abstraction

`providers/base.rs` defines the `Provider` trait (`stream`, `complete`, `complete_fast`, `get_model_config`, `fetch_model_info`, `supports_embeddings`, `manages_own_context`, retry config, canonical-model mapping) and `ProviderDef` (metadata + `from_env`). `provider_registry.rs` + `init.rs` register **30+ providers**: Anthropic, OpenAI, Google, Ollama, OpenRouter, Azure, Bedrock, GCP Vertex, GitHub Copilot, xAI, Databricks (v1/v2), Snowflake, SageMaker, LiteLLM, NanoGPT, Tetrate, Avian, Kimi, local-inference (candle/llama.cpp), plus **ACP providers** (`claude_acp`, `codex_acp`, `copilot_acp`, `amp_acp`, `pi_acp`, `gemini_cli`, `cursor_agent`, `chatgpt_codex`, `claude_code`). Declarative providers can be added via config (`declarative_providers.rs`). A `ModelInfo` carries context limits + cost; `Usage`/`ProviderUsage` track tokens and cost per call.

### ACP-style subscription auth

The standout auth pattern: instead of API keys, Goose can drive a **locally-authenticated CLI of another vendor** via the Agent Client Protocol. E.g. `providers/claude_acp.rs` spawns the `claude-agent-acp` adapter binary, relying on the user's already-authenticated Claude CLI ("Ensure your Claude CLI is authenticated"), so a user pays via their **existing Claude/ChatGPT/Gemini subscription** rather than per-token API billing. These ACP providers `connect` over a child-process transport and translate ACP into Goose's internal message/tool shape. There is also OAuth/device-flow (`oauth.rs`, `oauth_device_flow.rs`, `gemini_oauth.rs`) and `keyring` secret storage.

### Where state lives

- **Sessions**: `session/session_manager.rs` uses **SQLite via sqlx** (not append-only JSONL). A `Session` holds conversation, working_dir, provider/model config, goose_mode, recipe, token/cost accumulators, project_id, archived_at. There's chat-history search, diagnostics, legacy migration, and `nostr_share`.
- **Config**: `config/` is layered (`base.rs`, env params like `GOOSE_PROVIDER`/`GOOSE_MODEL`/`GOOSE_MODE`/`GOOSE_MAX_TURNS`/`GOOSE_AUTO_COMPACT_THRESHOLD`), YAML at `~/.config/goose/config.yaml`, keyring secrets, migrations, experiments flags, per-tool permission store.
- **Permissions**: `permission/permission_store.rs` caches per-tool AlwaysAllow/AskBefore/NeverAllow decisions.

## Feature Inventory

**Project / context discovery (goosehints/recipes):**
- `.goosehints` files (`hints/load_hints.rs`, `GOOSE_HINTS_FILENAME`) plus **AGENTS.md** support; both global (`~/.config/goose/`) and local/project (walks to git root, multiple dirs). Hints are concatenated into the system prompt as "Global Hints" / local hints sections. Supports `@file` import expansion (`import_files.rs`) and gitignore-aware traversal.
- **Recipes** (`recipe/mod.rs`): reusable YAML workflows. `Recipe` = title/description, `instructions` and/or `prompt`, `extensions` to enable, `settings`, `parameters` (typed, with requirement/default), `response` (JSON-schema-constrained output), `sub_recipes`, and `retry`. Recipe deep-links, security scanning (`check_for_security_warnings`), local recipe discovery, templating (minijinja).
- **Skills** (`skills/`): `SKILL.md`-based skills per the agentskills.io spec (frontmatter + markdown), discovered from `~/.agents/skills` (global) and `<project>/.agents/skills` (project); exposed as a `skills` extension and as slash commands.

**Tool / action grammar:**
- Core `developer` tools are deliberately **flat and minimal**: `write`, `edit` (exact find/replace), `shell`, `tree` (a test asserts exactly these four). Tools carry annotations (title, read-only, destructive, idempotent, open-world) used for auto-permissioning.
- `shell` output capped at 2000 lines/stream with overflow spilled to temp files; structured `ShellOutput` (stdout/stderr fields).
- `analyze` adds tree-sitter directory overviews, file detail, and symbol call graphs across Go/Java/JS/TS/Python/Ruby/Rust/Swift/Kotlin.
- `todo` list tool; `final_output` tool for structured completion.

**Extension system & MCP marketplace:** see Architecture. Five config types (sse/stdio/streamable_http/builtin/platform), in-process builtins, malware checking, per-session enable state, runtime enable/disable via a `manage_extensions` tool, mcp-ui proxying for UI extensions.

**Provider abstraction (breadth) & ACP auth:** 30+ providers, declarative provider config, canonical-model mapping, `toolshim` (tool-calling emulation for models without native tool calls), local inference, OAuth/device flow, and the ACP subscription-reuse pattern (see above).

**Permissions & safety:**
- Four `GooseMode`s (`config/goose_mode.rs`): `Auto` (approve all), `Approve` (ask every tool), `SmartApprove` (LLM judge + annotations decide), `Chat` (no tools).
- `tool_inspection.rs` defines a pluggable `ToolInspector` trait + `ToolInspectionManager` (vec of inspectors) producing `InspectionAction::{Allow, RequireApproval, Deny}`.
- `permission/permission_inspector.rs` maps mode + cached permission level + read-only annotation → action; `permission_judge.rs` is an **LLM-based read-only detector** (`detect_read_only_tools`, prompt `permission_judge.md`) powering SmartApprove.
- **Security inspectors** (`security/`): `adversary_inspector` (opt-in via `~/.config/goose/adversary.md`, prompt-injection/abuse detection with a classification client), `egress_inspector` (extracts URLs/git remotes from shell commands to flag exfiltration destinations), `scanner.rs`/`patterns.rs` (regex-based finding patterns). `recipe-scanner/` is a separate crate for scanning shared recipes.
- `extension_malware_check.rs` validates extensions before load.

**Sessions & state / persistence:** SQLite-backed sessions with token/cost accumulation, working_dir, recipe, schedule_id, project grouping, archive, chat-history search, diagnostics, legacy migration, and optional Nostr-based session sharing.

**Context management & compaction:** `context_mgmt/mod.rs`. Auto-compaction at `DEFAULT_COMPACTION_THRESHOLD = 0.8` (configurable `GOOSE_AUTO_COMPACT_THRESHOLD`), triggered in `reply` before the turn; `compact_messages` summarizes (prompt `compaction.md`) and the session conversation is replaced (`HistoryReplaced` event). Token counting via tiktoken (`token_counter.rs`) or cached session token totals. Separately, **tool-pair summarization** (`maybe_summarize_tool_pairs`, `compute_tool_call_cutoff`, `tool_ids_to_summarize`) summarizes *old tool call/result pairs* mid-run once a `tool_call_cut_off` is exceeded — a finer-grained, cheaper compaction tier than full summarization. Providers that `manages_own_context()` skip compaction.

**Verification / completion:** `final_output_tool.rs` — recipes with a `response.json_schema` force the model to call `recipe__final_output` with schema-validated structured output; the loop short-circuits when final output is set. `SuccessCheck::Shell` + `RetryManager` run shell verification gates with retry/on_failure. `goose-self-test.yaml` is a recipe where Goose tests its own tools end-to-end (first-person integration test).

**UX surfaces:**
- **Desktop**: Electron/TS app over `goosed` (OpenAPI-generated client); also an Ink/React terminal UI (`ui/text`).
- **CLI**: `goose` with rich subcommands incl. interactive session, `run --recipe`, scheduling, projects, plugins, doctor, review.
- **API/server**: axum HTTP+WS, SSE event bus, action-required (elicitation) endpoints, sampling, tunnel for remote access.
- **ACP server**: Goose-as-ACP-agent for editors. **Gateway**: remote chat bridges incl. Telegram.

**Recipes / reusable workflows:** parameterized YAML recipes, sub-recipes (composition/delegation), JSON-schema-constrained responses, retry/success-checks, deep-links, security scanning, a recipe contribution flow (`CONTRIBUTING_RECIPES.md`, `workflow_recipes/`).

**Scheduling:** `scheduler.rs` + `schedule_tool.rs` — cron-style scheduled/triggered sessions (`SessionConfig.schedule_id`).

**Hooks:** `hooks/` + `HookManager` loaded from working dir. Events: `SessionStart/End`, `UserPromptSubmit`, `BeforeShellExecution`, `BeforeReadFile`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure` — with matcher context (tool name, command, path). Mirrors Claude Code hooks.

**Multi-agent / subagents:** `orchestrator` platform extension exposes `list_sessions`, `view_session` (incl. LLM `summarize` mode), `start_agent` (new session, own working dir, inherits provider/model), `send_message`, `interrupt_agent`. Plus `subagent_execution_tool/` + `subagent_handler.rs` + `TaskConfig` for delegated tasks (sync/async), with nested-delegation prevention.

**Anything unique:** `tom.rs` (Theory-of-Mind extension), `code_execution`/`code-mode` (model writes code that calls tools via a registry, reducing per-tool prompt overhead — a tool-sprawl mitigation), `moim` mid-loop injection, dictation (voice), `autovisualiser` (auto-charts), `computercontroller`/`peekaboo` (GUI/screen control), Nostr session sharing, custom distros (`CUSTOM_DISTROS.md` — rebrand + preconfigure providers/extensions).

## Standout / Unique

- **Provider breadth + ACP subscription reuse.** 30+ providers and, crucially, the ACP pattern that lets Goose ride a user's *existing Claude/ChatGPT/Gemini/Copilot subscription* by spawning that vendor's locally-authenticated CLI as an ACP agent. This decouples "which model" from "how it's billed" and is a genuine cost lever.
- **Genuinely task-general core.** Coding is just the `developer` platform extension; the loop, tools, permissions, and recipes have no coding bias. Recipes/skills/extensions let it be a research/writing/automation agent on the same engine.
- **Layered, cheap-first context management.** Tool-pair summarization (cut off and summarize *old tool I/O* mid-run) is a finer, cheaper tier than full-conversation compaction at 0.8 — two tiers before you pay for a big summary.
- **Pluggable safety inspector pipeline** with an LLM permission "judge," opt-in adversary/prompt-injection detection, and egress (exfiltration) inspection on shell commands — safety treated as a composable inspector chain feeding a typed `InspectionAction`.
- **Explicit tool-sprawl awareness:** the system prompt itself nudges the model/user to disable extensions past recommended limits, and `code-mode` exists specifically to compress tool surface.

## Pros

- Single Rust core, three thin surfaces (CLI/desktop/server) + ACP + gateway — clean actuator/engine separation Glave wants.
- Streaming-first agent loop with a small `AgentEvent` enum and typed `ReplyContext` — good structural reference.
- Typed permission model (`GooseMode` + `InspectionAction` + annotations + per-tool store) decided **before** execution, not improvised at shell time.
- Verification primitives exist (success checks, retry/on_failure, schema-constrained final output, self-test recipe).
- Token/cost accounting per call accumulated on sessions; SQLite persistence is queryable.
- MCP done via the official `rmcp` SDK with multiple transports; built-ins are in-process MCP servers (uniform shape).
- Neutral Linux-Foundation/AAIF governance; Apache-2.0; documented custom-distro path.

## Cons / Limitations

- **Tool/feature sprawl is real and self-acknowledged.** 30+ providers, 10+ platform extensions, 70+ MCP extensions, gateways, dictation, GUI control, autovisualiser — the system prompt literally warns about too many tools hurting accuracy. This is exactly the failure mode the Glave README flags.
- **Massive surface area.** `agent.rs` ~3000 lines and `extension_manager.rs` ~2700 lines; the loop interleaves compaction, hooks, frontend tools, elicitation, multi-provider thinking-content quirks — hard to test deterministically as one unit.
- **Local-first trust model.** State (sessions, permission cache, hooks, secrets) lives on the client/local machine; hooks are user shell scripts. Not a server-owned trust boundary.
- **Provider-specific leakage in the loop.** Special-casing for Gemini/Kimi/DeepSeek thinking/reasoning echo lives in the core reply loop, not fully isolated behind the adapter boundary.
- **Compaction is threshold/summary-based**, not a budgeted retrieval system; repo-map-style structural context is `analyze` (a tool the model must choose to call), not automatic packing.
- Eval/measurement story (`evals/open-model-gym/`) is benchmarking-oriented, not a per-run cost-per-correct trace contract.

## Lessons for Glave (econo-frontier, feature-parity lens)

- **Copy:**
  - The **ACP subscription-reuse** pattern as a provider adapter type — letting users bring an existing Claude/ChatGPT subscription is a direct $/correct lever and a differentiator; model it as one `ProviderKind` terminating in Glave's internal message/action shape.
  - **Two-tier context management**: cheap mid-run tool-pair summarization (summarize old tool I/O past a cutoff) *before* full-conversation compaction. Glave's "context as budgeted subsystem" should have this tier, not just a 0.8 summary.
  - **Typed pre-execution permission decision**: `GooseMode` + tool annotations (read-only/destructive) + cached per-tool decisions + `InspectionAction{Allow,RequireApproval,Deny}` is a clean, gateable model — map directly onto Glave's RunSpec `allowed_actions`.
  - **Schema-constrained final output as a typed completion** (`final_output` + JSON schema) — this *is* the `submit`/`finalize`-with-evidence pattern the README calls for.
- **Adapt:**
  - The **inspector pipeline** (`ToolInspector` trait → typed action) is a good shape for Glave's safety policy, but make it **server-owned deterministic policy** rather than optional local inspectors + user shell hooks. Keep the LLM "judge" as an *advisory* inspector, never the boundary.
  - **Recipes** → Glave's typed `RunSpec`/reusable workflow, but compile to typed run specs (params, allowed extensions, success checks, response schema) instead of prompt macros.
  - **Hooks** → keep the *event taxonomy* (UserPromptSubmit/BeforeShell/BeforeRead/PostTool) for trace + policy hooks, but evaluate them as server policy, not user scripts.
- **Skip (for V0):**
  - Breadth-first provider count, GUI control (peekaboo/computercontroller), autovisualiser, dictation, Nostr sharing, Telegram gateway, custom distros — all product polish that the README explicitly says comes "only after the core is measured."
  - The desktop Electron app and the huge platform-extension catalog. Start with the `developer`-equivalent flat tool set (write/edit/shell/tree + search) only.
  - In-process "platform" extensions with direct agent-state access — that's the opposite of a typed action boundary; keep tools behind the action grammar.
- **Cost angle:** ACP subscription reuse is the headline cost move. Beyond that: the two-tier compaction + per-call `Usage`/cost accumulation on sessions is a baseline; but Goose measures tokens for compaction, **not** cost-per-correct as a first-class run contract — Glave should make that the trace primitive Goose lacks. `complete_fast` (a cheaper model path) and `code-mode` (compressing many tools into one code-execution tool to cut prompt tokens) are both concrete token-reduction tactics worth stealing.
- **Rust-architecture notes:** Worth borrowing structurally: (1) one core crate, thin CLI/server/ACP surfaces over it; (2) streaming agent as `BoxStream<Result<AgentEvent>>` with a *small* event enum; (3) a typed `ReplyContext` assembled once per turn; (4) `Provider` trait with `stream`/`complete`/`complete_fast`/`get_model_config`/`manages_own_context` + a registry of `ProviderDef::from_env`; (5) `ToolInspector` trait vector → typed action; (6) tools as a uniform `Tool` shape with annotations regardless of source (builtin/MCP). **Avoid**: a 3000-line agent file and provider-specific quirks bleeding into the loop — keep adapters fully isolating provider semantics.
- **Task-general notes:** Goose validates Glave's thesis directly — its core loop has zero coding bias; "coding" is one extension (`developer`) and one recipe family. The lesson: define a generic `TaskType` with coding as the *first implemented* task (the README's own recommendation), expose capabilities *selectively per task type / trust tier / budget / eval evidence* rather than loading everything, and treat recipes/skills as the task-specialization layer over a neutral kernel. Goose's tool-sprawl warning is the cautionary tail: generality without selective exposure degrades tool-selection accuracy.

## Source

Key paths in the clone (`refs/harnesses/goose/`):
- Core loop: `crates/goose/src/agents/agent.rs` (`reply`, `reply_internal`, `AgentEvent`, `ReplyContext`)
- Mod map: `crates/goose/src/agents/mod.rs`; types: `agents/types.rs`
- Extensions: `agents/extension.rs` (`ExtensionConfig`), `agents/extension_manager.rs`, `agents/platform_extensions/` (developer/orchestrator/todo/analyze/code_execution/tom/...), `crates/goose-mcp/src/lib.rs` (`BUILTIN_EXTENSIONS`)
- Developer tools: `agents/platform_extensions/developer/{mod,edit,shell,tree}.rs`
- Providers: `providers/base.rs` (`Provider` trait), `providers/init.rs` + `provider_registry.rs` (registration), ACP: `providers/claude_acp.rs`, `codex_acp.rs`, `copilot_acp.rs`, etc.; `providers/toolshim.rs`
- Permissions/safety: `tool_inspection.rs`, `permission/permission_inspector.rs`, `permission/permission_judge.rs`, `config/goose_mode.rs`, `security/{adversary_inspector,egress_inspector,scanner,patterns}.rs`
- Context mgmt: `context_mgmt/mod.rs` (`check_if_compaction_needed`, `compact_messages`, `maybe_summarize_tool_pairs`, `compute_tool_call_cutoff`)
- Sessions: `session/session_manager.rs` (SQLite/sqlx)
- Recipes/skills: `recipe/mod.rs`, `skills/mod.rs`, `slash_commands/`, `agents/final_output_tool.rs`, `agents/retry.rs`
- Hooks: `hooks/mod.rs`; Scheduler: `scheduler.rs`
- Prompts: `crates/goose/src/prompts/{system,compaction,permission_judge,subagent_system,plan}.md`
- Hints/recipes config: `hints/load_hints.rs` (`.goosehints`/AGENTS.md), top-level `.goosehints`, `goose-self-test.yaml`
- Surfaces: `crates/goose-cli/src/{main,cli}.rs` + `commands/`, `crates/goose-server/src/routes/`, `crates/goose/src/acp/`, `crates/goose/src/gateway/`
- Docs: `README.md`, `AGENTS.md`, `CUSTOM_DISTROS.md`, `CONTRIBUTING_RECIPES.md`

Upstream: https://github.com/block/goose (migrating to https://github.com/aaif-goose/goose) · docs https://goose-docs.ai · ACP providers https://goose-docs.ai/docs/guides/acp-providers · MCP https://modelcontextprotocol.io · snapshot `1cb5cb0`.
