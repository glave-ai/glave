# Glave — Build TODOs & Worklog

> **RESUME PROTOCOL — read this first on every (re)start (e.g. after a compaction):**
> 1. Read this file top to bottom. It is the source of truth for where the build is.
> 2. Read memory: `product-architecture` (v2) + `glave-go-to-market` — the locked tenets.
> 3. Run `git log --oneline -15` on this branch to see what's actually committed.
> 4. Find the first unchecked box under the current change, re-confirm against the spec, continue.
>
> **Discipline:** update the **Status** table + the per-change **Decisions & gotchas** log after every
> meaningful step. Never mark a change done without green `cargo clippy --all-targets --all-features
> -- -D warnings` AND `cargo test`.

## Tenets that constrain HOW (full detail in memory)
- One knob = *where it runs*; isolation + materialization *follow*. Boring-correct over clever.
- **Types enforce invariants. The oracle stays SEALED from the agent — compiler-enforced, not convention.**
- Build the **seams + the session loop, then STOP**. Out of scope this round: cloud exec impl,
  OS-sandbox impl, cross-device sync, billing, MCP-beyond-existing.
- Workspace lints **DENY** unwrap/expect/panic/indexing/slicing → use `?` and
  `try_from(...).unwrap_or(...)` (the mandated idiom, not a wart).

## Decisions (baked defaults)
- Branch: `product/agent-surface` (off main; merge when green).
- Session storage: `~/.glave/sessions/<project>/`.
- Interim local safety: raw-direct + guard irreversible ops (OS-sandbox deferred).
- Offline only — InMemory for all tests, no live model. Live scorecard = a separate human-triggered step.

## Status
| # | Change | State | Commit |
|---|--------|-------|--------|
| 1 | Workspace / Grader split (CALIBRATION UNIT) | ✅ **done — green, awaiting review** | see `git log` (seal-the-oracle) |
| 2 | `Plan` construction seam | ✅ **done — green** | see `git log` (Plan seam) |
| 3 | Session engine (event-sourced, local) | ✅ **done — green** | see `git log` (session engine) |
| 4 | Interactive `glave` loop | ✅ **done — green** | see `git log` (interactive loop) |

Order by value 1 → 2 → 3 → 4. If time runs short, truncate the REPL polish (4), NEVER engine or tests.

---

## Change 1 — Workspace / Grader split  ·  **CALIBRATION UNIT (full depth)**

**Goal:** the agent depends only on a `Workspace` port; `grade` becomes an eval-only `Grader` port the
agent has no path to. The oracle-seal becomes a *type* guarantee.

**Why first:** smallest, pure refactor (no new product surface), and the foundation #2–4 stand on.
If it lands clean, the depth + rhythm is validated and we mass-produce #2–4 in this shape.

**Current state (verified by reading the code):**
- `ports.rs:129` — `WorkspaceRuntime` = read_file, write_file(&mut), list_files, run, **grade**, grep, find.
- `grade` is called in **exactly one place**: `oracle.rs:23` and `oracle.rs:29` (`rt.grade(...)`) inside `evaluate`.
- Implementors: `LocalRuntime` (`local.rs:62`, grade = run no-op), `DockerRuntime` (`docker/lib.rs:172`,
  real ephemeral-oracle-container grade), `InMemoryRuntime` (testkit).
- Call chain: engine `execute` builds rt via `RuntimeFactory::prepare` → `run.rs` `run_task` drives the
  agent over rt, then calls `oracle::evaluate(rt, task)`.

**Edits (done):**
- [x] `ports.rs`: replaced `WorkspaceRuntime` with `Workspace` (6 agent methods, **no grade**), `Grader`
  (grade only), and `pub trait EvalRuntime: Workspace + Grader {}` + blanket impl. Added a `compile_fail`
  doctest on `Workspace` + a `#[cfg(test)] mod seal`.
- [x] `local.rs`: `impl Workspace for LocalRuntime` — grade method deleted (local product never grades).
- [x] `docker/lib.rs`: `impl Workspace for DockerRuntime` + separate `impl Grader for DockerRuntime`
  (grade moved verbatim). Auto-satisfies `EvalRuntime`. Factory returns `Box<dyn EvalRuntime>`.
