# AGENTS.md

Rules for everyone who changes this repository, human or agent. Read this before every change. When a rule changes, update this file in the same pull request.

## What Glave is

Glave is the open-source user agent for AI: one binary that runs anywhere (a laptop, a company's servers, Glave Cloud), works with any model, and ships Hope as its free house model. People get simple defaults; companies plug in their own identity, storage, models and compute. Both run the same code.

## Founding principles

1. **Library first.** Every capability is a crate you can depend on. The `glave` binary is one way to wire them together; anyone can build their own.
2. **Every socket has a protocol.** A port can be satisfied in-process (a crate) or across a boundary (any service that speaks the protocol). The same conformance tests verify both.
3. **Transport is configuration.** One process by default. When pieces split: stdio or Unix sockets on one machine, TCP + TLS across machines. Same protocol everywhere.
4. **Consumer defaults, enterprise plugs.** No enterprise feature may add a single click for a consumer. If it would, it belongs in a plug.
5. **One of each.** One storage backend per deployment, one identity source, one event log. No component brings its own database.
6. **One job each.** Every piece owns its data; anything else that needs that data asks through its port.
7. **Standards first.** OpenAI-compatible API (models), MCP (tools), A2A (agents), OIDC/SAML/SCIM (identity), S3/Postgres (storage), AGENTS.md (instructions). Define our own only where none exists: the event log, receipts, computer lifecycle, eval reports.
8. **Subtract before adding.** Nothing optional is on by default unless it is invisible and harmless.
9. **Measure, don't guess.** Design and performance decisions are settled by evals and benchmarks.

## Concepts

| Term | Meaning |
| --- | --- |
| Space | The scope boundary: memory, history, files, members, policy, budget, default environment |
| Thread | A conversation or task in a space. Workers fork from a parent with permissions and budget ⊆ the parent's |
| Environment | A reusable template for where work runs: computer type, image, setup, network, credentials, limits |
| Computer | A running instance of an environment: created, snapshotted, slept, woken, destroyed |
| Harness | The agent loop: context, tool calls, approvals, receipts, threads, scheduling |
| Gateway | The single door out for models and remote tools: routing, keys, policy, budgets, metering |
| Data store | Everything durable: the event log (history), memory, files, config. Enforces scope on every query |
| Hope | Glave's open model. Great in any harness, best in Glave |

## Repository

```
Cargo.toml          workspace: lints, profiles, shared dependencies
xtask/              developer automation (`cargo xtask`)
crates/event/       glave-event: the versioned event format (envelope, payloads, upgrade-on-read)
crates/ports/       glave-ports: event log, blob store, model provider, tool source and computer ports, the clock and entropy, in-memory versions, conformance suites
crates/identity/    glave-identity: who is calling: the verified caller, OIDC and local sign-in, the person an issuer's subject is
crates/config/      glave-config: config and policy (specs/policy.md): choices nearest-wins, constraints that only narrow, project trust, `policy explain`; shared by the CLI and the servers
crates/provider-openai/  glave-provider-openai: OpenAI-compatible streaming provider
crates/provider-anthropic/  glave-provider-anthropic: Anthropic's native Messages API provider (thinking sent back signed)
crates/gateway/     glave-gateway: the in-process gateway (specs/gateway.md): routes by purpose, safe fallback, budgets, metering
crates/gateway-service/  glave-gateway-service: the gateway as a service (`glave gateway serve`): OpenAI and Anthropic surfaces, OIDC sign-in (glave-identity), per-caller policy, budgets and metering, the MCP tool catalog at /mcp
crates/keys/        glave-keys: the key hierarchy (root, organization, space, data keys), sealing at rest, erasure; roots in the OS keychain (`keychain`) or a key file
crates/store-sqlite/  glave-store-sqlite: the event log, blob store and wrapped keys on one local SQLite file, content sealed
crates/store-postgres/  glave-store-postgres: the event log, blob store and wrapped keys on Postgres, an organization's one data store (`[store] url`), TLS off this computer, content sealed
crates/shell/       glave-shell: shell commands read before they run (tree-sitter-bash): read-only, risky with a reason, or neither
crates/mcp/         glave-mcp: MCP servers as tool sources, through the official rmcp SDK (a test server behind `test-server`); open-world tools and servers marked untrusted return untrusted output
crates/memory/      glave-memory: memory (specs/memory.md): an in-process MCP server over the data store, each memory sealed under its own key, scoped by the session; a conformance suite for memory servers
crates/web/         glave-web: web_fetch as a tool source: private addresses refused at every hop, limits, a host policy, HTML as Markdown, output untrusted (a local page server behind `pages`)
crates/harness/     glave-harness: the agent loop, its six tools (search, glob, read, write, edit, bash) run on a computer through the port, approvals, resume, AGENTS.md, skills (agentskills.io), custom commands, hooks
crates/computer/    glave-computer: computers (specs/computers.md): this machine as one (in the sandbox by default), environments from devcontainer.json or environment.toml, workers on Docker or Kubernetes (the `kubernetes` feature), served (`glave computer serve`, TLS across machines) and reached over JSON-RPC
crates/session/     glave-session: the session API (specs/session.md) in-process, over stdio, and over HTTP with OIDC or local sign-in (glave-identity), and its JSON Schema (specs/session.schema.json, generated); every surface, the CLI first, runs threads through it
crates/eval/        glave-eval: the eval factory, the harness run in-process on evals/tasks against real models
crates/cli/         glave: the command-line binary
web/client/         the session API's TypeScript client: types generated from the schema (`cargo xtask schema`), a small typed client, no framework
evals/tasks/        eval tasks in Harbor's layout: instruction, starting files, hidden grader, reference solution
evals/judge/        judge cases: a request, an action, and whether it should run in auto mode (README.md)
specs/              the contracts: events, storage, config and policy, computers, the gateway, the sandbox, approvals, the session API; tool calls, verbs and scopes planned
.github/workflows/  CI: `cargo xtask ci` on Linux (every push and PR) and macOS (main); workers on a `kind` cluster (Linux)
```

**Dependency rule.** Shared types depend on nothing in Glave. Ports depend only on types. Domain crates (log, harness, gateway) depend on types and ports, never on each other's internals or on adapters. Adapters depend on types, ports and their one external library. Only the binary depends on everything.

## Commands

```sh
cargo xtask ci          # the full gate; must pass before anything is "done"
cargo xtask file-sizes  # line limits only
cargo xtask schema      # regenerate specs/session.schema.json and the web client's types after changing the session API
cargo xtask bench       # benchmarks, alone on the machine (see Performance)
cargo xtask eval        # the harness on evals/tasks against three real models, graded (costs cents)
cargo xtask eval judge  # the action judge scored on evals/judge (the deterministic checks today)
cargo fmt --all         # format
cargo build --profile debugging   # full debug info, for a debugger
```

## Definition of done

`cargo xtask ci` passes: format, clippy with `-D warnings`, all tests, `cargo deny`, file sizes. New behavior has tests. User-visible numbers stay within budget. The change is small, focused and reviewed through a pull request.

## Rust

**Safety and errors**
- `unsafe` is forbidden. Use existing safe crates for anything that needs it (e.g. llama.cpp bindings).
- No `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `dbg!`, `process::exit` or unchecked indexing. These lints are **forbidden**: no attribute can switch them off. Tests get their exemptions only through `clippy.toml`. Use `?`, `.get()`, `match`, `ok_or_else`.
- `#[allow]` is banned. If a style lint is genuinely wrong in one spot, use `#[expect(lint, reason = "...")]`, which fails the build if the lint ever stops firing. Safety lints can't be suppressed at all.
- Don't use `#[tokio::main]` or `#[tokio::test]`: they emit `#[allow(clippy::expect_used)]`, which the forbidden lint rejects. Build the runtime explicitly in `main`; tests use a small `block_on` helper.
- Libraries define typed errors with `thiserror`; binaries use `anyhow`. Errors say what failed and with what input.

**Structure**
- `lib.rs` and `main.rs` are tables of contents: module declarations, re-exports, crate docs. No logic.
- One concept per module. If a file can't be described in one sentence, split it.
- Module layout is `foo.rs` + `foo/`. Never `mod.rs` (clippy enforces this).
- Files: target ≤ 400 lines, hard limit 800 (CI fails). Functions ≤ 80 lines (clippy).
- Crate-private by default (`pub(crate)`); the public API is small and documented.
- No global state or singletons. Pass dependencies in explicitly.

**Types**
- Make illegal states unrepresentable: newtypes for IDs (`SpaceId`, `EventId`), enums over flags, validated constructors.
- IDs are UUID v7 (time-ordered).
- Everything persisted or sent over the wire carries a schema version. Migrations can be replayed.

**Async and concurrency**
- tokio. Never block inside async code; use `spawn_blocking` for blocking work.
- Channels and queues are bounded. Every cache has a size cap. Memory is bounded, not unbounded.
- Every spawned task has an owner that awaits or cancels it. No fire-and-forget.
- Never hold a lock across `.await`. Code must be cancellation-safe.

**Dependencies**
- Every new dependency needs a reason in its pull request. Prefer `std`.
- Declare versions once in `[workspace.dependencies]`.
- `cargo deny` must pass: allowed licenses, no known vulnerabilities, crates.io only.

**Ports and adapters**
- Ports are traits in the ports crate. Every port ships an in-memory adapter and a conformance suite every adapter must pass.
- Design every port as if it were across a network: coarse-grained calls, batched writes, streamed reads, serializable types.
- A new port needs two real implementations. Until then, keep the code concrete.
- Extensions are crates (compile time) or protocols (network). Never runtime-loaded shared libraries.

## Testing

- Unit tests sit beside the code in `#[cfg(test)] mod tests`; when they grow large, move them to a sibling `tests.rs`.
- Integration tests live in each crate's `tests/`.
- Property tests (`proptest`) for parsers and state machines. Snapshot tests (`insta`) for stable formats such as receipts and events.
- Tests are deterministic: no sleeps, no real clocks (inject a clock), no network in unit tests.
- Run tests with `cargo nextest`. Measure coverage; don't chase a percentage.

## Performance

**Budgets** (provisional: set from the first real measurements, then only tightened):

| Budget | Target |
| --- | --- |
| `glave` cold start to prompt | < 50 ms |
| Release CLI binary (stripped) | < 30 MB |
| Gateway overhead per request, in-process | < 1 ms p99 |
| Session resume, 10k events | < 100 ms |
| Knowledge search, 16 queries batched | < 100 ms |

- Hot paths get microbenchmarks (event append and replay, stream parsing, gateway overhead, cache lookups).
- Benchmarks run only through `cargo xtask bench`. It takes a machine-wide exclusive lock that test runs share, so benchmarks never run beside tests or each other.
- Wall-clock numbers come from an idle machine. CI regression gates use instruction counts, which don't vary with load.
- Optimize only what a measurement shows is slow.

## Security and data

- No secrets in clients, logs, responses or the repository. Dev secrets come from the environment and go only to server logs, behind a dev flag.
- No prompt or message content in logs by default. Usage events are content-free.
- Data at rest is encrypted: the whole local database (key in the OS keychain); per-space keys via a KMS plug for organizations. Deleting a space's key erases it.
- Scope is enforced by the data store on every query, never by the model.
- Content from the web is labeled untrusted. Risky actions after reading it need approval.

## Models and training data

- Hope is trained only on data we have the right to use. Never on outputs of models whose terms forbid it.
- Sealed eval sets never enter training data. Training data is deduplicated against every eval set.

## Web client (TypeScript)

- Strict TypeScript. No `any`; use `unknown`. `const` and immutability everywhere; `readonly` arrays, maps and fields.
- Angular (current choice): standalone components, signals as `readonly` fields, native control flow, `inject()`, `input()`/`output()`.
- Angular Material, styled only through design tokens. Form fields use `appearance="outline"`.
- Accessibility: WCAG AA, passes AXE.

## Git

- One branch per change, from `main`; merge through pull requests with CI green. Nobody pushes to `main` directly. Stack pull requests only when a change truly depends on unmerged work.
- Pull requests are squash-merged: one commit on `main`, titled with the pull request's conventional title (`feat(gateway): ...`, `fix(log): ...`, `chore: ...`).
- Every pull request links its issue: `Closes #N` when it finishes one, `Part of #N` when it doesn't. Work with no issue gets one first.
- Branch names start with the change's type (`feat/`, `fix/`, `test/`, `docs/`, `chore/`); labels follow from them and from the files changed.
- Never force-push a shared branch.

## Working here as an agent

- Read the code you are changing and this file first.
- Make the smallest change that does the job. Don't add crates, ports, dependencies or files the task doesn't need.
- No TODO comments; note follow-ups in the pull request.
- Run `cargo xtask ci` before saying something is done, and report failures as they are.
- After changes the model will feel (prompt, tools, approvals, the loop), run `cargo xtask eval --repeat 3` (add `--tier all` for the hard tasks) and report pass rates per model. Load the endpoint from the environment first (`set -a; source .env; set +a`).
- Ask before anything destructive or hard to undo.
