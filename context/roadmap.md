# Glave v2.0 Roadmap

Living plan for the harness + self-improvement flywheel rebuild.
Branch: `upgrade/v2.0` · Last updated: 2026-05-29

This is the operational source of truth. Strategy lives in `sacred/`; deeper
design rationale in `context/architecture/hope-surface-area.md`,
`context/research/self-improvement-flywheel.md`, and
`context/research/harnesses/executive-summary.md`.

---

## North star

Glave is an **econo-frontier** company: deliver **~90% of frontier value at 3–5%
of frontier price**. The product is a **self-improvement flywheel**; the harness
is the IP; the metric is **cost per correct completed task**.

The promise is a *coupled* pair on the same tasks: **≥0.9 of frontier's success
AND ≤0.05 of frontier's cost-per-correct, simultaneously.** You can't buy price
by sacrificing success, or value by burning tokens.

## Operating principles (locked)

- **Assume-frontier design, plug in cheap.** Build a clean, general harness;
  close the capability gap with the *loop*, not hand-engineered crutches. Default
  model: `qwen/qwen3.6-27b`, **reasoning on**.
- **The promise is met by the route, not the model.** Cheap model for the bulk;
  escalate the hard tail rarely. `model_route` is a flywheel surface.
- **Server-owned harness.** Clients and containers are untrusted actuators.
- **Owned, real-world evals are the lifeblood** — not public benchmarks
  (contaminated, distribution-skewed). Grade on *behavior*, never diff-match.
- **Two rules of order:** signal before polish; guard the ruler before building
  the optimizer.
- **Rust for all implementation.** Only non-Rust: eval fixtures (their target
  language) and Dockerfiles (config).
- **Reasoning is first-class** — enabled explicitly, reasoning tokens measured.
- **Minimalism.** Smallest thing that produces a real number; gate features by
  measured evals.

## Architecture (decided)

- **Harness** = Rust binary: runner + gateway + oracle + trace.
- **Gateway** = the single chokepoint for model calls. Owns routing, reasoning,
  cost metering, audit. Pluggable source: OpenRouter now → local vLLM / frontier
  leg later, by config.
- **`WorkspaceRuntime` trait** = the execution seam. Docker now (ephemeral,
  `--network none`, read-only oracle mount, resource caps); Firecracker / cloud
  later with no harness rewrite.
- **Task contract**: `workspace/` (rw, editable) · `oracle/` (ro, two-sided
  `FAIL_TO_PASS` + `PASS_TO_PASS`) · `solution/` (reference, validation only).
  Image carries *deps*; mounts carry *code*. Per-task images (`FROM <lang>-base`)
  when a task needs dependencies.
- **Traces**: flat-file JSONL, run-id correlated, two halves (runner +
  model/gateway). The trace corpus is a compounding asset.
- **Languages**: Python, TypeScript, Go, Rust.

## Phases

| Phase | Goal | Status |
| --- | --- | --- |
| **0 — Deterministic skeleton** | Run one task in an isolated container, two-sided oracle, JSONL trace, stub agents | ✅ done (`b4a0ebe`) |
| **1 — First real number** | Gateway → OpenRouter, real agent loop, reasoning on, cost recorded | ✅ done (uncommitted) |
| **1.5 — Pin the provider** | Kill OpenRouter provider drift (`provider.order` + no fallbacks); record provider | ⏭ next |
| **2 — Task variety** | ~8–12 tasks × 4 languages × easy/medium, incl. dependency-bearing → per-task images + `bake`, multi-lang oracles | |
| **3 — Concurrency** | ≤8 ephemeral containers in parallel → scorecard; prove the footprint claim | |
| **4 — Frontier baseline + scorecard** | Run the frontier model on the same suite; relative success + relative $/correct vs (0.9, 0.05) | |
| **5 — Guard the ruler** | Enforce `editable_surface`; oracle-gaming defenses; digest-pin images; richer failure detail in traces; sealed hidden split | |
| **6 — Close the loop** | Champion manifest (hash-addressed components), proposer over a bounded surface, compare-runs gate, promote-on-win, hidden-eval promotion | |

Phases 1–4 take us from "the machine runs" to "the machine measures the bet."
5 hardens it. 6 is the actual flywheel — the product.

## Current state

- Branch `upgrade/v2.0`. Slice 1 committed (`b4a0ebe`); Phase 1 (gateway + model
  agent + reasoning) built and working, **uncommitted**.
- **First real number:** `qwen/qwen3.6-27b` @ reasoning=high solves the
  `python/easy fix-failing-test` task in 3 turns, 1,721 tokens (219 reasoning),
  **$0.00146**, ~9s.

## Open decisions

- Commit Phase 1 to `upgrade/v2.0`.
- Which OpenRouter provider to pin for `qwen3.6-27b` (Phase 1.5).
- Which model is our **frontier reference** for the baseline ratio (Phase 4).
- When to introduce per-task images / `bake` (Phase 2, at the first dep-bearing task).
- Trace retention policy (traces/ currently local + gitignored).
