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
| 3 | Session engine (event-sourced, local) | ☐ not started | — |
| 4 | Interactive `glave` loop | ☐ not started | — |

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

## Change 3 — Session engine (event-sourced, local)  ·  *(flesh out after #1)*
- [ ] Session = UUID + append-only RunEvent log; transcript REBUILT by replay, never stored; **log events, not requests**.
- [ ] Replay reconstructs the EXACT `Message` array (user / assistant-with-tool_calls / tool results) — add
  message-level events if RunEvent today is observability-shaped, not resume-shaped.
- [ ] Store append-only → `~/.glave/sessions/<project>/<session_id>.jsonl` (reuse `JsonlSink`); load = replay.
- [ ] Fork = new session with `(parent_session_id, fork_index)` + own events; replay = parent[0..index] + own
  (pointer, not copy, not content-addressing). Invariant: write→reload round-trips the transcript exactly.

## Change 4 — Interactive `glave` loop  ·  *(flesh out after #1)*
- [ ] Bare `glave` → interactive session in cwd; resolve project via `.glave/project.json` (create if absent);
  create or resume a session.
- [ ] Loop: read → append → run engine w/ Direct runtime (LocalRuntime on cwd) → stream RunEvents → append → repeat.
- [ ] `-p "…"` / positional = one-shot; `--resume <id>` / `-c` = continue-last. Autonomous-by-default via
  AutonomousPolicy; gate irreversible ops. Thin line-based REPL — the engine is the substance.

## Acceptance across all changes (brutal)
- [ ] Property/adversarial tests on every load-bearing path: event-log append/replay round-trip; fork correctness;
  `Plan` valid + invalid (cloud stub errors clearly); session resume round-trip (N events → reload → identical);
  LocalRuntime path-escape (`../` + symlink rejected) + in-place edits; measurement-integrity guards still pass;
  the agent-can't-reach-`Grader` compile fact.
- [ ] End-to-end via InMemory provider + Direct/InMemory runtime (NO GPU): scripted session edits a temp file,
  resumes, forks — all asserted.
- [ ] No-regression: scorecard path compiles + runs through the refactored engine OFFLINE.

## Deferred — do NOT build this round
cloud execution impl · OS-sandbox · cross-device sync · billing · MCP-beyond-existing · live scorecard run.
Spot a real follow-up? Note it here, don't build it.
