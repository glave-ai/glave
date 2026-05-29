# Pi

**Identity:** A deliberately minimal, aggressively-extensible TypeScript coding-agent harness whose core ships only four tools and pushes everything else (MCP, subagents, permissions, plan mode, todos) out to user-authored extensions/skills/packages.
**Source:** https://github.com/earendil-works/pi · MIT · TypeScript (Node ≥22, also Bun) · snapshot `20bcab2`
**Family:** terminal (minimal pole) — but with a clean SDK + RPC split that lets the same engine be embedded headlessly.

## Overview

Pi (`earendil-works/pi-mono`, by Mario Zechner / badlogic) is the explicit minimalist counter-pole to Claude Code and OpenCode. Its thesis, stated in `packages/coding-agent/README.md` ("Philosophy") and a companion blog post: keep the core tiny and let users *adapt pi to their workflow* via extensions rather than forking internals. By default the model gets exactly four tools — `read`, `write`, `edit`, `bash` — and three more (`grep`, `find`, `ls`) are available but off the default system-prompt list. Everything a heavier agent bakes in is reframed as something you either don't need, can do with a file, or can add as a TypeScript extension.

The repo is a clean 4-package monorepo with sharp layering. `@earendil-works/pi-ai` is a standalone, provider-portable LLM client (TypeBox-typed tools, normalized streaming events, ~25 providers, cross-provider context handoff). `@earendil-works/pi-agent-core` is a provider-agnostic stateful agent loop (tool execution, steering/follow-up queues, `beforeToolCall`/`afterToolCall` hooks, custom message types). `@earendil-works/pi-coding-agent` is the interactive CLI + coding tools + extension system + sessions/compaction. `@earendil-works/pi-tui` is a differential-rendering terminal UI library. The lower two packages have no coding or terminal assumptions and are independently publishable — pi-ai is usable as a generic SDK and even runs in browsers.

The minimalism is real and enforced, not marketing. Grepping the core source (`packages/*/src`) for MCP, subagents, permission popups, plan mode, and todo tools returns nothing functional — the only "mcp" hit is an OAuth scope string; "permission" hits are OS file-mode code. The single gating primitive that exists is the `beforeToolCall` hook in pi-agent-core, which extensions use to build their own confirmation/sandbox flows. The README is unusually candid about this: "No permission popups. Run in a container, or build your own confirmation flow with extensions."

Pi also leans into a data-flywheel posture that maps directly onto Glave's trace-corpus thinking: it actively asks users to publish their real OSS coding sessions (JSONL transcripts) to Hugging Face via `pi-share-hf`, arguing real-world tool-use/failure/fix data beats toy benchmarks for improving agents. The maintainer publishes his own `pi-mono` work sessions as a public dataset.

## Architecture

**Layered control loop.** The runtime is a strict pipeline: `AgentMessage[] → transformContext() → AgentMessage[] → convertToLlm() → Message[] → LLM`. `transformContext` is the optional pruning/compaction/context-injection hook; `convertToLlm` (required for custom message types) filters UI-only messages and converts app-specific types down to the three roles the LLM understands (`user`, `assistant`, `toolResult`). The loop itself lives in `packages/agent/src/agent-loop.ts` (`agentLoop`, `agentLoopContinue`, `runLoop`) and is wrapped by a stateful `Agent` class (`packages/agent/src/agent.ts`). A turn = one LLM call plus its tool executions; the loop continues while the model emits tool calls and stops on a plain assistant message, on `shouldStopAfterTurn`, or when every tool result in a batch returns `terminate: true`.

**Where state lives.** State is plain serializable data, not hidden runtime. `AgentState` (systemPrompt, model, thinkingLevel, tools, messages, plus readonly isStreaming/streamingMessage/pendingToolCalls/errorMessage) is exposed via `agent.state`. Persistence is JSONL session files (`packages/agent/src/harness/session/jsonl-*.ts`), one entry per line, each carrying `id` + `parentId` — a tree, not a list. This enables in-place branching, `/tree` navigation, `/fork`, and `/clone` without copying files. Entry kinds are typed (`SessionMessageEntry`, `ThinkingLevelChangeEntry`, `ModelChangeEntry`, `CompactionEntry`, `BranchSummaryEntry`, `CustomEntry`, `LabelEntry`, `SessionInfoEntry`, `CustomMessageEntry`) in `packages/coding-agent/src/core/session-manager.ts`, and there is an explicit v1→v2 migration that retrofits the id/parentId tree. Sessions live under `~/.pi/agent/sessions/` organized by working directory; `SessionManager.inMemory()` gives an ephemeral variant for the SDK.

