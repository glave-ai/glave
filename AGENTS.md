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
7. **Standards first.** OpenAI-compatible API (models), MCP (tools), A2A (agents), OIDC/SAML/SCIM (identity), S3/Postgres (storage), AGENTS.md (instructions). Define our own only where none exists and the protocol is part of the product: the event log format, receipts, the session API, computer lifecycle, eval reports. Storage has none of its own: SQLite and Postgres (S3 and a KMS later), and clients reach data through the server (`specs/storage.md`, "Which stores").
8. **Subtract before adding.** Nothing optional is on by default unless it is invisible and harmless.
9. **Measure, don't guess.** Design and performance decisions are settled by evals and benchmarks.

## Concepts

| Term | Meaning |
| --- | --- |
| Organization | The tenant: one on a laptop (never shown), one in a company, many on Glave Cloud. Holds providers, catalogs, policy, the theme, the budget (`specs/model.md`) |
| Team | A flat group of people, granted models, connections and budgets |
| Person | Someone who signs in: their Chats, their computer, their own sign-ins and choices |
| Project | The scope boundary: a project (folders, instructions, members), or a person's Chats, their project of kind `personal`. Holds memory, history, files, members, policy, budget, default environment |
| Thread | A conversation or task in a project. Workers fork from a parent with permissions and budget ⊆ the parent's |
| Environment | A reusable template for where work runs: computer type, image, setup, network, credentials, limits |
| Computer | A person's one machine (laptop, server or cloud machine) where the agent works, its reach drawn by the operating system's sandbox: secrets hidden, the machine readable, the project writable. Fresh per-task workers are an upgrade |
| Harness | The agent loop: context, tool calls, approvals, receipts, threads, scheduling |
| Gateway | The single door out for models and remote tools: routing, keys, policy, budgets, metering |
| Data store | Everything durable: the event log (history), memory, files, config. Enforces scope on every query |
| Hope | Glave's open model. Great in any harness, best in Glave |

## Repository

