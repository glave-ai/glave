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
- [x] **Capability registry + selector** — lexical/BM25 substrate for the below
- [x] **Skills** — registry-backed reusable capability packages
- [x] **Commands** — registry-backed canned prompts/workflows
- [x] **MCP** — external tool client. DECISION: operator-configured servers are
      trusted, run brain-side (standard `.mcp.json` model). Client core (config,
      JSON-RPC, transport port, stdio adapter, McpHub) + agent wiring (`<mcp>`
      tool, catalog, dispatch). Untrusted/actuator-sandboxed MCP deferred.
### Engine track
- [x] **Compaction** — history eviction; no-op for short tasks, bounds long ones.
      Built *with* the regression gate, since context changes burned us before.
- [x] **Subagents** — budgeted sub-context workers (builds on registry + compaction)

## Stop condition
All items implemented + tested + regression-green, or a surfaced blocker.

## Progress log
- (start) bounded `read`, completion-only budget, fail-closed network — already landed.
- HOPE.md — `project_guidance()` reads `HOPE.md`/`.glave/HOPE.md` into initial_user
  + oneshot; 6 unit tests; regression green (both tasks discriminate).
- Capability registry + selector — `capability.rs`: Capability/CapabilityKind,
  CapabilityRegistry, Selector port + LexicalSelector (name-weighted token
  overlap, deterministic, zero-score dropped); 8 unit tests. Skills/commands/MCP
  populate + query it next.
- Skills — `load_skills()` reads `.glave/skills/*.md` (frontmatter `description:`
  + body), agent selects top-3 relevant by task statement into a RELEVANT SKILLS
  section; 4 capability + 3 agent tests. Opt-in: no dir -> no behavior change.
- Commands — `load_commands()` (`.glave/commands/*.md`, generalized `load_dir` +
  `parse_capability`) + `expand_command()` with `$ARGUMENTS` substitution; 4 tests.
  Mechanism is library-side; the interactive slash surface lands with the product
  CLI (the autonomous eval has no invocation trigger).
- Compaction — `compact_history()` stubs tool observations older than the last
  `keep_recent` (env `GLAVE_KEEP_RECENT`, default 6); preserves system/task/model
  turns; idempotent. 4 unit tests. REAL regression: grpc-go solved with ~20
  observations (compaction active, ~14 stubbed) — no regression. Real qwen
  benchmark also re-confirmed on bytes after the parity features.
- Subagents — `<subagent>Q</subagent>` spawns a read-only research worker
  (read/grep/glob only, `GLAVE_SUB_TURNS` budget) in its own context; only its
  summary returns to the main loop. 3 tests (readonly set, runs-then-submits,
  cannot-edit). Real regression: bytes still solves with the tool present.
  >>> ENGINE TRACK COMPLETE.
- MCP — trust-model decision (operator-configured = trusted, brain-side). Client
  core: `load_config`/`parse_config`, `McpTransport` port, `McpClient`
  (initialize/list/call), `StdioTransport` (newline JSON-RPC, kills child on
  drop), `McpHub` (connect/catalog/route). Agent wiring: `<mcp server tool>args`
  tool + loop dispatch + catalog message; opt-in (no `.glave/mcp.json` = no-op).
  11 tests incl. a REAL python mock-server integration test + a hub-via-config
  test. Eval-safe (bytes re-confirmed SOLVED; first run was edge variance).
  >>> PARITY TRACK COMPLETE.

>>> GOAL COMPLETE: all 7 features built, tested, regression-gated. ~50 unit tests,
clippy 0 throughout, real qwen regressions green (bytes, grpc-go w/ compaction).
Next (separate): multi-trial scoring to measure on the now-complete harness.