**The pi-ai package (the typed seam).** `packages/ai/src/types.ts` is the heart. Content blocks are a tagged union: `TextContent`, `ThinkingContent` (with `thinkingSignature`/`redacted` for provider continuity), `ImageContent` (base64), `ToolCall` (`{type, id, name, arguments, thoughtSignature?}`). Messages are `UserMessage | AssistantMessage | ToolResultMessage`, all timestamped; `AssistantMessage` carries `api`, `provider`, `model`, `responseModel?`, `responseId?`, `diagnostics?`, `usage`, and a typed `stopReason` (`stop | length | toolUse | error | aborted`). `Context = { systemPrompt?, messages, tools? }` — fully JSON-serializable, the unit of persistence and provider handoff. `Model<TApi>` is parameterized by API so metadata (cost per-million tokens for input/output/cacheRead/cacheWrite, contextWindow, maxTokens, reasoning flag, input modalities) stays type-accurate, and the `compat` field is conditionally typed to the API (OpenAI-completions vs responses vs Anthropic-messages each get their own compat interface).

**Streaming as a normalized event protocol.** `AssistantMessageEvent` (in `types.ts`) is the provider-independent event union: `start`, `text_start/delta/end`, `thinking_start/delta/end`, `toolcall_start/delta/end`, `done`, `error`. Every delta carries a `contentIndex` and a running `partial: AssistantMessage`; the README warns that blocks may interleave, so consumers key off `contentIndex` rather than assuming contiguous runs. Tool-call arguments stream as progressively-parsed partial JSON (`toolcall_delta` gives best-effort partial args, never `undefined`, minimum `{}`), enabling UI like "Writing to: <path>" before content finishes. Errors and aborts are *encoded in the stream*, not thrown — `StreamFunction`'s contract is that once invoked it must surface failures as a terminal `error` event with an `AssistantMessage` of stopReason `error`/`aborted` plus partial content, which can be appended to context and continued.

**Provider portability.** ~25 providers normalize to a small set of `KnownApi` adapters (`anthropic-messages`, `openai-completions`, `openai-responses`, `openai-codex-responses`, `azure-openai-responses`, `google-generative-ai`, `google-vertex`, `mistral-conversations`, `bedrock-converse-stream`). Adapters are lazily loaded via a registry (`register-builtins.ts`). Cross-provider handoff is automatic: messages from another provider's API have thinking blocks converted to `<thinking>`-tagged text while tool calls/results/text pass through, so you can switch Claude→GPT-5→Gemini mid-conversation. A large `OpenAICompletionsCompat` surface (developer-role, reasoning_effort format, cache_control convention, session-affinity headers, max-tokens field name, strict mode, etc.) absorbs the dialect differences across OpenAI-compatible servers (Ollama, vLLM, LiteLLM, DeepSeek, Together, zAI…). Cost/token usage is tracked per message in `Usage` (input/output/cacheRead/cacheWrite/totalTokens + a parallel `cost` breakdown).

**Argument validation.** `validateToolArguments`/`validateToolCall` (`packages/ai/src/utils/validation.ts`) run TypeBox `Value.Convert` (coercion) then a compiled validator's `Check`; on failure they throw a formatted, path-pointed message. In `agentLoop`, validation is automatic and a failure is returned to the model as a tool error so it can retry. Tools carry TypeBox `parameters` schemas (serializable as plain JSON), and `StringEnum` is provided because `Type.Enum` produces anyOf/const patterns Google rejects.

## Feature Inventory

