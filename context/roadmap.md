# Glave v2.0 Roadmap

Living plan for the harness + self-improvement flywheel.
Branch: `upgrade/v2.0` · Last updated: 2026-05-29

Operational source of truth. Strategy lives in `sacred/`; deeper design rationale
in `context/architecture/hope-surface-area.md`,
`context/research/self-improvement-flywheel.md`, and
`context/research/harnesses/executive-summary.md`.

---

## North star

**Econo-frontier:** deliver **~90% of frontier value at 3–5% of frontier price.**
The product is a **self-improvement flywheel**; the harness is the IP; the metric
is **cost per correct completed task**. The promise is a *coupled* pair on the
same tasks: ≥0.9 of frontier's success AND ≤0.05 of its cost-per-correct.

## Operating principles (locked)

- **Assume-frontier design, plug in cheap.** Default model `qwen/qwen3.6-27b`,
  **reasoning on**. Close the gap with the loop, not hand-engineered crutches.
- **The promise is met by the route, not the model** — escalate the hard tail.
- **Server-owned harness; client/actuator is untrusted.** Moat = the brain.
- **Owned, real-world evals are the lifeblood** — behavior-graded, not katas, not
  contaminated. Public benchmarks are a thermometer, never the optimization target.
- **Tool richness ≠ codebase size.** Even small codebases need ranged reads,
  patching, and bash. Build the real tool surface, not a toy.
