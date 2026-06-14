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
- Branch: `product/local-runtime` (continue; merge when green).
- Session storage: `~/.glave/sessions/<project>/`.
- Interim local safety: raw-direct + guard irreversible ops (OS-sandbox deferred).
- Offline only — InMemory for all tests, no live model. Live scorecard = a separate human-triggered step.

## Status
| # | Change | State | Commit |
|---|--------|-------|--------|
| 1 | Workspace / Grader split (CALIBRATION UNIT) | ☐ not started | — |
| 2 | `Plan` construction seam | ☐ not started | — |
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

**Edits (check off as done):**
- [ ] `ports.rs`: replace `WorkspaceRuntime` with:
  - `pub trait Workspace { read_file; write_file(&mut self,…); list_files; run; grep; find }` — the 6 agent methods, **no grade**.
  - `pub trait Grader { fn grade(&self, cmd: &str) -> Result<ExecResult>; }` — eval-only.
  - `pub trait EvalRuntime: Workspace + Grader {}` + blanket `impl<T: Workspace + Grader> EvalRuntime for T {}`
    — the eval path needs both views from one object. (Relies on **trait upcasting** → needs **Rust ≥ 1.86**;
    run `rustc --version` and record it in Decisions. Fallback if older: make `run_task` generic
    `<R: Workspace + Grader>` instead of using `dyn EvalRuntime`.)
- [ ] `local.rs`: `impl Workspace for LocalRuntime` — **delete the `grade` method** (the local product never grades).
- [ ] `docker/lib.rs`: `impl Workspace for DockerRuntime` (the 6) + `impl Grader for DockerRuntime`
  (move `grade` here, body unchanged). DockerRuntime then auto-satisfies `EvalRuntime` via the blanket impl.
- [ ] `testkit` (`InMemoryRuntime`): `impl Workspace` + `impl Grader` so eval/discrimination tests keep working.
- [ ] `oracle.rs`: change `evaluate(rt: &dyn WorkspaceRuntime, …)` → `evaluate(grader: &dyn Grader, …)`.
- [ ] `run.rs` (**READ IT FIRST**): `run_task` takes the eval runtime as `&mut dyn EvalRuntime`; hand the
  **Workspace** view to the agent (upcast `rt as &mut dyn Workspace`) and the **Grader** view to
  `oracle::evaluate` (upcast `rt as &dyn Grader`).
- [ ] `engine.rs` + `ports.rs RuntimeFactory`: `prepare(...) -> Result<Box<dyn EvalRuntime>>` for now.
  (Change #2 will replace `RuntimeFactory` with `Plan` — here just widen the return type so the split lands.)
- [ ] `lib.rs`: export `Workspace`, `Grader`, `EvalRuntime`; remove the `WorkspaceRuntime` export.
- [ ] `grep -rn WorkspaceRuntime` across the workspace and fix every remaining reference
  (glave-serve, glave-cli, glave-docker, all tests).

**Invariant — the whole point:**
- [ ] The agent (`agent.rs` ModelAgent + tools) and the future product loop receive **only** `&mut dyn Workspace`.
  `Workspace` has no `grade`, so the agent literally cannot reach the oracle — enforced by the compiler.
- [ ] Prove it: a test/assertion that the agent path never names `Grader` (e.g. a `trybuild` compile-fail that
  `<dyn Workspace>::grade` does not resolve, or at minimum confirm `agent.rs` imports `Workspace`, never `Grader`).

**Acceptance for change 1:**
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test` green across all crates.
- [ ] Oracle discrimination still works (Solve solves, Noop doesn't) through the `Grader` path.
- [ ] Checkpoint commit: `Workspace/Grader split: seal the oracle from the agent by type`.

**Decisions & gotchas (fill in as you go):**
- _(rustc version + whether trait upcasting was used or the generic fallback)_
- _(anything surprising in run.rs / glave-cli wiring)_

---

## Change 2 — `Plan` construction seam  ·  *(flesh out to full depth after #1 calibrates)*
- [ ] Replace `Location` + `open_runtime` (`runtime.rs`) AND `RuntimeFactory::prepare(task,…)` with one `Plan`:
  `Direct { folder }` → LocalRuntime (in-place, no container); `Sandboxed { place, source, box }` → container
  (`place = Local | Cloud`-stub; `source` = materialization, CopyFrom(path) for eval; `box` = image+resources).
- [ ] Product constructor `open(plan) -> Result<Box<dyn Workspace>>`; exhaustive match, Cloud arm bails clearly.
- [ ] Eval edge builds the `EvalRuntime` from a `Plan::Sandboxed` + maps `TaskSpec → Plan` THERE; oracle dir goes
  only into the `Grader`, never the Plan/Workspace. Invariant: no domain type (TaskSpec) in any port or in `open`.

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