- **Tool / action grammar (the small tool set).** Built-ins: `read`, `bash`, `edit`, `write`, `grep`, `find`, `ls` (`packages/coding-agent/src/core/tools/`). `allToolNames` is a 7-element set; default system prompt advertises only `read/bash/edit/write`. Curated bundles exist: `createCodingTools` (the four), `createReadOnlyTools` (read/grep/find/ls). CLI gates tools with `--tools`/`-t` (allowlist), `--exclude-tools`/`-xt`, `--no-builtin-tools`, `--no-tools`. `read` supports ranged reads; `edit` is a structured diff/replace with a file-mutation queue; `bash` is a streamed shell exec with truncation (`DEFAULT_MAX_LINES`/`DEFAULT_MAX_BYTES`) and pluggable `BashOperations` so an extension can redirect execution to SSH/sandbox.
- **Argument validation (TypeBox schemas).** All tools (built-in, extension, custom) declare TypeBox `parameters`; `Static<TParams>` gives compile-time types and runtime validation/coercion. `ToolDefinition` adds `prepareArguments?` for pre-validation arg shimming and a `defineTool` helper to preserve param inference.
- **Streaming tool-call events.** Normalized `AssistantMessageEvent` union with partial-JSON tool args, `contentIndex` correlation, interleave-safe semantics, abort/error-as-event. Agent layer re-emits as `message_update` carrying the inner `assistantMessageEvent`.
- **Provider-portable context serialization.** `Context`/`Message` are plain JSON; `JSON.stringify(context)` persists or transfers a conversation including images and thinking signatures. Cross-provider handoff transforms thinking blocks automatically. Supports OpenAI/Anthropic/Google/Vertex/Mistral/Bedrock + ~20 OpenAI-compatible providers, OAuth (Anthropic Pro/Max, OpenAI Codex, GitHub Copilot, Gemini CLI), and fully custom `Model` objects for local endpoints.
- **Sessions & state.** JSONL tree (id/parentId), per-cwd storage, `/resume`/`-r`, `-c` continue, `--session <path|id>`, `--fork`, `--no-session` ephemeral. Tree ops: `/tree` (navigate/branch in place with filter modes and bookmarks), `/fork`, `/clone`. Auto-save; typed entry kinds; migrations.
- **Compaction.** First-class, on by default (`packages/coding-agent/src/core/compaction/`). Two triggers: reactive (recover+retry on context overflow) and proactive (approaching the limit). Manual `/compact [instructions]`. Lossy summary entries are written to the tree but full history is retained and revisitable via `/tree`. Branch summarization exists for tree navigation. Customizable/cancelable via the `session_before_compact` extension event.
- **Extension mechanism (the core extensibility play).** TypeScript modules `export default (pi: ExtensionAPI) => {...}` (sync or async; async factories block startup, useful for fetching remote model lists before `registerProvider`). `ExtensionAPI` (`packages/coding-agent/src/core/extensions/types.ts`) is large: `registerTool`, `registerCommand`, `registerShortcut`, `registerFlag`/`getFlag`, `registerMessageRenderer`, `registerProvider`/`unregisterProvider` (incl. OAuth), `sendMessage`/`sendUserMessage`/`appendEntry`, session metadata, `getActiveTools`/`setActiveTools`, model/thinking control, a shared `events` EventBus, and a rich `on(...)` event surface (~30 events: session lifecycle, `context`, `before_provider_request`/`after_provider_response`, `before_agent_start` (can rewrite system prompt), turn/message/tool execution events, `tool_call` (can block — the permission hook), `tool_result` (can rewrite), `user_bash`, `input`). UI primitives (`ExtensionUIContext`) let extensions add widgets, footers, headers, overlays, custom editors, selectors, confirmations, status lines.
- **Skills.** Agent-Skills-standard markdown (`SKILL.md` with frontmatter name/description, `disable-model-invocation`). Loaded from `~/.pi/agent/skills/`, `.agents/skills/`, `.pi/skills/`, packages. Invoked via `/skill:name` or auto-loaded by the model; injected as `<skill name=... location=...>` blocks (only when `read` tool present). Description capped at 1024 chars to limit prompt cost.
- **Prompt templates.** Reusable markdown with `{{var}}` placeholders, expanded by typing `/name`.
- **Pi Packages.** Bundle extensions/skills/prompts/themes; installed from npm/git/ssh/https with `pi install`, version/ref pinning, project-local `-l`, `pi config` to enable/disable resources. Manifest via a `pi` key in `package.json` or auto-discovery from conventional dirs. Supply-chain hardening: exact-pinned deps, `min-release-age=2`, shrinkwrap with lifecycle-script allowlist, `--ignore-scripts` everywhere.
- **CLI UX & modes.** Four modes: interactive TUI, `--print`/`-p` (one-shot, merges piped stdin), `--mode json` (JSON-lines events), `--mode rpc` (LF-delimited JSONL over stdin/stdout for process integration), plus an embeddable SDK (`createAgentSession`, `AgentSessionRuntime`). `@file` args, image paste/drag, `!cmd`/`!!cmd` inline bash, steering (Enter) vs follow-up (Alt+Enter) message queues, `@`-fuzzy file refs, model cycling (Ctrl+P), thinking-level cycling (Shift+Tab), themes (hot-reloading), `/export` HTML, `/share` (GitHub gist).
- **Context files & system prompt.** Loads `AGENTS.md`/`CLAUDE.md` walking up from cwd plus global; concatenated into `<project_context>`. System prompt is replaceable (`.pi/SYSTEM.md`, `--system-prompt`) or appendable (`APPEND_SYSTEM.md`, `--append-system-prompt`). The default prompt is short and *tool-aware*: a tool only appears in "Available tools" if a one-line snippet is supplied, and guidelines are conditioned on which tools are active.
- **Session sharing as data (flywheel relevance).** Explicit, first-class: publish JSONL sessions to Hugging Face via `pi-share-hf`; maintainer ships a public `pi-mono` dataset. The JSONL tree format doubles as a trace corpus.
- **Unique.** `pi-ai` and `pi-agent-core` are genuinely standalone, browser-capable, provider-agnostic libraries — the harness is decomposable. `terminate: true` tool hint to short-circuit the follow-up LLM call. Custom `AgentMessage` types via TypeScript declaration merging. Faux in-memory provider (`registerFauxProvider`) for fully offline deterministic tests (used by the regression suite — no paid tokens).