- [x] `testkit` (`InMemoryRuntime`): split into `impl Workspace` + `impl Grader`.
- [x] `oracle.rs`: `evaluate(grader: &dyn Grader, …)`; the two `.grade(…)` call sites updated.
- [x] `run.rs`: `run_task` takes `&mut dyn EvalRuntime`; upcasts `&mut *rt` → `&mut dyn Workspace` for the
  agent and `&*rt` → `&dyn Grader` for the oracle.
- [x] `engine.rs` + `ports.rs RuntimeFactory`: `prepare(...) -> Result<Box<dyn EvalRuntime>>`.
- [x] `lib.rs`: exports `Workspace`, `Grader`, `EvalRuntime`; `WorkspaceRuntime` removed.
- [x] `mcp.rs`, `capability.rs`, `tools.rs`, `agent.rs`: read-only/agent consumers renamed to `&dyn Workspace`.
- [x] `grep -rn WorkspaceRuntime` → **zero references remain** (fully migrated).

**Invariant — the whole point:**
- [x] The agent + tools receive **only** `&mut dyn Workspace`; `Workspace` has no `grade`, so the agent
  cannot reach the oracle — compiler-enforced.
- [x] Proven by a zero-dep `compile_fail` doctest on `Workspace` (grading through the agent handle does not
  compile). `grep Grader crates/glave-core/src/agent.rs` → none. (Chose the doctest over `trybuild` to avoid
  a network dep + brittle `.stderr` goldens; it fails loudly if `grade` is ever re-added to `Workspace`.)

**Acceptance for change 1:**
- [x] `cargo clippy --all-targets --all-features -- -D warnings` clean.
- [x] `cargo test` green: 95 unit + 1 seal doctest, 0 failures.
- [x] Oracle discrimination still works (Solve/Noop) through the `Grader` path — glave-core suite green.
- [x] Checkpoint commit (below).

**Decisions & gotchas:**
- **rustc 1.93.0** → trait upcasting is stable, so the `EvalRuntime` supertrait + `&mut *rt`/`&*rt` upcasts
  compiled first try. No generic-`run_task` fallback needed.
- `grade` had **exactly one caller** (`oracle::evaluate`), so the split was clean — no scattered grade calls.
- Seal proof = `compile_fail` doctest, **not** trybuild (see Invariant note). If we later want hard
  `.stderr`-pinned negatives, trybuild is the upgrade — deferred, not blocking.
- Refactor was mechanical + wide (12 files) but type-driven: the compiler caught every site; clippy + tests
  both green on the first full build after edits.

---

## Change 2 — `Plan` construction seam  ·  **full depth**

**Goal:** one `Plan` sum-type expresses the knob (where/how a workspace runs); remove `TaskSpec`
from the `RuntimeFactory` port (the leak); product `open(Plan)` returns a grader-free `Workspace`;
the oracle reaches the grader by a separate path, never the plan.

