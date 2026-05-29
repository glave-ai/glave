# Harness Landscape — Executive Summary

Date: 2026-05-29
Audience: Glave / Hope harness design
Inputs: the nine source-grounded harness writeups in this folder, plus public
information on the closed competitors.

---

## TL;DR

The field has already converged on what a serious harness needs. Nine
independent open-source codebases keep reinventing the **same dozen subsystems** —
typed action grammar, context budgeting, sandbox policy, sessions/traces,
provider abstraction, compaction, verification, cost accounting. That convergence
is a gift: the parity surface is *knowable*, not speculative.

Glave's edge is not a new subsystem. It is **ownership and measurement**:

1. Almost every harness treats **cost-per-correct-task as an afterthought** —
   they count tokens only to trigger compaction. For Glave it is the north-star
   metric, so it must be a first-class trace contract from line one.
2. Almost every harness puts the **trust boundary on the client** (local files,
   local secrets, permission *prompts*). Only OpenHands and Codex CLI treat
   policy as a server-owned typed contract. Glave is server-owned by doctrine, so
   it should start where they ended up.
3. SWE-agent's own authors proved the decisive point: when they **subtracted**
   their elaborate interface down to bare bash (Mini-SWE-agent), the SWE-bench
   score held while tokens and complexity dropped. **Measurement, not intuition,
   decides what stays in the harness.** That is Glave's entire flywheel thesis,
   already validated by the reference harness.

Recommendation: build the *smallest typed Rust kernel* that can run a gated loop,
emit a replayable trace with cost, and verify a result — then let evals add
features one at a time. Parity with "the things that matter" is ~10 subsystems,
most of which the kernel needs anyway. Everything else (MCP, subagents, IDE,
integrations) is a deferrable adapter.

---

## The strategic lens

Glave is an **econo-frontier** company building one engine, **Hope**. Coding is
one task type on that engine, not the product. The harness is the core IP. The
metric is **cost per correct completed task**. The harness is a typed,
server-owned Rust kernel; clients and workspaces are untrusted actuators. The
improvement loop is a **fully autonomous (zero-HIL) measured flywheel**. Glave
wants **parity with the features that matter** while staying minimal and
cost-efficient — not a clone of any one tool.

Every recommendation below is filtered through that lens.

---

## Compiled cross-harness findings

The patterns that recurred across the nine, and who does each best (→ where to
read more).

1. **A typed event stream wants to be the source of truth.** OpenHands models a
   conversation as an append-only stream of typed `Action`/`Observation` events;
   Codex CLI uses a versioned `Thread`/`Turn`/`Item` protocol with wire bindings
   generated *from* the Rust types; Pi serializes `Context`/`Message` as plain
   JSON. This *is* Glave's `RunEvent` corpus. → openhands, codex, pi

2. **The trust boundary belongs on the server — most harnesses put it on the
   client.** Aider, Pi, OpenCode, Gemini CLI, Cline, and Goose all keep state,
   secrets, and policy on the local machine and gate with permission prompts.
   Only **OpenHands** (server holds secrets/policy/trace; sandbox is a
   session-keyed actuator; privileged actions go through the server's own MCP
   host so the agent never holds a credential) and **Codex CLI** (sandbox policy
   as a typed protocol enum) match Glave's doctrine. → openhands, codex

3. **Permissions must be deterministic policy, evaluated before the model acts —
   not prompts, not an LLM judge.** Codex CLI: `SandboxPolicy` ×
   `AskForApproval` enums + a unit-tested Starlark `execpolicy`. Gemini CLI: a
   tiered TOML policy engine (DEFAULT<EXTENSION<WORKSPACE<USER<ADMIN) →
   ALLOW/DENY/ASK. Goose: a `ToolInspector` trait chain → typed
   `Allow/RequireApproval/Deny`. The LLM "judge" in Goose/OpenHands is advisory
   only; never the boundary. Glave's zero-HIL constraint means **policy must
   resolve without a human and without trusting the model.** → codex, gemini-cli,
   goose