## Deliberate Omissions

Pi's README "Philosophy" section states each omission with rationale; source confirms none of these exist in core:

- **No built-in MCP.** Rationale (linked blog "what if you don't need MCP?"): build CLI tools with READMEs and expose them via Skills, or add MCP through an extension. MCP is treated as a workflow choice, not a harness primitive.
- **No subagents.** Rationale: "many ways to do this" — spawn pi instances via tmux, or build it as an extension/package. No delegation/child-task system in core.
- **No permission popups.** Rationale: "Run in a container, or build your own confirmation flow with extensions inline with your environment and security requirements." The only gating seam is the `beforeToolCall`/`tool_call` hook; default behavior runs tools without prompting.
- **No plan mode.** Rationale: "Write plans to files, or build it with extensions, or install a package."
- **No built-in to-dos.** Rationale: "They confuse models. Use a TODO.md file, or build your own."
- **No background bash.** Rationale: "Use tmux. Full observability, direct interaction." `bash` is foreground/streamed only.
- **Implicitly minimal default toolset:** `grep`/`find`/`ls` exist but are excluded from the default prompt; the model is steered to use `bash` for exploration unless those tools are explicitly enabled.

The throughline: the *core* refuses to own anything that (a) can be a file, (b) can be an extension, or (c) is a security boundary the user should own per-environment. Safety is explicitly delegated downward to the container/extension layer.

## Standout / Unique

- **Clean separation of "LLM transport" from "agent semantics" from "coding product."** pi-ai is a publishable, browser-runnable, provider-portable client with normalized streaming and typed tool args — the kind of seam Glave wants between provider adapters and harness logic.
- **Provider normalization done thoroughly.** ~25 providers, automatic cross-provider context handoff, an exhaustive `compat` matrix for OpenAI-compatible dialects, OAuth subscription auth — all terminating in one `Message`/`AssistantMessageEvent` shape.
- **Typed tool arguments end-to-end** via TypeBox: serializable schemas, runtime coercion+validation, validation errors fed back to the model for self-correction.
- **Session-tree-as-data.** id/parentId JSONL gives branching, fork, clone, in-place navigation, and a ready-made shareable trace corpus, with typed entry kinds and migrations.
- **Extension surface is a contract, not a plugin afterthought.** ~30 typed lifecycle events with typed results, including hooks that can rewrite the system prompt, block tools, rewrite results, and replace the provider stream.
- **Honest minimalism.** The omissions are documented with rationale and actually absent from source — a rare match for a subtraction-first design doctrine.