**Current state (verified):**
- `runtime.rs`: `Location { LocalMachine, Cloud }` + `open_runtime(Location, &Path) -> Box<dyn Workspace>`.
  Callers: only its own tests + the lib.rs export (NOT used by engine/CLI yet — it's the product seam).
- `ports.rs`: `RuntimeFactory::prepare(&TaskSpec, run_id) -> Box<dyn EvalRuntime>` — the leak.
- `engine.rs:82`: `self.runtimes.prepare(&req.task, &req.run_id)` — the one real caller (the eval engine).
- `docker/lib.rs`: `DockerRuntime::prepare(&TaskSpec, run_id)` reads workspace_dir/oracle_dir/image/resource off TaskSpec.
- Layering: glave-core builds `LocalRuntime` (in-core) but NOT `DockerRuntime` (glave-docker → glave-core, not the
  reverse). So `open()` handles `Direct` only; the sandbox is built behind the injected `RuntimeFactory`.

**Design:**
- `runtime.rs`: `Plan::Direct { folder }` | `Plan::Sandboxed { place, source, sandbox }`; `Place { Local, Cloud }`;
  `Source { CopyFrom(PathBuf) }`; `Sandbox { image, workdir, mem_mb, cpus, pids, network }`. `Cloud` lives ONLY under
  `Sandboxed` → "direct cloud" unrepresentable. No oracle anywhere in `Plan` (seal).
- `open(plan) -> Box<dyn Workspace>` (product): Direct → LocalRuntime; Sandboxed → bail (container/cloud deferred).
- `RuntimeFactory::prepare(plan: &Plan, oracle: &Path, run_id) -> Box<dyn EvalRuntime>`: TaskSpec gone; oracle is a
  plain path routed ONLY to the grader.
- `engine.rs` maps `TaskSpec → Plan` at the eval edge (`eval_plan(task)`), passes `task.oracle_dir()` separately.

**Edits (done):**
- [x] `runtime.rs`: `Location`/`open_runtime` → `Plan`/`Place`/`Source`/`Sandbox` + `open(&Plan)`; tests rewritten.
- [x] `ports.rs`: `RuntimeFactory::prepare(&Plan, &Path, &str)`; dropped `TaskSpec` import; added `runtime::Plan` + `std::path::Path`.
- [x] `docker/lib.rs`: `DockerRuntime::prepare(source, sandbox, oracle, run_id)`; factory matches the `Plan`
  (Sandboxed{Local} → build; Cloud/Direct → bail). Imports Plan/Place/Source/Sandbox + `bail`; TaskSpec dropped.
- [x] `engine.rs`: `eval_plan(&TaskSpec) -> Plan` at the eval edge; `prepare(&plan, &oracle, run_id)`; test factory sig updated.
- [x] `lib.rs`: exports `open, Plan, Place, Sandbox, Source` (dropped `open_runtime, Location`).

**Invariant:**
- [x] `grep TaskSpec crates/glave-core/src/ports.rs` → **zero** — no domain type crosses the port or `open`.
- [x] Illegal states unrepresentable: `Cloud` only under `Sandboxed`; `Plan` carries no oracle.

**Acceptance:**
- [x] clippy -D warnings clean; `cargo test` green (95 unit + 1 seal doctest).
- [x] Eval path intact: engine `execute` tests pass through the reshaped seam (TaskSpec→Plan→factory→grade).
- [x] Checkpoint commit (below).

**Decisions & gotchas:**
- Kept the product (`open`) and eval (`RuntimeFactory`) constructors **parallel, not unified** — they return
  different types (`Box<dyn Workspace>` vs `Box<dyn EvalRuntime>`) and only eval needs an oracle, so one function
  would mean an awkward `Option<oracle>` + a varying return. The shared `Plan` type IS the unification (the "one knob").
- **Layering forced the split point:** glave-core can't build `DockerRuntime` (wrong dep direction), so `open()`
  handles `Direct` and bails on `Sandboxed`; the container is built behind the injected factory. Correct + expected.
- `Plan` lives in `runtime.rs`; `ports.rs` references `crate::runtime::Plan`. Intra-crate module cross-ref — fine
  in Rust (cycles only matter across crates).
- `Source` is a 1-variant enum (`CopyFrom`) on purpose — it names the materialization strategy + marks the
  extension seam (clone-from-git later) rather than passing a bare `PathBuf`.

## Change 3 — Session engine (event-sourced, local)  ·  **full depth**

**Goal:** a session = UUID + append-only log; the transcript is REBUILT by replay (never stored as a
snapshot); persisted to `~/.glave/sessions/<project>/<id>.jsonl`; fork = pointer (parent, index) + own.

**Current state (verified):**
- `event.rs`: `RunEvent` is `#[derive(Serialize)]` only (NO Deserialize) + observability-shaped (`ModelTurn` =
  tokens+content, `AgentAction` = action/path/detail). It **cannot** rebuild a transcript and can't be read back.
- `sinks.rs`: `JsonlSink` is `TraceSink` (RunEvent-typed, write-only, flush-per-emit).
- `ports.rs`: `Message`/`Role`/`ToolCallRequest` derive only Debug/Clone — **not** serde-serializable yet.

**Design decision:** a **dedicated `SessionEvent`** (Serialize + Deserialize, wrapping real `Message`s), NOT an
extension of `RunEvent`. Reason: RunEvent is observability + Serialize-only; conversation-reconstruction needs the
structured messages and round-trip (de)serialization. Two concerns, two logs (the RunEvent trace stays as-is).

**Edits (done):**
- [x] `ports.rs`: `Message` + `ToolCallRequest` derive `Serialize, Deserialize, PartialEq`; `Role` derives serde
  (+ `rename_all = "snake_case"`). Added the serde import.
- [x] new `session.rs`: `SessionEvent` (`Started { forked_from }` | `Message(Message)`), `ForkPoint`, pure `Session`
  (`new`/`push`/`transcript`/`fork`/`own_messages`), and `SessionStore` (`save`/`append_message`/`load` + recursive
  fork composition; `default_root` = `~/.glave/sessions`).
- [x] `lib.rs`: `pub mod session;` + exports `Session, SessionStore, SessionEvent, ForkPoint`.
- [x] Lints satisfied — `.iter().take(n)` / `.skip(n)`, no indexing/slicing/unwrap.

**Invariant:**
- [x] Round-trip proven (`save_then_load_round_trips_the_transcript`): N messages incl. tool_calls/tool_call_id reload identical.
- [x] Fork is a POINTER (`fork_replays_parent_prefix_plus_own`): fork file = `Started` + 1 own line (NOT the 2 inherited);
  replay = parent[0..2] + own; parent untouched. Clamp covered.

**Acceptance:**
- [x] clippy -D warnings clean; `cargo test` green — 4 new session tests (round-trip, incremental, fork, clamp) pass.
- [x] Checkpoint commit (below).

**Decisions & gotchas:**
- Dedicated `SessionEvent` (NOT extending `RunEvent`): RunEvent is observability + `Serialize`-only and can't rebuild a
  transcript; the session log needs structured messages + round-trip (de)serialization. Two concerns, two logs.
- `SessionEvent` is serde **internally-tagged** (`tag="kind"`) with a newtype `Message(Message)` variant — works
  because `Message` serializes as a struct/map; the round-trip test confirms it.
- Fork prefix is **recomputed from the parent on load**, never written to the fork's file → true pointer semantics
  (verified by counting the fork file's lines).
- `save` (full, truncating) + `append_message` (incremental, durable) both provided: the loop will `save` once on
  create, then `append_message` per turn (wired in #4).

## Change 4 — Interactive `glave` loop  ·  **full depth**

**Goal:** a `glave chat` that opens the cwd as a `Direct` workspace, drives a model turn over a
persisted session, and loops — the product surface, on `open(Plan)` + the session engine.

**Current state (verified):**
- glave-cli is a clap subcommand harness (BuildBase/Run/Validate/Ingest/Serve/Score); no interactive path.
- `ModelAgent::act` is eval-coupled (`TaskSpec`, oracle, bug-fix system prompt) — not reusable as-is for chat.
- `tools::execute(call, rt, _task: &TaskSpec, viewed)` takes an UNUSED `&TaskSpec` — a latent leak.
- `AutonomousPolicy::new(&[])` = allow-all (empty editable surface) — the right local-product default.

**Design:**
- glave-core `session.rs`: a `Turn` trait (`respond(transcript, workspace) -> Vec<Message>`) — the seam
  between the loop and whatever drives a turn (a model in the product, a fake in tests) — and a `step`
  driver (record user msg → `respond` → append + persist all). Pure + offline-testable.
- `tools::execute`: drop the unused `&TaskSpec` param (+ update agent.rs's 2 callers).
- glave-cli `chat`: resolve/create `.glave/project.json` (uuid); `open(Plan::Direct{cwd})`; `SessionStore`
  rooted at `~/.glave/sessions/<project>`; new-or-`--resume` session seeded with a product system prompt;
  REPL read→`step`→print; `-p` one-shot. A `ModelTurn` (provider + AutonomousPolicy(&[]) + tool loop)
  implements `Turn`.

**Edits (done):**
- [x] `tools.rs`: dropped the unused `_task: &TaskSpec` from `execute` + its import. `agent.rs`: 2 callers updated, and
  `run_subagent`'s now-unused `task` param removed (caught by clippy).
- [x] `session.rs`: `Turn` trait + `step(...) -> Vec<Message>` (record user → respond → append + persist all) + test.
- [x] `lib.rs`: exports `Turn`, `step`.
- [x] glave-cli: `cmd: Option<Cmd>` (bare `glave` → chat) + `Chat` subcommand; `resolve_or_create_project` (UUID
  marker); `ModelTurn` (provider + AutonomousPolicy(&[]) + tool loop); REPL (`/exit`, EOF) + `-p` one-shot.

**Invariant:**
- [x] The loop drives the agent over a `Box<dyn Workspace>` from `open(Plan::Direct{cwd})` — no grader, no TaskSpec.
- [x] `step` round-trips offline (`step_records_user_then_turn_and_persists`): user + turn persist, reload in order.

**Acceptance:**
- [x] clippy -D warnings clean; `cargo test` green (loop step test passes).
- [x] `glave --help` / `glave chat --help` confirm the wiring (bare `glave` → chat; `-p`/`--resume`/`--model`).
      Live model turn exercised by the human (offline run can't reach the endpoint).
- [x] Checkpoint commit (below).

**Decisions & gotchas:**
- The product turn is a **new `Turn` seam**, NOT `ModelAgent` (which is eval-coupled: TaskSpec, oracle, bug-fix
  prompt). The loop core is model-agnostic + offline-tested; `ModelTurn` (the real provider-backed turn) is wired and
  clippy-clean but **not live-tested in this offline run** — validate it live against the endpoint.
- Dropped the unused `&TaskSpec` from `tools::execute` so the product turn runs tools without a fake task — a real
  de-leak, not just plumbing.
- Local product policy = `AutonomousPolicy::new(&[])` (allow-all): the local repo is trusted (the OS-sandbox rung is
  the deferred hardening). Subagent/MCP are explicitly deferred in chat (print a "not available yet" note).
- Bare `glave` → chat via `Option<Cmd>` — keeps the existing eval subcommands intact while making the product surface
  the default.

## Acceptance across all changes (brutal) — ✅ all green
- [x] Property/adversarial on every load-bearing path: event-log round-trip + incremental (session); fork + clamp;
  `Plan` valid + cloud-stub-errors (runtime); LocalRuntime path-escape + in-place edits (local); measurement-integrity
  guards (openrouter, 23 tests); the agent-can't-reach-`Grader` `compile_fail` doctest.
- [x] End-to-end (InMemory runtime, NO GPU): `end_to_end_edit_resume_fork` — a turn edits the workspace, the session
  persists + resumes from disk, then forks and diverges while the parent stays intact.
- [x] No-regression: the engine path (`engine.execute` → `run_task` → agent over Workspace + grade via Grader) runs
  offline through the refactored Plan + Workspace/Grader seams — `engine_runs_*` tests green.

**Final state:** clippy `--all-targets --all-features -D warnings` clean; **101 unit tests + 1 seal doctest, 0
failures** (glave-core 75, openrouter 23, serve 2, cli 1). Branch `product/agent-surface`, 5 checkpoint commits.
Live `glave chat` model turn is wired + clippy-clean but awaits a live-endpoint run (offline build couldn't call it).

## Review fixes (local max-effort code-review, 2026-06-14)
Local review surfaced 15 findings (the refactor itself = **zero** regressions; bugs clustered in the new
product loop + session robustness). Fixed in three green gates:
- **Gate A — security** (`aeb82c4`): session-id + project-id path-component validation; atomic marker write +
  no silent re-ID; destructive-bash gated behind `Ask`/human-confirm (interim — OS-sandbox is the real boundary).
- **Gate B — product loop** (`2a36622`): submit ends the turn; `viewed` seeded from transcript; empty-turn nudge;
  step-budget signal.
- **Gate C — session robustness** (`1992b53`): fork cycle/depth guard; tolerate a torn trailing line; header-first;
  persist-then-push in `step`; `save` initialize-once contract.

**Deferred (deliberate, tracked):**
- **#13 — extract a shared agent loop** so `ModelTurn` stops duplicating `ModelAgent`. The deep fix, but it churns
  the battle-tested eval path; do it as its own reviewed change, not a fix-sweep.
- **Polish** — `append_message` re-opens per message (O(messages) syscalls); per-turn `to_vec` + clone churn.
  Micro-optimizations, not worth the churn yet.
- **`ModelTurn` live run** — still needs a live-endpoint test.

## Deferred — do NOT build this round
cloud execution impl · OS-sandbox · cross-device sync · billing · MCP-beyond-existing · live scorecard run.
Spot a real follow-up? Note it here, don't build it.
