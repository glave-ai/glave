# Goal: harness feature parity (engine + parity tracks)

Self-directed build. Run autonomously until both tracks are functionally complete
or a blocker needs a human decision.

## Per-feature acceptance (the gate)
1. Mechanism implemented behind existing ports / the tool grammar — no new core coupling.
2. **High-density unit tests**: happy path + edges + negatives, I/O-free via `testkit`.
3. `cargo build` clean · `cargo clippy --all-targets` = **0** · `cargo test` green.
4. **Regression gate (fast, every feature)**: `glave validate` on the real tasks still
   discriminates (noop FAIL / solve PASS). Stub-based, no API, cheap.
5. Committed separately, descriptive message.

## Regression benchmark (periodic, per track)
Real end-to-end check: `glave run --agent model` (qwen) solves `bytes` (and `grpc-go`),
ending in `submit`. Costs API → run at track boundaries, not every feature.

## Edit surface
`crates/glave-*`, `images/`, `tasks/`, `context/`. Moat-boundary semantics
(`glave-docker` isolation) only change with a note in `context/security.md`.

## Build order (cheap/foundational first, heaviest last)
### Parity track
- [x] **HOPE.md** — repo guidance (CLAUDE.md analog) folded into agent context
- [ ] **Capability registry + selector** — lexical/BM25 substrate for the below
- [ ] **Skills** — registry-backed reusable capability packages
- [ ] **Commands** — registry-backed canned prompts/workflows
- [ ] **MCP** — external tool client (heaviest; protocol work)
### Engine track
- [ ] **Compaction** — history eviction; no-op for short tasks, bounds long ones.
      Built *with* the regression gate, since context changes burned us before.
- [ ] **Subagents** — budgeted sub-context workers (builds on registry + compaction)

## Stop condition
All items implemented + tested + regression-green, or a surfaced blocker.

## Progress log
- (start) bounded `read`, completion-only budget, fail-closed network — already landed.
- HOPE.md — `project_guidance()` reads `HOPE.md`/`.glave/HOPE.md` into initial_user
  + oneshot; 6 unit tests; regression green (both tasks discriminate).