## Pros

- Tiny, legible core loop that is easy to reason about and test (faux provider enables zero-cost deterministic eval).
- Strong typing throughout (TypeBox tool schemas, API-parameterized `Model`, tagged-union content/events); strict TS hygiene rules in `AGENTS.md`.
- Excellent provider portability and cost/token accounting baked into `Usage`.
- Compaction is first-class with reactive + proactive triggers, and history is non-destructive (tree retains everything).
- Clean SDK + RPC + print + JSON modes — the same engine runs interactive and headless, which is exactly what eval/automation needs.
- Flywheel-aligned: session sharing and a public dataset are part of the project's identity.
- Supply-chain discipline (pinned deps, min-release-age, shrinkwrap allowlist, `--ignore-scripts`).

## Cons / Limitations

- **"No permission popups; run in a container" pushes the entire trust boundary onto the operator.** By default `bash`/`write`/`edit` execute against the real workspace with no policy check. For Glave's server-owned model this is a non-starter as-is: policy, budget, and audit cannot be optional extension code that runs *inside* the same trusted process.
- **The only gate is an in-process hook (`beforeToolCall`).** It is advisory, runs in the same Node process as the model-driven tools, and is bypassable by simply not loading the extension — not a server-enforced boundary.
- **Extensions/packages run with full system access** (README security note: "Extensions execute arbitrary code, and skills can instruct the model to perform any action"). Outsourcing core capabilities to third-party packages is a supply-chain and trust hazard at scale.
- **Local CLI/files are the source of truth** (sessions on disk, settings in `~/.pi`, auth.json in cwd). No server-owned state, hidden evals, or central budget enforcement.
- **TypeScript/Node, not Rust** — opposite of Glave's typed Rust kernel target; useful as design reference, not as a code base to adopt.
- **Minimal default toolset can cost tokens** by routing exploration through `bash` rather than cheaper structured `grep`/`find`/`ls` (those are gated off by default).
- **No repo map / structural code context** (unlike Aider) — context selection is left to the model + compaction.

## Lessons for Glave (econo-frontier, feature-parity lens)

- **Copy:**
  - The **provider-normalization seam**: one internal `Message`/content-block/streaming-event shape that all provider adapters terminate in, with cross-provider handoff and per-message cost/token accounting. This is directly Cross-Harness Pattern #6.
  - **Typed, validated tool arguments** with validation errors returned to the model for self-correction. In Rust: schema-typed action args (serde/typed structs) with structured rejection.
  - **Streaming events with explicit `contentIndex` correlation and errors-as-data** (no thrown exceptions across the stream boundary) — a clean trace schema.
  - **Session-tree JSONL with id/parentId** as the trace format; branching/fork/clone for free, and it doubles as the eval/flywheel corpus.
  - **The faux/in-memory provider** for deterministic, zero-cost eval of the loop — exactly the fast measured-eval discipline Glave wants.
  - **Compaction as a first-class subsystem** with both reactive (overflow-recover) and proactive triggers.
- **Adapt:**
  - The **extension event surface** is the right *shape* for hooks, but Glave should invert ownership: the equivalents of `before_provider_request`, `tool_call` (block), `context`, `before_agent_start` (rewrite prompt) must be **server-owned typed policy stages**, not user TypeScript. Keep the hook points; move the authority server-side.
  - **AGENTS.md/CLAUDE.md project-context loading** and the tool-aware system-prompt assembly (a tool only appears in the prompt when active) — adapt into deterministic, budgeted context packing.
  - **Skills/prompt-templates** as on-demand capability packs — adapt into typed, eval-gated capability bundles surfaced selectively by task type/trust tier.
  - **RPC/SDK/print/json modes** — adapt the headless protocols, but make the *server* the orchestrator and the client an actuator.