```
Cargo.toml          workspace: lints, profiles, shared dependencies
xtask/              developer automation (`cargo xtask`)
crates/event/       glave-event: the versioned event format (envelope, payloads, upgrade-on-read)
crates/ports/       glave-ports: event log, blob store and the objects that hold blobs' bytes, key erasure, model provider (and how one is reached: a key, or credentials that change), tool source, computer and memory ports, the settings config fills and the domain crates read (hooks, the judge, a project's protections, worker limits, what the web tools reach, `.mcp.json`), glave's own streams beside threads (the ledgers' reserved ids, reads caught up, appends retried), the caller, the clock and entropy, in-memory versions, conformance suites and the test doubles every crate shares
crates/identity/    glave-identity: who is calling: OIDC and local sign-in that verify the caller, browsers signed in by one-time links to sessions kept as hashes, the person an issuer's subject is
crates/config/      glave-config: config and policy (specs/policy.md): choices nearest-wins, constraints that only narrow, project trust, `policy explain`; shared by the CLI and the servers
crates/provider-openai/  glave-provider-openai: OpenAI-compatible streaming provider
crates/provider-anthropic/  glave-provider-anthropic: Anthropic's native Messages API provider (thinking sent back signed), also as Vertex AI's rawPredict
crates/azure/       glave-azure: Azure OpenAI's v1 endpoint, and Microsoft Entra tokens from AKS workload identity (a hand-rolled exchange)
crates/aws/         glave-aws: Amazon Bedrock's OpenAI-compatible endpoints and Anthropic Messages route, and SigV4 (aws-sigv4) with keys from the environment, IRSA or EKS Pod Identity
crates/google/      glave-google: Google Vertex AI's endpoints, and Application Default Credentials as a refreshing bearer token (the GKE metadata server, service account keys, gcloud's user credentials, workload identity federation), hand-rolled
crates/gateway/     glave-gateway: the in-process gateway (specs/gateway.md): routes by purpose, safe fallback, budgets, metering
crates/gateway-service/  glave-gateway-service: the gateway as a service (`glave gateway serve`): OpenAI and Anthropic surfaces, OIDC sign-in (glave-identity), per-caller policy, budgets and metering, the MCP tool catalog at /mcp
crates/keys/        glave-keys: the key hierarchy (root, organization, project, data keys), sealing at rest, erasure; roots in the OS keychain (`keychain`) or a key file
crates/bucket/      glave-bucket: blobs' sealed bytes, apart from their records, in a folder in glave's folder, through object_store
crates/store-sqlite/  glave-store-sqlite: the event log, blobs' records and wrapped keys on one local SQLite file, content sealed
crates/store-postgres/  glave-store-postgres: the event log, blobs' records and wrapped keys on Postgres, an organization's one data store (`[store] url`), TLS off this computer, content sealed
crates/process/     glave-process: every operating-system process glave starts, through one place (`cargo xtask spawns` checks): its own process group, a ledger under GLAVE_HOME/run while it runs, and what a glave that ended left stopped at the next start
crates/shell/       glave-shell: shell commands read before they run (tree-sitter-bash): read-only, risky with a reason, or neither
crates/mcp/         glave-mcp: MCP servers as tool sources, through the official rmcp SDK (a test server behind `test-server`); open-world tools and servers marked untrusted return untrusted output
crates/memory/      glave-memory: memory (specs/memory.md): an in-process MCP server over the data store, each memory sealed under its own key, scoped by the session
crates/web/         glave-web: web_fetch as a tool source: private addresses refused at every hop, limits, a host policy, HTML as Markdown, output untrusted (a local page server behind `pages`)
crates/harness/     glave-harness: the agent loop, its seven tools (search, glob, read, write, edit, bash, command_output for commands kept running in the background) run on a computer through the port, approvals, resume, AGENTS.md, skills (agentskills.io), custom commands, hooks
crates/computer/    glave-computer: computers (specs/computers.md): this machine as one (in the sandbox by default), environments from devcontainer.json or environment.toml, workers on Docker or Kubernetes (the `kubernetes` feature), served (`glave computer serve`, TLS across machines) and reached over JSON-RPC
crates/session/     glave-session: the session API (specs/session.md) in-process, over stdio, and over HTTP with OIDC or local sign-in (glave-identity), and its JSON Schema (specs/session.schema.json, generated), and the accounts ledger a host keeps a person's model accounts and git hosts in; every surface, the CLI first, runs threads through it
crates/eval/        glave-eval: the eval factory, the harness run in-process on evals/tasks against real models
crates/cli/         glave: the command-line binary; with the `web` feature, as releases build it, the web app inside (build.rs)
web/client/         the session API's TypeScript client: types generated from the schema (`cargo xtask schema`), a small typed client, no framework
web/glaze/          Glaze (`@glave/glaze`): glave's Angular component library, web platform first, then the Angular CDK, themed only through CSS custom properties (its AGENTS.md says how to use it); its showcase, a dev-only app (`glaze/showcase`)
web/app/            the web app `glave serve --http` (and `glave` alone, at the folder's project) serves at `/`: sign-in; adding a model on a first run; three panes (chats and projects with what needs you, a thread streaming live with approvals, questions and a model picker, its tools: tasks, changed files and commands running in the background); tasks, settings and memory
web/e2e/            end-to-end smoke tests (Playwright, axe) of the built app and Glaze's showcase, served with glave's Content Security Policy
evals/tasks/        eval tasks in Harbor's layout: instruction, starting files, hidden grader, reference solution
evals/judge/        judge cases: a request, an action, and whether it should run in auto mode (README.md)
specs/              the model (`model.md`: organization to thread, server and computer), the deployments it serves (`deployments.md`), and the contracts: events, storage, config and policy, computers, the gateway, the sandbox, approvals, the session API; the invariants (`invariants.md`), each with its test; tool calls, verbs and scopes planned
.github/workflows/  CI: `cargo xtask ci` on Linux (every push and PR) and macOS (main); workers on a `kind` cluster (Linux); releases on version tags (`v0.1.0`): release.yml, made by `cargo xtask release-workflow` (`dist generate` from dist-workspace.toml and .github/build-setup.yml, then only the job that publishes can write), never edited by hand
```

**Dependency rule.** Shared types depend on nothing in Glave. Ports depend only on types. Domain crates (harness, gateway, keys, config) depend on types and ports, never on each other or on adapters. Adapters (providers, stores, computers, MCP, web, memory, identity) depend on types, ports and their one external library. Only the binary, and the crates it is wired from (the session API, the gateway service), depend on everything; tools (xtask, eval) may too, and nothing depends on them. Only normal dependencies count: tests may use adapters. CI checks it (`cargo xtask deps`) against one table in `xtask/src/deps.rs`: each crate's layer and why, and the few edges that cross the rule on purpose, each with its reason.