4. **Context is a budgeted subsystem, and it is the biggest cost lever.**
   - Repo map: Aider's tree-sitter → PageRank → token-budgeted symbol graph is
     the best structural-context idea in the set.
   - Compaction tiers: Gemini compresses at 50% / preserves recent 30%; Goose
     does cheap *tool-pair* summarization before full compaction; OpenCode
     summarizes at 95%; Cline does reserve-buffer math.
   - Lazy tool loading: Codex `tool_search` (BM25) and Goose `code-mode` keep
     unused tool specs out of the prompt.
   → aider, gemini-cli, goose, codex

5. **Cheap, deterministic, in-loop feedback beats burning model turns.**
   OpenCode injects **LSP diagnostics** inline right after every edit; Aider and
   SWE-agent **lint-gate edits** (reject syntactically broken edits before they
   land); Cline and Gemini do **loop detection**. This converts "write broken
   code → discover 3 turns later" into one corrective signal. → opencode, aider,
   swe-agent

6. **Completion must be an explicit, evidence-bearing action — not "the model
   stopped."** SWE-agent's `submit` writes a patch behind sentinels with a staged
   self-review gate; Goose's `final_output` is a schema-constrained
   submit-with-evidence tool. OpenCode's "done = no more tool calls" is the
   anti-pattern. → swe-agent, goose

7. **Edit application is a designed surface, not free text.** Aider maintains
   multiple tolerant edit-format parsers (SEARCH/REPLACE, udiff, whole) chosen
   per-model by benchmark; Codex's `apply_patch` has a formal Lark grammar.
   Lesson: Glave should make editing a **typed, validated action** at the server
   boundary, not a free-text parse. → aider, codex

8. **Cost/token accounting is first-class in exactly the harnesses Glave most
   resembles — and absent everywhere else.** OpenCode persists per-session AND
   per-subagent cost; OpenHands has `MetricsSnapshot` + `max_budget_per_task`;
   SWE-agent exposes `cost_limit` + best-of-N reviewer as a cost↔correctness
   knob. Everyone else counts tokens only for compaction. → opencode, openhands,
   swe-agent

9. **The eval/trace loop is the product — SWE-agent is Glave's flywheel made
   concrete.** A frozen dated YAML config = a hypothesis; run over a fixed
   instance set; emit `.traj` traces + predictions + per-step cost; score
   autonomously (`sb-cli`); compare runs. Fully zero-HIL. The Mini-SWE-agent
   pivot is the proof that subtraction is measurable. → swe-agent

10. **Provider abstraction must be separate from harness semantics.** Pi's
    `pi-ai` normalizes ~25 providers behind one streaming/tool-call/serialization
    shape; Goose has a `Provider` trait + registry over 30+; Goose's **ACP
    subscription-reuse auth** (ride an existing Claude/ChatGPT/Gemini
    subscription instead of per-token billing) is a direct $/correct lever.
    Codex CLI is the anti-pattern: hard-coupled to OpenAI auth/wire format. →
    pi, goose, codex

11. **Subagents are context-compression workers, not magic.** OpenCode's read-only
    search subagent (own child session, cost rolled up), Cline's max-5
    token-budgeted subagent tool, and Gemini's "subagent = compression" framing
    all agree: child task nodes with explicit budgets and outputs. → opencode,
    cline, gemini-cli

12. **Rollback/diff artifacts build trust and aid evaluation.** Shadow-git
    checkpoints (Cline, Gemini — commits to an *isolated* git repo, no pollution
    of user history), Aider per-edit commits, OpenCode SQLite file-version
    history. → cline, gemini-cli, aider

13. **Minimalism is validated, not just an aesthetic.** Pi ships 4 tools and
    pushes everything else to extensions; Mini-SWE-agent is ~100 lines and
    matches SWE-bench Verified. The right unit of subtraction (Pi) is "anything
    that can be a file, an extension, or a per-environment choice." → pi,
    swe-agent

---

## Feature-parity matrix

Filtered for an econo-frontier, server-owned, cost-measured harness.

### Table-stakes — parity required to be credible
These are also mostly what the kernel needs anyway.