- **Two eval types, kept separate:** *harness evals* (fixed model, optimize the
  harness — what we're doing) vs *model evals* (training/fine-tuning — later).
  A harness-ruler task must never become model-training data.
- **Zero-HIL is scoped to the eval loop** (autonomous, no human). The *product*
  supports human-in-the-loop — same `Policy`, different mode.
- **Two rules of order:** signal before polish; guard the ruler before the optimizer.
- **Rust for all implementation.** Strict/disagreeable lints (clippy 0/0).

## Architecture (as built)

Hexagonal monorepo (`crates/`):
- **`glave-core`** — domain + ports (`ModelProvider`/`WorkspaceRuntime`/`TraceSink`),
  the tool grammar, mediation `Policy`, agent loop, oracle, orchestration,
  in-memory fakes. Depends only on ports → moat boundary + testability are
  compiler-enforced.
- **`glave-openrouter`** — `ModelProvider` adapter (reasoning + cost).
- **`glave-docker`** — `WorkspaceRuntime` adapter (sandbox + hardening).
- **`glave-cli`** — composition root, bin `glave`.

**Tool grammar** (modeled on Anthropic's surface): `view` (ranged) · `str_replace`
(exact patch) · `create` · `bash` · `grep`/`glob` (git-aware, ripgrep) ·
`run_tests` · `submit`. Read-before-edit enforced; edits mediated by `Policy`.

**Task contract**: `workspace/` (rw) · `oracle/` (ro, two-sided FAIL/PASS shell
commands) · `solution/` (reference). Fields incl. `difficulty`, `codebase_size`,
`editable_surface` (enforced), provenance/calibration. Per-language base images.

---

## Phases

### ✅ Done — the harness core
Hexagonal monorepo · strict Rust posture · real tool grammar + mediation ·
4-language eval foundation (Python/Go/Rust/TS) with real-world bug-fix tasks
(oracle-validated, model-solved) · 1 medium multi-file task · ~3,000 real-world
tasks acquired (SWE-bench + Multi-SWE-bench) as authoring source.

### ▶ Now — Measure (first real signal)
- **`glave score`**: run a task-set through cheap *and* frontier models; emit
  per-task + aggregate solve rate, tokens, $, and the two ratios that *are* the
  promise — relative success, relative $/correct.
- **Pin the OpenRouter provider** (kill routing drift) so comparisons are clean.
- Run on the current tasks → first real read on the 90%/3–5% bet (the cost ratio
  on tasks both solve is already the price half).

### Next — Grow the gradient (the corpus)
- Calibrate → **workflow** authoring (author → critic → Docker-validate gate).
- low / medium / high difficulty × {Python, Go, Rust, TS}, codebase small (1–10
  files), **biased toward high-difficulty** (where the cheap-vs-frontier gap
  opens — the baseline says where to aim). ~40–60 tasks.
- Sealed **hidden split** from the start.

### Then — Guard the ruler (hardening)
Digest-pin base images (record in trace) · oracle-gaming defenses · richer
per-test failure detail in traces · formalize the hidden split. Must precede the
proposer loop.

### Then — Close the loop (the flywheel)
Champion manifest (hash-addressed prompt / tool-policy / model-route) · a proposer
that edits a bounded surface · compare-runs gate · promote-on-win · hidden-eval
promotion. **This is the product.**

### Horizon
- Model-eval / fine-tuning track (separate corpus, train/val/hidden splits).
- Large-codebase / context-management tier (repo map, selective context) — where
  the deferred context lever gets measured.
- Product surface: cloud client/server (runs API, WebSocket, `.hope/`), HIL modes,
  the parallel runner (8–16 containers).

---

## Current state

- `upgrade/v2.0`, all committed. Latest: `0e57037` (grep/glob).
- Harness core complete and verified (clippy 0/0, unit tests, model solves the
  4-language smoke set + the medium task end-to-end).
- **Smoke set, not a distribution:** ~5 tasks, all solved by the cheap model. No
  frontier comparison yet → the bet is unmeasured. That's what "Now" fixes.

## Open decisions

- **Which model is the "frontier" reference** for the baseline (defines the
  promise's denominator). — *blocks "Now".*
- Authoring scale: workflow vs batches (leaning workflow with a bounded
  validation gate).
- When to add the `insert` tool, the escalation route, the parallel runner.
- Trace retention policy.

## Known debt (deferred, by design)

Type-design review (2026-06) resolved the cheap warts in-place — `Message.role`
is now a `Role` enum, and write-only fields (`codebase_size`, `provenance` field,
`measured_peak_mb`, `p2p_output`) are gone. Two are deliberately deferred because
each is coupled to a larger piece, not a standalone fix:

- **Trace enrichment** — the trace records the model's *output* + action names,
  but not the *observations it saw*, the *prompts sent*, or the *harness config*.
  The shape (append-only event log) is right; it needs enrichment to be a
  replayable flywheel dataset. **Blocked on `HarnessConfig`**: real config-
  attribution needs a config object to stamp (the tunable surface is still
  hardcoded/env). Do trace-enrichment + `HarnessConfig` together as one
  flywheel-foundation pass (audit items #4 + #5), when we prioritize the flywheel.
- **Durable storage** — traces are local JSONL files; tasks are local dirs. Fine
  for now. Going to a DB/object store is a `TraceSink` adapter, not a brain change
  — it belongs with the **commercial layer** (where per-tenant metering lives),
  not before the product that needs it.
- **Native search — DONE on the eval side** (`de41d3d`). `grep`/`glob` are now a
  `WorkspaceRuntime` op implemented in Rust via ripgrep's crates (`grep`,
  `ignore`, `globset`) over the host bind-mount — gitignore-aware, vendoring-
  skipping, `rg` dropped from all 4 images. Live-verified (qwen solved bytes-auto
  using native grep). *Remaining for full eval↔product parity:* the local-client
  reuses the same module (extract to a shared crate when the client is built).

## Next up: prove the thesis (the measurement)

Everything is staged; this is the next focused effort (a multi-hour curation +
validation grind — deserves fresh focus):

- **Decided:** frontier baseline = **Opus 4.8, high reasoning** via OpenRouter
  (`anthropic/claude-opus-4.8`; both legs run the same harness at the same
  reasoning — fair armed-vs-armed; the Opus leg is the real $ cost).
- **Task bar:** real, recent (2026, non-contaminated) merged PRs from established
  real-usage repos, with a held test, contained — **weighted to everyday work**
  (Django bug / React component / script level), a *few* harder for spread. NOT
  skewed hard (over-taxing difficulty understates the pitch). No toy/AI-slop repos.
- **Languages:** all 4 (Python, Go, Rust, TypeScript) — ingest now supports them;
  base images built (rust 1.44GB, ts 485MB, python 384MB; go builds on demand).
- **Sequence:** (1) VALIDATE py/ts gating end-to-end first — the `derive`/`vendor`
  logic (pytest oracle, in-container venv, npm install) is unverified until a real
  task gates; expect dep-iteration. (2) Curate ~10 tasks (spread, 4 langs),
  validate-gate each (noop fails / solve passes — zero-HIL). (3) `glave score
  <tasks> --baseline anthropic/claude-opus-4.8 --trials 1-2` → the `$/correct`.
- **Disk-safe:** bounded base images; per-task vendored deps cleanable; containers
  `rm -f -v` (no leftovers). Clean: `docker rm -fv $(docker ps -aq --filter
  name=glave) 2>/dev/null; rm -rf .runs .ingest`.