## Commands

```sh
cargo xtask ci          # the full gate; must pass before anything is "done"
cargo xtask deps        # the dependency rule: each crate depends only on the layers its own may use
cargo xtask file-sizes  # line limits only
cargo xtask secrets     # no secret-shaped literal in a tracked file
cargo xtask spawns      # every process glave starts goes through glave-process
cargo xtask schema      # regenerate specs/session.schema.json and the web client's types after changing the session API
cargo xtask release-workflow   # regenerate .github/workflows/release.yml after changing dist-workspace.toml or .github/build-setup.yml (needs dist, the version dist-workspace.toml names)
cargo xtask bench       # benchmarks, alone on the machine (see Performance)
cargo xtask eval        # the harness on evals/tasks against three real models, graded (costs cents); the endpoint from .env (or `--env-file <path>`)
cargo xtask eval judge  # the action judge scored on evals/judge (the deterministic checks today)
cargo fmt --all         # format
cargo build --profile debugging   # full debug info, for a debugger
cargo build --release -p glave --features web   # the binary as it ships, the web app inside (after npm run build in web/)
(cd web && npm run check)         # the web checks alone; cargo xtask ci runs them (Node 22.22.3+, or GLAVE_SKIP_WEB=1)
```

## Definition of done

`cargo xtask ci` passes: format, no secret-shaped literals (`cargo xtask secrets`), the dependency rule (`cargo xtask deps`), clippy with `-D warnings`, docs with `-D warnings`, all tests, with nothing left behind (no process they started still running, no `glave-*` entry in their temp folder but what glave keeps there), `cargo deny`, no unused dependencies (`cargo machete`), file sizes. New behavior has tests. User-visible numbers stay within budget. The change is small, focused and reviewed through a pull request.

## Quality

The bar is code a careful reader admires: every line earns its place, and each idea lives in one spot. Lints catch unsafe code. These rules are about what lints can't catch:

- **One PR, one change.** A second feature, or a new workspace dependency, goes in its own PR. A PR over 40 files or 1,500 lines is split, or says in its description why it can't be.
- **One concept, one definition.** A type that crosses crates is defined once, in the lowest crate that needs it (`glave-event` or `glave-ports`). Two enums with the same variants and a `From` between them is a defect.
- **Search before writing.** Before adding a helper, parser, URL check, error mapper or storage wrapper, `git grep` for one. Where a second copy would go, share the first instead.
- **Nothing before its second user.** A trait needs two real implementations, and a config key two values that both work today. Anything `pub` needs a caller outside its crate. Code whose last caller goes is deleted with it.
- **Specs hold decisions and invariants,** never method tables, field lists or crate descriptions; the generated schema is the reference. A PR that changes a type rereads the spec section that describes it. A spec's Status is a list, one line per piece built or pull request, so a change edits only the line it owns, never a paragraph another pull request is also rewriting.
- **Invariants keep their tests.** A change that touches one of `specs/invariants.md` says so in its pull request and keeps the test that proves it; one that closes a gap updates the entry.
- **Reviewed before merge.** Every feature PR gets an independent review (not its author) before it merges, beyond CI.

**Keeping principle 4.** A person on their own computer runs exactly what an organization runs. What differs is what sits behind the seams, never what the person sees:
- **Five deployments, one program.** `specs/deployments.md` is the matrix of who glave serves: a laptop, a person's own server, Glave Cloud's personal and enterprise plans, and an enterprise's own servers. Every design is checked against all five columns in its spec or pull request description, and a gap is named, not skipped. A capability exists because the server's configuration provides it, never because a client or a deployment-type setting says so.
- **The server says what exists.** `server/discover` tells a client whether there is an organization, groups, sign-in or workers, and admin UI renders only when it says so. A local server says none. No client decides which admin features exist from its own config.
- **The server says what happened; each client draws it.** Everything about a person's work comes from the server as typed facts, each with one plain sentence: a thread's items and their status; an approval or question with its sentence, detail and choices as fields; what needs the person and why; receipts; the names of models and services; what the caller may do; why a request was refused. A client never writes its own sentence about the work from events, counts or a parsed string: it shows the server's and draws the facts around it. A client owns its chrome:
  - layout, order, folding, icons and motion;
  - labels, empty states, and the words for when the server can't be reached;
  - markdown and diffs drawn from text, never HTML;
  - numbers, money and times formatted for the reader, and words relative to the reader ("this computer");
  - what this tab remembers: drafts, panes, scroll.

  A click may show its result at once; the server's answer is what stays. No item names a pane, a fold or a place on screen: every item prints as one line of plain text, which the CLI proves.