- **Skip (for the core; revisit later behind evals):**
  - Pi-package install-from-npm/git as a core feature (supply-chain risk; Glave exposes capabilities by policy, not arbitrary third-party code).
  - The "no permission popups, run in a container" stance — Glave must add what Pi omits here.
  - Terminal-first TUI polish, themes, doom-as-extension — not the moat.
  - Background-bash-via-tmux and "spawn pi in tmux for subagents" — Glave needs first-class child task nodes with budgets/verification, not process spawning.
- **Cost angle:** A minimal core = fewer tokens (short tool-aware system prompt, 4 default tools, skill descriptions capped at 1024 chars), fewer moving parts to evaluate, and a deterministic faux-provider for cheap regression. Per-message `Usage` with input/output/cacheRead/cacheWrite cost breakdown is precisely the substrate for a `$/correct-task` metric. The lesson: small typed loop + cheap deterministic eval is what makes the cost axis measurable. (Counter-note: gating `grep/find/ls` off by default can push exploration into pricier `bash` turns — Glave should expose cheap structured actions by default.)
- **Minimalism notes:** Pi proves the subtraction discipline is achievable and that the right unit of subtraction is "anything that can be a file, an extension, or a per-environment security choice." That maps cleanly onto a small typed Rust kernel: a minimal action grammar (read/edit/bash/write + submit), typed args/events, JSONL-tree traces, budgeted context, and pluggable provider adapters. **Where Glave must add what Pi omits:** the kernel must *own* policy (what an action may do, evaluated before the model acts), budget (token/cost ceilings enforced centrally), and audit (every allow/deny + change + cost in the trace) as non-optional, server-side stages — precisely the things Pi deliberately delegates to the container and to user extensions. Pi's `beforeToolCall` hook and sandbox-via-`BashOperations` indirection show *where* those seams go; Glave's job is to make them server-enforced and deterministic rather than advisory and in-process.

## Source

Snapshot `20bcab2` under `refs/harnesses/pi/`.

- Philosophy / omissions / CLI: `refs/harnesses/pi/packages/coding-agent/README.md` (esp. "Philosophy", "CLI Reference", "Customization").
- Core types (Context, Message, content blocks, streaming events, Model, compat): `refs/harnesses/pi/packages/ai/src/types.ts`.
- Tool-arg validation: `refs/harnesses/pi/packages/ai/src/utils/validation.ts`.
- pi-ai entry / provider registry: `refs/harnesses/pi/packages/ai/src/index.ts`, `.../src/api-registry.ts`, `.../src/stream.ts`; faux provider in `.../src/providers/faux.ts`.
- Agent loop & state: `refs/harnesses/pi/packages/agent/src/agent-loop.ts`, `.../src/agent.ts`, `.../src/types.ts`; agent-core README `refs/harnesses/pi/packages/agent/README.md`.
- Session tree / JSONL: `refs/harnesses/pi/packages/agent/src/harness/session/`, `refs/harnesses/pi/packages/coding-agent/src/core/session-manager.ts`.
- Compaction: `refs/harnesses/pi/packages/coding-agent/src/core/compaction/`.
- Built-in tools: `refs/harnesses/pi/packages/coding-agent/src/core/tools/` (`index.ts`, `bash.ts`, `edit.ts`, `read.ts`, …).
- Extension API & events (incl. `beforeToolCall`/`tool_call` gating, `registerProvider`): `refs/harnesses/pi/packages/coding-agent/src/core/extensions/types.ts`.
- System prompt assembly: `refs/harnesses/pi/packages/coding-agent/src/core/system-prompt.ts`.
- Skills loader: `refs/harnesses/pi/packages/agent/src/harness/skills.ts`.
- RPC protocol: `refs/harnesses/pi/packages/coding-agent/src/modes/rpc/rpc-types.ts`.
- pi-ai capabilities (providers, handoff, validation, OAuth, faux): `refs/harnesses/pi/packages/ai/README.md`.
- Upstream: https://github.com/earendil-works/pi · packages: `@earendil-works/pi-ai`, `@earendil-works/pi-agent-core`, `@earendil-works/pi-coding-agent`, `@earendil-works/pi-tui`. Session-sharing: https://github.com/badlogic/pi-share-hf, dataset `badlogicgames/pi-mono`.
