# Cline

**Identity:** Open-source, human-in-the-loop coding agent that lives in your IDE and terminal, with a shared agent engine powering VS Code, JetBrains, a CLI, an SDK, and a Kanban multi-agent board.
**Source:** https://github.com/cline/cline · Apache-2.0 · TypeScript (Node) — extension host + browser-safe runtime; React webview; gRPC/protobuf transport · snapshot `38f1c7e`
**Family:** IDE (with a strong terminal/CLI + SDK secondary surface)

## Overview

Cline is the reference IDE/approval agent. Its product thesis is *visibility and consent*: every file edit shows as a reviewable diff, every command and edit can require explicit approval, and all changes are tracked by checkpoints so the user can roll back. The marketing surface (`README.md`) leads with five products sharing one engine — CLI (`npm i -g cline`), VS Code Marketplace extension (`saoudrizwan.claude-dev`), JetBrains plugin (closed binary, talks to the shared core), web Kanban board, and an `@cline/sdk` for building custom agents.

The repo is a monorepo mid-migration. The mature, feature-complete implementation is the **VS Code extension** under `apps/vscode/src/` — this is where the real agent loop, tools, system prompt, checkpoints, MCP, and context manager live (~3,800-line `core/task/index.ts`). A newer, cleaner **portable engine** is being extracted into `sdk/packages/`: `@cline/agents` (a 1,544-line browser-safe `AgentRuntime`), `@cline/core` (Node runtime, auth, sessions, cron, hub, hooks), `@cline/llms` (config-driven provider/model selection), and `@cline/shared`. The CLI (`sdk/apps/cli/`) is built on the SDK and adds connectors (messaging platforms), `schedule` (cron agents), `team` (multi-agent), `hub`, and `plugin` commands.