| Feature | Why it matters | Best reference |
| --- | --- | --- |
| Project context file (`CLAUDE.md`/`AGENTS.md`-style) | Cheap, high-leverage, now a shared dialect | codex, gemini-cli |
| Core action set: inspect/search/read/edit/run/submit | The minimal coding workbench | swe-agent, opencode |
| Typed, validated edit action | Avoids free-text parse failures | aider, codex |
| Context compaction + budgeting | Largest cost lever on long tasks | gemini-cli, goose |
| Server-owned sandbox + permission policy | Safety boundary; zero-HIL requirement | codex, openhands |
| Session/state persistence + replayable trace | Debugging, eval, audit | openhands, swe-agent |
| Provider abstraction (multi-provider) | Avoid lock-in; route by cost | pi, goose |
| Per-run cost/token accounting + budget ceiling | The north-star metric | openhands, opencode |
| Explicit verification/submit gate | "Done" must be checkable | swe-agent |
| Diffs/checkpoints for rollback | Trust + failure inspection | cline, aider |

### Cost levers / differentiators — where Glave should win
The features that turn parity into an econo-frontier advantage.

| Lever | Payoff | Reference |
| --- | --- | --- |
| **Cost-per-correct as a first-class trace contract** | Makes the metric optimizable, not anecdotal | (Glave-native; closest: swe-agent) |
| **Structural repo map** (Rust, token-budgeted) | Fewer blind reads/turns | aider |
| **LSP / typed post-condition checks in-loop** | Catches breakage in 1 turn, not 3 | opencode |
| **Measured eval flywheel** (frozen config → autoscore → compare) | Decides features by data | swe-agent |
| **Lazy tool loading** (BM25/registry) | Keeps prompts lean → cheaper turns | codex, goose |
| **ACP subscription-reuse auth** | Cuts raw inference $ directly | goose |
| **Weak/editor-model routing** (architect/editor split) | Cheap model does the mechanical edits | aider |
| **Cache-aware prompt assembly** (stable prefix → volatile suffix) | Higher cache hit rate → lower $ | aider |

### Defer — valuable, but only after the core is measured
- MCP — design the internal action registry so MCP is an *adapter* later.
- Subagents — as budgeted context-compression workers, not before the base loop.
- Hooks / skills / custom commands — prompt sprawl without evals.
- IDE / GUI / desktop surfaces — clients are actuators; ship the engine first.
- GitHub issue-to-PR, Slack/Jira/Linear integrations.
- Multi-agent Kanban, scheduled/cron agents, messaging connectors.

### Skip / actively avoid
- Cloning Claude Code, or starting at OpenHands platform scale.
- Client-owned state / proto-IPC tax (Cline's gRPC/proto coupling, documented
  silent-failure modes).
- Permission **prompting** as the security boundary — violates zero-HIL.
- An LLM judge as the *boundary* (advisory only — Goose/OpenHands agree).
- Huge context windows as a substitute for context management (Gemini's own
  prompt warns against this).
- Tool sprawl — Goose's own system prompt warns that too many tools degrade
  selection accuracy.
- Free-text edit parsing as the only edit interface.

---

## The closed-source landscape (parity context only)

No readable harness; covered from public info as competitive context. **Design
signals only — never sourced.**

- **Claude Code** — the dev-terminal parity bar: `CLAUDE.md`, subagents, hooks,
  MCP, skills, permissions, GitHub/IDE surfaces. The features Glave needs eventual
  parity with cluster here. (Public docs only; do not use leaked source or
  leak-adjacent clones.)
- **Cursor / Windsurf** — IDE-native multi-file agents (Composer / Cascade),
  codebase indexing, tab completion. UX moat; not Glave's game.
- **Devin (Cognition)** — autonomous cloud SWE agent: planning, shell + browser +
  editor in a remote sandbox. Closest in *posture* to a server-owned autonomous
  Hope.
- **GitHub Copilot agent mode / Copilot Workspace** — issue-to-PR, distribution
  via GitHub. Distribution moat.
- **Amp (Sourcegraph), Replit Agent, Google Jules, JetBrains Junie, Factory
  Droid** — variations on autonomous/cloud/IDE agents.