- **One project shows no projects.** With only the person's Chats, the UI shows no project grouping, members or moving between projects.
- **Plain words.** No user-visible text names a file, flag, command or error code unless the person typed it. Every error a client can show has a message a person can act on.
- **The consumer journey test** (`web/e2e/consumer.e2e.ts`, planned in #251) runs against a real server with a fresh home: add a model, chat, approve a command, connect GitHub. It fails if an admin word appears (policy, organization, member, group, admin, SSO, `config.toml`).
- **Startup and size are measured** on every pull request, in the merge queue and after every merge to `main` (`budgets.yml`). A cloud or server dependency moves behind a Cargo feature only for a real reason (a security or licence problem, or a size that hurts people), not to meet a number: one build serves every deployment.

## Rust

**Safety and errors**
- `unsafe` is forbidden. Use existing safe crates for anything that needs it (e.g. llama.cpp bindings).
- No `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `dbg!`, `process::exit` or unchecked indexing. These lints are **forbidden**: no attribute can switch them off. Tests get their exemptions only through `clippy.toml`. Use `?`, `.get()`, `match`, `ok_or_else`.
- `#[allow]` is banned. If a style lint is genuinely wrong in one spot, use `#[expect(lint, reason = "...")]`, which fails the build if the lint ever stops firing. Safety lints can't be suppressed at all.
- Don't use `#[tokio::main]` or `#[tokio::test]`: they emit `#[allow(clippy::expect_used)]`, which the forbidden lint rejects. Build the runtime explicitly in `main`; tests use `glave_ports::block_on!` (the `conformance` feature).
- Libraries define typed errors with `thiserror`; binaries use `anyhow`. Errors say what failed and with what input.

**Written to pass the lints** (`[workspace.lints]`, `clippy.toml`), the first time:
- Write `pub(crate)` unless another crate calls it; a binary's and a test's own modules use `pub(crate)` too.
- Name an item by what `use` already brought in; no full path for something imported.
- Clone an `Arc` or `Rc` as `Arc::clone(&x)`, never `x.clone()`, so cheap clones look cheap.
- Show a type's lifetime in paths: `&[Crate<'_>]`, `Formatter<'_>`.
- Libraries never print: return the value or emit `tracing`. Only glave's binary, xtask, eval's command line, benches and examples print, each saying so once with `#![expect(clippy::print_stdout, …)]` at its root; tests may print.
- Iterate a `BTreeMap`, `BTreeSet` or `Vec` wherever order can be seen (output, ids, eviction, tests); a `HashMap` is for lookups. Where order truly can't matter, `#[expect(clippy::iter_over_hash_type, reason = "…")]` with that reason.
- A function that runs until it is dropped or cancelled returns `!`; an `async fn`, which can't, says so with `#[expect(clippy::infinite_loop, reason = "…")]`.
- An impl that forwards a port to another (the CLI's `DataLog` and `DataBlobs`, the session's `Observed`, the gateway's `Model` for every provider) carries `#[deny(clippy::missing_trait_methods)]`, so a new port method can't fall back to a default unseen.
- Read the time through a `glave_ports::Clock`: the one handed in where a test sets the time, else `SystemClock`. `Timestamp::now` and `SystemTime::now` belong to `SystemClock` alone (`disallowed-methods`, tests and the binary included).
- Doc links are `` [`Item`] `` paths that resolve from where they're written, to items as public as the doc; anything else is plain `` `code` ``. `cargo xtask ci` builds the docs with warnings denied.

**Structure**
- `lib.rs` and `main.rs` are tables of contents: module declarations, re-exports, crate docs. No logic.
- One concept per module. If a file can't be described in one sentence, split it.
- Module layout is `foo.rs` + `foo/`. Never `mod.rs` (clippy enforces this).
- Files: target ≤ 400 lines, hard limit 800 (CI fails). Functions ≤ 80 lines (clippy).
- Crate-private by default (`pub(crate)`); the public API is small and documented.
- No global state or singletons. Pass dependencies in explicitly.

**Types**
- Make illegal states unrepresentable: newtypes for IDs (`ProjectId`, `EventId`), enums over flags, validated constructors.
- IDs are UUID v7 (time-ordered).
- Everything persisted or sent over the wire carries a schema version. Migrations can be replayed.
- Until glave 0.1 ships there is no compatibility: formats change in place, each store has one schema, and people remove their data. From 0.1 on, versions, migrations and upgrade-on-read apply.

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
- Integration tests live in each crate's `tests/`, as one program: `tests/main.rs` declares each file as a module (`autotests = false`), so the dependency tree is linked once per crate, not once per file. nextest still runs each test in a process of its own.
- Property tests (`proptest`) for parsers and state machines. Snapshot tests (`insta`) for stable formats such as receipts and events.
- Tests are deterministic: no sleeps, no real clocks (inject a clock), no network in unit tests.
- Run tests with `cargo nextest`. Measure coverage; don't chase a percentage.
- Fakes live in one place: `glave-ports` ships the test doubles for its ports (clock, entropy, cancellation, `block_on`, fake secrets, temp folders) behind the `conformance` feature. Never write a second copy in a crate.
- Every test removes what it makes: its folders are `glave_ports::testing::TempDir`s (`outside_temp` for the sandbox's tests), removed with what they hold when dropped, and a server it starts in the background is stopped by a guard when it drops.
- No secret-shaped string in the source: a test that needs a provider's shape makes one at run time (`fake_secret("ghp_")`), any other uses `FAKE_SECRET`, and no assertion message or log line prints a secret.
- A scenario is tested once: in the conformance suite when there is one, otherwise beside the code it tests.

## Performance

**Budgets** (provisional: set from the first real measurements, then only tightened):

| Budget | Target |
| --- | --- |
| A command answered by a running server | < 50 ms |
| Release CLI binary (stripped) | < 40 MB |
| Gateway overhead per request, in-process | < 1 ms p99 |
| Session resume, 10k events | < 100 ms |
| Knowledge search, 16 queries batched | < 100 ms |

Two owner decisions: the release binary was raised from 30 to 40 MB on 2026-10-19, since splitting builds costs more than a few MB; and "a command answered by a running server" (`glave tasks` against the person's server, about 5 ms on an M3) replaced "cold start to prompt" when `glave` alone came to open the browser and left no prompt to time (`specs/one-server.md`, decision 9). The budgets are set in one place, `crates/cli/benches/budgets.rs`.

- Hot paths get microbenchmarks (event append and replay, stream parsing, gateway overhead, cache lookups).
- Benchmarks run only through `cargo xtask bench`. It takes a machine-wide exclusive lock that test runs share, so benchmarks never run beside tests or each other.
- Wall-clock numbers come from an idle machine. CI regression gates use instruction counts, which don't vary with load.
- Optimize only what a measurement shows is slow.

## Security and data

- No secrets in clients, logs, responses or the repository. Dev secrets come from the environment and go only to server logs, behind a dev flag.
- No prompt or message content in logs by default. Usage events are content-free.
- Data at rest is encrypted: the whole local database (key in the OS keychain); per-space keys via a KMS plug for organizations. Deleting a project's key erases it.
- Scope is enforced by the data store on every query, never by the model.
- Content from the web is labeled untrusted. Risky actions after reading it need approval.

## Models and training data

- Hope is trained only on data we have the right to use. Never on outputs of models whose terms forbid it.
- Sealed eval sets never enter training data. Training data is deduplicated against every eval set.

## Web client (TypeScript)

- Strict TypeScript. No `any`; use `unknown`. `const` and immutability everywhere; `readonly` arrays, maps and fields.
- Angular, current stable, fully modern: standalone, zoneless, OnPush, signals as `readonly` fields, `input()`/`output()`/`model()` (two-way binding with `model()` wherever a component owns a value its parent can change), `computed()`, `inject()`, native control flow. No `@Input`/`@Output` decorators, no NgModules.
- Glaze (`web/glaze`) for every component, not Angular Material. Web platform first (native `<dialog>`, popover, `<details>`, native inputs, CSS `field-sizing`), checked against Baseline; then the Angular CDK where the platform has nothing (keyboard models, LiveAnnouncer, menus, clipboard, layout); our own code last. Styled only through design tokens: CSS custom properties, light and dark from `light-dark()` and `color-scheme`. Glaze keeps no app state and has no services.
- One style system, in Glaze's tokens (`web/glaze/styles/tokens.css`); nothing else sets a colour, size, space, radius or shadow:
  - **Parents own spacing.** No component sets its own outer margin: the space between things is its parent's `gap`. Stylelint fails on any margin but `0` or `auto` in component styles; the rare one the platform needs carries its reason in the disable comment.
  - **Layout primitives**, plain classes: `glz-stack` (one above another), `glz-row` (side by side, wrapping), `glz-grid` (auto columns), each with a `glz-gap-*` from the space scale.
  - **Insets** pad each kind of box the same way everywhere: `--glz-inset-row`, `-control`, `-button`, `-strip`, `-bubble`, `-card`, `-composer`, `-dialog`, `-pane-*`, `-page-*`.
  - **Fluid space and type**: `--glz-space-*` and `--glz-text-*` grow with `clamp()` from a phone to a wide screen, never jumping at a breakpoint. Radii are a fixed scale, one per kind of box.
  - **Prose flows**: running text (`glz-flow`, which markdown uses) spaces its own blocks and sits flush in what holds it.
- Every component is four files, made with `ng generate`: `x.component.ts`, `.html`, `.scss`, `.spec.ts`. Folders by kind: `components/`, `directives/`, `services/` (stateful, signal state), `guards/`, `interceptors/`, `pipes/`, and `lib/` for pure functions.
- No logic in templates beyond native control flow: no computation, no expressions beyond reading a signal or field, no user-visible strings or hard-coded values. Derive in the controller (`computed()`, readonly constants). angular-eslint enforces what it can (`conditional-complexity` 1, `prefer-control-flow`, `no-inline-styles`, `no-call-expression` is not used since signals are calls).
- Test hooks: every element an end-to-end test needs has a `data-testid` (`showcase-open-dialog`); end-to-end tests select only by these. Glaze never names hooks itself: its components take a `testId` input and give their parts `<testId>-<part>` (`testId="thread-composer"` gives `thread-composer-send`).
- Linted and formatted: angular-eslint (recommended, template and accessibility), typescript-eslint strict and stylistic type-checked, stylelint for SCSS and CSS, Prettier across `web/`. CI checks all of it.
- Tested: unit tests (Vitest) for every component, directive, service and `lib/` function, with axe on what they render; end-to-end smoke tests with axe in light and dark, in Chromium and in WebKit (Safari's engine).
- Accessibility: WCAG 2.2 AA, passes axe.
- A Glaze component, directive or input ships in the same PR as the app screen that uses it. The showcase demonstrates Glaze; it never justifies a piece.
- The app is served under a strict Content Security Policy: no inline script; inline styles only with each load's nonce.

## Git

- One branch per change, from `main`; merge through pull requests with CI green, by the merge queue, which tests each change on top of the ones ahead of it. Nobody pushes to `main` directly. Stack pull requests only when a change truly depends on unmerged work.
- Pull requests are squash-merged: one commit on `main`, titled with the pull request's conventional title (`feat(gateway): ...`, `fix(log): ...`, `chore: ...`).
- Every pull request links its issue: `Closes #N` when it finishes one, `Part of #N` when it doesn't. Work with no issue gets one first.
- Branch names start with the change's type (`feat/`, `fix/`, `test/`, `docs/`, `chore/`); labels follow from them and from the files changed.
- Never force-push a shared branch.

## Working here as an agent

- Read the code you are changing and this file first.
- Make the smallest change that does the job. Don't add crates, ports, dependencies or files the task doesn't need.
- No TODO comments; note follow-ups in the pull request.
- Run `cargo xtask ci` before saying something is done, and report failures as they are.
- After changes the model will feel (prompt, tools, approvals, the loop), run `cargo xtask eval --repeat 3` (add `--tier all` for the hard tasks) and report pass rates per model. It reads the endpoint from `.env` itself (the nearest at or above the workspace, else the main checkout's from a worktree), so no shell step comes first.
- Ask before anything destructive or hard to undo.