Cline is deliberately not locked to one model: `src/shared/providers/providers.json` lists ~170 lines of providers (Anthropic, OpenAI, OpenAI-Codex/native Responses API, Gemini, Bedrock, Vertex, OpenRouter with 200+ models, Vercel AI Gateway, Cerebras/Groq, Ollama/LM Studio, any OpenAI-compatible endpoint, plus Cline's own account). The system prompt is heavily engineered per model family, and tools can be emitted as either XML pseudo-grammar or native tool-calls depending on the provider.

The core UX contract is **Plan/Act mode** plus **auto-approve**: the user toggles between an exploration phase (read-only-ish, plan presented for sign-off) and an execution phase, and separately decides how much autonomy to grant via granular per-action approval toggles or full "YOLO" mode.

## Architecture

**Control loop (VS Code core).** A `Task` (`apps/vscode/src/core/task/index.ts`) owns a session. `initiateTaskLoop()` runs a `while (!abort)` loop calling `recursivelyMakeClineRequests(userContent, includeFileDetails)`. Each iteration: load context + environment details → stream an API request via `attemptApiRequest()` → parse the streamed assistant message → `presentAssistantMessage()` renders text/tool blocks and dispatches tool calls through `ToolExecutor`. If the model emits **no tool call**, the harness injects `formatResponse.noToolsUsed()` and increments `consecutiveMistakeCount` to nudge it back to acting; `attempt_completion` ends the loop. There is explicit streaming/presentation coordination (`StreamResponseHandler`, `StreamChunkCoordinator`, `TaskPresentationScheduler`, a `presentAssistantMessageLocked` mutex) so the agent can act on partial streamed output.

**Tool grammar.** Two modes. Legacy/default is an **XML pseudo-grammar** parsed by `parse-assistant-message.ts` (`parseAssistantMessageV2`) — tool names are XML tags, params are nested tags. Newer providers (OpenAI Responses API: `openai-codex`, `openai-native`) force **native tool calling** (`enableNativeToolCalls`, gated by `apiFormat: ApiFormat.OPENAI_RESPONSES` and `isNextGenModelProvider()`); XML tools "don't work" there per `.clinerules/general.md`. Tool *definitions* live in `core/prompts/system-prompt/tools/*.ts` as `ClineToolSpec` objects (name, description, parameters with `instruction`/`usage`/`dependencies`/`contextRequirements`); *execution* lives in `core/task/tools/handlers/` wired through `ToolExecutorCoordinator` + `ToolValidator`.

**System prompt = compiled, modular, per-family.** A `PromptRegistry` singleton loads **variants** (model families: `generic`, `next-gen`, `native-next-gen`, `gpt-5`, `native-gpt-5`, `native-gpt-5-1`, `gemini-3`, `glm`, `hermes`, `devstral`, `trinity`, `xs` for small/local) and **components** (`agent_role`, `tool_use`, `mcp`, `editing_files`, `act_vs_plan_mode`, `task_progress`, `capabilities`, `feedback`, `rules`, `system_info`, `objective`, `user_instructions`, `skills`). A `PromptBuilder` resolves a `{{PLACEHOLDER}}` template via `TemplateEngine`, ordering components per the variant's `componentOrder` and allowing `componentOverrides`/`toolOverrides`. Variants are declared with a fluent `createVariant(family).components(...).tools(...).overrideComponent(...)` builder and **validated at build time** (`validateVariant`, strict). Output is golden-snapshot-tested across families/context permutations.

**Where state lives.** Per-task JSON files in a task directory (`core/storage/disk.ts`): `api_conversation_history.json`, `ui_messages.json`, `context_history.json` (compaction edits), `task_metadata.json`. Global state goes through a cached `StateManager` over VS Code `globalState`; MCP settings, model catalogs, and remote org config are separate JSON files. The CLAUDE.md documents the brittle multi-place plumbing required to add a global state key or provider (proto enum + two conversion functions + state-helpers + StateManager) — the cost of the gRPC/proto IPC architecture.

**Multi-surface split.** Extension host (Node) ↔ React **webview** communicate over a **gRPC-like protocol over VS Code message passing**; `.proto` files in `apps/vscode/proto/cline/*.proto` (per domain: `task.proto`, `ui.proto`, `state.proto`, `models.proto`, `account.proto`, `common.proto`) generate typed clients/servers (`npm run protos`) into `src/generated/{grpc-js,nice-grpc,hosts}`. JetBrains is a thin client over the same core. The **SDK `AgentRuntime`** is a parallel, cleaner re-implementation: a deterministic `execute()` loop (`run`/`continue`), `maxIterations` cap, typed events (`run-started`, `turn-started`, `assistant-message`, `message-added`, `turn-finished`, `run-finished`), and a **hook bag** (`beforeRun/afterRun/beforeModel/afterModel/beforeTool/afterTool/onEvent`) populated by config + plugins. The CLI/connectors run on top of this.

**Deterministic vs model-driven.** Model-driven: which tool to call, plan content, todo list contents, when to call `attempt_completion`. Deterministic harness logic: context truncation/compaction thresholds, auto-approval policy, command allow/deny parsing, loop detection, checkpoint commits, focus-chain file watching, the "no tools used → nudge" rule, and native-vs-XML tool selection.

## Feature Inventory

### Project / context discovery (rules, context)
- **`.clinerules`**: project rules auto-loaded by all surfaces. Supports a single `.clinerules` file *or* a `.clinerules/` directory; reserved subdirs `workflows/`, `hooks/`, `skills/` (`core/context/instructions/user-instructions/cline-rules.ts`). Global rules dir also supported, with per-rule enable/disable **toggles** (`globalClineRulesToggles`) synchronized to disk.
- **Rule conditionals & frontmatter**: rules can carry frontmatter and conditional activation (`rule-conditionals.ts`, `frontmatter.ts`); `RuleContextBuilder` assembles the active set.
- **Workflows**: markdown workflow files in `.clinerules/workflows/` exposed as slash commands.
- **Skills** (`skills.ts`, `components/skills.ts`, `use_skill` tool): named rule bundles the model can lazily *load on demand* rather than always paying their token cost — explicitly a context-cost optimization.
- **`@mentions`** (`core/mentions/`): users reference files/folders/URLs/problems to inject context.
- **Environment details**: each user turn is appended with `environment_details` (cwd, open tabs, current mode, etc.); first turn includes file listing (`includeFileDetails`).
- **`.clineignore`**: a `ClineIgnoreController` gates which files the agent may read/act on.

### Tool / action grammar (the actual tools)
From `src/shared/tools.ts` (`ClineDefaultTool` enum):
- File: `read_file`, `write_to_file`, `replace_in_file` (SEARCH/REPLACE diff blocks), `apply_patch` (Codex-style), `search_files` (regex), `list_files`, `list_code_definition_names`.
- Execution: `execute_command` (terminal; supports long-running/background processes that the agent keeps reacting to).
- Web/browser: `browser_action` (Puppeteer-driven click/type/scroll/screenshot session), `web_fetch`, `web_search`.
- MCP: `use_mcp_tool`, `access_mcp_resource`, `load_mcp_documentation`.
- Flow/meta: `ask_followup_question`, `attempt_completion`, `plan_mode_respond`, `act_mode_respond`, `new_task`, `focus_chain` (todo/`task_progress`), `condense`, `summarize_task`, `new_rule`, `report_bug`, `generate_explanation`, `use_skill`, `use_subagents`.
- **`replace_in_file` format**: `------- SEARCH` / `=======` / `+++++++ REPLACE` blocks; "ONLY replace first match," "include just enough lines to uniquely match," empty REPLACE = delete; strips `read_file` line-number prefixes. Diff application + edge-case handling in `core/assistant-message/diff.ts`.

### Plan / Act mode
- Two modes selected per message via `environment_details` (`components/act_vs_plan_mode.ts`). **ACT** = all tools except `plan_mode_respond`; ends via `attempt_completion`. **PLAN** = gather context + `plan_mode_respond` to present a plan for the user to approve before switching to ACT. A dedicated `switchToActModeCallback()` exists; switching to ACT can *force* creation of a `task_progress` todo list (`focus-chain/prompts.ts`).

### Approvals, permissions & safety (human-in-the-loop)
- **Auto-approve** (`core/task/tools/autoApprove.ts`, `docs/features/auto-approve.mdx`): granular per-action toggles — `readFiles`, `readFilesExternally`, `editFiles`, `editFilesExternally`, `executeSafeCommands`, `executeAllCommands`, `useBrowser`, `useMcp`. Reads/edits distinguish **inside-workspace vs external** paths (`shouldAutoApproveToolWithPath`, multi-root aware). `autoApproveAllToggled` and **`yoloModeToggled`** (YOLO) bypass everything. A **max-requests** counter forces a re-confirm after N auto-approved actions.
- **Command permissions** (`core/permissions/CommandPermissionController.ts`): a `CLINE_COMMAND_PERMISSIONS` env JSON with `allow`/`deny`/`allowRedirects`; **chained commands are split on `&&`/`||`/`|`/`;` and each segment validated**; dangerous chars (backticks in double quotes, stray newlines) are blocked; parse failure → block (fail-closed). No config → allow all (back-compat).
- **Diff review**: every edit surfaces in a `DiffViewProvider`; user can edit/revert in-editor before accepting.

### Checkpoints & diff review
- **Shadow Git** (`integrations/checkpoints/CheckpointTracker.ts`, `simple-git`): a *separate* git repo per workspace (`getShadowGitPath`, `hashWorkingDir`), so checkpoints never touch the user's real git. Commits the workspace state at each step (`git add .`, allows empty commits), provides diffs between checkpoints and reset-to-checkpoint. Refuses to run in sensitive dirs (home/desktop). Multi-root support via `MultiRootCheckpointManager`; lock files prevent concurrent corruption. `multifile-diff.ts` renders cross-file change review.

### Sessions & state / persistence
- Per-task JSON (api history, ui messages, context-history edits, metadata) under a per-task dir; resumable via `resumeTaskFromHistory()`. Task history list, favorites. `new_task` spawns child tasks. SDK `@cline/core` adds a `session/` package and team state that "persists across sessions."

### Context management & compaction
- `core/context/context-management/ContextManager.ts`: maintains `context_history.json` of edits applied to history. Two strategies, chosen by `useAutoCondense` + next-gen model: **(a) auto-condense** — LLM `summarizeTask`/`condense` at `autoCondenseThreshold = 0.75` (display warning at 0.60); **(b) programmatic truncation** — `getNextTruncationRange()` always keeps the first user/assistant pair and drops an even number of middle messages (½ or ¾ depending on pressure). `attemptFileReadOptimization()` dedupes repeated reads of the same file in history (cheap token recovery). Window math in `context-window-utils.ts`: `maxAllowedSize = contextWindow - {27k|30k|40k}` reserve buffers per window size.
- **Loop detection** (`core/task/loop-detection.ts`): counts consecutive *identical* tool calls; soft threshold injects a self-correction warning, hard threshold escalates to user/fail — saves tokens on stuck agents.

### Verification / completion
- `attempt_completion` is the terminal tool; harness can append a completion command and surface the result with a "Start New Task" button. No built-in test/lint *gate*, but Cline watches linter/compiler diagnostics (`integrations/diagnostics/`) live and feeds errors back as the agent works; terminal output (test/build failures) is fed back from `execute_command`.

### Provider & model routing
- ~170-entry `providers.json`; per-provider handler registered in `core/api/index.ts` `createHandlerForProvider()`. **Model-family detection** (`isClaude4`, `isGemini2.5`, `isGrok4`, `isGPT5`, `isNextGenModelProvider`) drives both the system-prompt variant *and* native-vs-XML tool calling. Special integrations: `integrations/claude-code/` and `integrations/openai-codex/`.

### Extensibility
- **MCP** (`services/mcp/McpHub.ts`): full client — stdio + Streamable HTTP (with reconnect handler), OAuth (`McpOAuthManager`/redirect resolver), an MCP marketplace catalog, `cline mcp` CLI management, and `load_mcp_documentation`/`use_mcp_tool`/`access_mcp_resource` tools.
- **Plugins (SDK)**: `createTool({name, description, inputSchema, execute})` + lifecycle **hooks** registered via the plugin system (`AgentRuntime` HookBag). VS Code core also has a `core/hooks/` system and `.clinerules/hooks/`.
- **Subagents** (`use_subagents` tool, `docs/features/subagents.mdx`): up to **5 in-process subagents in parallel**, each with its own prompt, returning research results + tool/token stats; gated by `subagentsEnabled` and disabled inside a subagent run (no recursion). Pitched explicitly as a *context-window* optimization for broad exploration.
- **Multi-agent teams**: `cline --team-name ... "..."`; a coordinator delegates to specialists; team state persists.
- **Scheduled agents**: `cline schedule create --cron ... --prompt ... --workspace ...`; cron jobs in `@cline/core/cron`, persist across restarts.
- **Messaging connectors**: `cline connect telegram|slack|discord|google-chat|whatsapp|linear ...` — each thread maps to an agent session, with access control (`sdk/apps/cli/src/connectors/`).
- **Slash commands / workflows**: built-in commands + user workflows from `.clinerules/workflows/`.

### UX surfaces
- **VS Code**: full React webview, inline diffs, terminal integration, browser session view.
- **JetBrains**: closed-source thin client on shared core.
- **CLI**: interactive TUI (OpenTUI/React) *and* **headless** mode for CI — `--json` streams `agent_event` objects, supports stdin piping (`git diff | cline "review"`).
- **Kanban**: separate repo; web board, **worktree per card**, auto-commit, dependency chains across cards.
- **SDK**: `@cline/sdk`/`@cline/core` for programmatic agents.

## Standout / Unique
- **Shadow-git checkpoints**: per-step version control in an *isolated* git repo, giving free rollback/diff without polluting the user's history — the cleanest "undo the agent" primitive in this cohort.
- **Compiled, validated, per-family system-prompt engine**: variants × components × templates with build-time validation and golden snapshots, plus automatic native-tool-call vs XML selection by model family. This is real prompt *engineering infrastructure*, not a string blob.
- **Granular, path-aware auto-approve + segment-level command allow/deny**: autonomy is a dial (per action type, inside-vs-outside workspace), and shell command policy parses chained commands segment-by-segment and fails closed.
- **One engine, many surfaces, real product extensibility**: VS Code/JetBrains/CLI/SDK/Kanban over a shared core, plus scheduled cron agents and messaging connectors — agents reachable from Slack/Telegram/Linear, on a schedule, headless in CI.
- **Subagents and skills framed as token-cost tools**, and a dedupe-file-reads context optimization — cost-awareness is baked into features, not bolted on.

## Pros
- Best-in-class human-in-the-loop UX: reviewable diffs, checkpoints, Plan/Act, tunable autonomy.
- Genuinely model-portable; prompt + tool-call format adapt per family, including small/local models (`xs`, `hermes`, `glm`).
- Clean SDK `AgentRuntime` with typed events + hook lifecycle is a good portable-core design and a stub-able seam.
- Strong extensibility: MCP (with OAuth + marketplace), plugins, skills/rules, scheduled agents, connectors, multi-agent teams.
- Deterministic safety logic (command parsing, loop detection, fail-closed permissions) sits in the harness, not the model.
- Live diagnostics/terminal feedback gives cheap signal before burning model turns.

## Cons / Limitations
- **Two parallel implementations** (mature `apps/vscode` vs newer `sdk/packages`) mid-migration → duplicated concepts, drift risk.
- **gRPC/proto IPC tax**: adding one state key or provider requires edits across many files (proto enum + conversions + helpers + UI) with *silent* failure modes documented in CLAUDE.md — high accidental complexity.
- The 3,800-line `Task` god-object is hard to reason about; streaming/presentation locking is intricate.
- IDE-first approval flow assumes a present human; not designed for server-owned hidden eval/budget loops.
- No first-class autonomous verification *gate* (relies on diagnostics + the model choosing to test).
- Compaction is partly last-minute (0.75 threshold summarize) rather than a measured running budget.
- TypeScript/Node runtime — looser typing than Glave's Rust target, and client/local state can act as source of truth.

## Lessons for Glave (econo-frontier, feature-parity lens)
- **Copy:**
  - **Shadow-git checkpoints** as the artifact/rollback primitive — isolated repo, per-step commit, diff + reset. Legible, cheap, server-ownable.
  - **Path-aware, per-action approval policy** + **segment-level command allow/deny that fails closed** — but as *server* policy, not a UX prompt.
  - **Loop detection** (consecutive-identical-tool soft/hard thresholds) and **file-read dedupe** — direct $/correct wins.
  - **Typed-event + hook-lifecycle loop** from the SDK `AgentRuntime` (`beforeModel/afterTool/onEvent`, `maxIterations`) as the shape of Hope's kernel trace + extension seam.
- **Adapt:**
  - The **compiled, validated, per-family system-prompt engine** (variants × components × templates, build-time validation, golden snapshots) → a typed Rust prompt compiler; pick variant + native-vs-structured tool format by model family. This is real reusable IP.
  - **Plan/Act** as a typed authority contract: an explore phase with reduced write authority + a sign-off gate, expressed as run-spec state, not a UI toggle.
  - **Skills/subagents as explicit token-budget tools** (lazy-loaded rule bundles; bounded parallel child contexts that report token stats) — fits Glave's child-task-node model with budgets.
- **Skip:**
  - The **gRPC/proto IPC + dual implementation**; Glave's typed Rust kernel should be the single source of truth with one schema, not webview message-passing.
  - IDE-resident state and the JetBrains/webview surface complexity as *core*; treat clients as actuators.
  - Auto-approve-as-UX and "YOLO mode" as a safety model; replace with deterministic server policy + budgets.
- **Cost angle:** Cline's most transferable cost levers are mechanical, not vibes: file-read dedupe, loop detection, lazy-loaded skills, subagents that isolate exploration token cost, reserve-buffer window math (`window - 27k/30k/40k`), and model-family-tuned prompts (smaller prompts for small models). Glave should make all of these *measured* (token saved per task) rather than threshold-guessed, and own them server-side so they apply across every task type, not just coding.

## Source
- Core agent loop: `apps/vscode/src/core/task/index.ts` (`initiateTaskLoop`, `recursivelyMakeClineRequests`, `attemptApiRequest`, `presentAssistantMessage`), `core/task/ToolExecutor.ts`, `core/task/loop-detection.ts`.
- Portable engine: `sdk/packages/agents/src/agent-runtime.ts`; `sdk/packages/{core,llms,sdk,shared}`.
- Tools: `apps/vscode/src/shared/tools.ts`; `apps/vscode/src/core/prompts/system-prompt/tools/*.ts` (incl. `subagent.ts`, `replace_in_file.ts`, `apply_patch.ts`).
- System prompt: `apps/vscode/src/core/prompts/system-prompt/{README.md,registry,components,variants,templates}`.
- Approvals/permissions: `apps/vscode/src/core/task/tools/autoApprove.ts`, `core/permissions/CommandPermissionController.ts`.
- Checkpoints: `apps/vscode/src/integrations/checkpoints/CheckpointTracker.ts`.
- Context mgmt: `apps/vscode/src/core/context/context-management/{ContextManager.ts,context-window-utils.ts}`; focus chain `core/task/focus-chain/`.
- Rules/skills: `apps/vscode/src/core/context/instructions/user-instructions/{cline-rules.ts,skills.ts,workflows.ts}`.
- MCP: `apps/vscode/src/services/mcp/McpHub.ts`.
- Storage: `apps/vscode/src/core/storage/{disk.ts,StateManager.ts}`.
- CLI/connectors/schedule: `sdk/apps/cli/src/{commands,connectors,runtime}`.
- Docs: `docs/features/{auto-approve,auto-compact,subagents,multiroot-workspace}.mdx`, `docs/tools-reference/all-cline-tools.mdx`; project lore in `.clinerules/general.md`.
- Upstream: https://github.com/cline/cline · Kanban: https://github.com/cline/kanban · snapshot `38f1c7e`.