Common thread: the closed players race on (a) autonomy + cloud execution,
(b) codebase-wide context/indexing, and (c) IDE-native UX. **Glave's wedge is
none of those** — it is cost-per-correct + a server-owned typed harness + a
measured flywheel. Parity matters only for the features developers now expect by
default (context files, MCP, sane permissions); the win is on cost and trust.

---

## Recommendations

**Copy (in Rust, reshaped):**
- The typed event stream as source of truth (OpenHands/Codex/Pi).
- Server-owned trust boundary: server holds secrets/policy/trace, sandbox is a
  session-keyed actuator, privileged actions are server-mediated (OpenHands).
- Sandbox + command policy as typed, deterministic, pre-action contracts
  (Codex/Gemini).
- Structural repo map (Aider) and a compaction tier (Gemini/Goose).
- LSP/typed post-condition checks injected with the edit result (OpenCode).
- Explicit `submit`-with-evidence + the eval/trace/autoscore/compare substrate
  (SWE-agent).
- Provider-normalized adapter shape (Pi `pi-ai`) and ACP subscription-reuse auth
  (Goose).

**Adapt:**
- Make every "permission popup" a deterministic server-side policy decision
  (zero-HIL): the model can only *request* escalation (Codex's model).
- Turn Cline's mechanical levers (loop detection, file-read dedupe, shadow-git
  checkpoints, per-model-family prompt selection) into **measured** knobs
  ($ saved per task), not threshold guesses.
- Make cost-per-correct a trace contract, not just a compaction counter.

**Skip:** see the Skip list above.

---

## Recommended build order (first slice)

Sharpens the direction in `refs/harnesses/README.md` with the new findings. The
first milestone is **not** "Hope can code." It is:

> Given a small task, Hope's harness runs a typed loop, executes only allowed
> actions, verifies the result, emits a replayable trace, and reports cost per
> correct task.

1. **Typed `RunEvent` stream** — append-only, the single source of truth
   (OpenHands/Codex/Pi). Include model I/O metadata, action proposed, policy
   decision, action result, artifact/diff, verification, and **cost** per step.
2. **`RunSpec`** — task type, input, workspace descriptor, allowed actions,
   budget, success criterion, model/provider config.
3. **Minimal typed action set** — `inspect_repo`, `search`, `read_file`,
   `edit_file` (validated), `run_check`, `submit` (with evidence).
4. **Server-owned policy** — sandbox + command policy resolved *before* the
   action, deterministically, no human, no model trust (Codex/Gemini/OpenHands).
5. **Deterministic in-loop feedback** — lint/LSP/typed post-conditions returned
   with the edit's own result (OpenCode/Aider).
6. **Cost accounting + budget ceiling** — every trace answers "what did it cost,"
   and a budget stop is a first-class action (OpenHands/SWE-agent).
7. **Eval substrate** — frozen config = hypothesis, fixed case set, traces,
   autonomous scoring, compare-runs; a deterministic/scripted model adapter for
   CI plus one live adapter (SWE-agent/Pi faux provider).
8. **Provider adapter** — one normalized message/action/event shape; Anthropic/
   OpenAI/Gemini/OpenRouter/local terminate there (Pi/Goose).

Then, gated by evals, add context depth (project file → repo map → selected
context → rolling summary → retrieval), and only after that the deferred surface
(MCP adapter, budgeted subagents, skills/hooks, clients, integrations).

---

## Open decisions

1. **First slice: coding-only or generic `TaskType` with coding first?**
   Recommendation: generic `TaskType`, coding first — Goose proves the
   task-general kernel is buildable without boiling the ocean.
2. **Local execution in V0?** Yes, but as an untrusted actuator behind the
   server-style run contract (OpenHands' model), even when execution is local.
3. **MCP in the first slice?** No — make the internal action registry
   MCP-adapter-ready and prove typed actions + budget + traces + verification
   first.
4. **ACP subscription-reuse auth — how early?** It's a real $/correct lever
   (Goose). Worth prototyping as a provider adapter once the normalized provider
   shape exists, but not before the trace/cost contract.
5. **Do we extend this research to per-file writeups of the closed players**
   (Cursor, Devin, Amp, Copilot agent) from public info, or keep them as the
   high-level parity context above?
