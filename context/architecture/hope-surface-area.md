# Hope Surface Area

## Core Product Thesis

Hope is a cloud agent runtime with a local actuator.

The local client gives Hope hands on the user's machine: files, shell, git, tests, editor integration, and local MCP/tool execution. The cloud runtime contains the agent brain: model calls, prompt assembly, skill selection, memory, orchestration, subtask scheduling, trace interpretation, and internal improvement machinery.

The moat lives in the cloud runtime, not in a shipped local harness.

## Hard Decisions

- Product runtime is cloud-first.
- No local model support in the product runtime.
- No direct completions API as the public platform surface.
- The local client is a remote tool runtime, not the agent brain.
- `.hope/` is the project-facing configuration surface.
- Customer runtime and internal improvement use the same core harness, but with different policies and purposes.
- Champion/challenger promotion is release management, not an automatic agent whim.
- Default trust posture should be no global training on customer content.
- Enterprise/private deployments are about data isolation, auditability, retention, and operational control.

## Main Actors

### Local Client

Runs on the customer's machine.

Responsibilities:

- authenticate user/session
- read project files when requested
- run shell/git/test commands when allowed
- apply patches
- bridge local MCP servers/tools
- enforce local approval and safety policy
- stream tool observations back to the cloud
- maintain short-lived local execution state

The local client should not own:

- proprietary prompt assembly
- planning strategy
- skill-selection policy
- model routing
- long-term memory
- eval/promotion logic
- training-data transformation

### Cloud Harness

Runs in Glave's cloud.

Responsibilities:

- create and manage runs
- assemble model context
- select relevant skills/tools/memory
- call model backends
- spawn workers/subtasks
- stream events
- persist run traces
- manage server-side memory
- request local tool execution
- route approvals
- summarize and resume runs
- power internal eval and improvement loops

### Internal Lab

Uses the same cloud harness to improve the product offline.

Responsibilities:

- propose candidate changes to prompts, skills, policies, routing, tools, or adapters
- run controlled eval suites
- compare champion and challenger versions
- generate scorecards
- curate traces
- train or tune model adapters later
- recommend or execute approved promotions

## Two Modes Of The Same Harness

### Customer Runtime Mode

Goal: solve a user's task.

Flow:

1. User starts a run through CLI, app, editor, or SDK.
2. Local client syncs project metadata, `.hope/` config, and available tools.
3. Cloud harness creates the run and assembles context.
4. Model chooses actions.
5. Cloud sends tool-call requests to the local client.
6. Local client executes allowed actions and returns observations.
7. Cloud continues until completion, pause, failure, or cancellation.
8. Run trace and artifacts are retained according to policy.

### Internal Improvement Mode

Goal: determine whether a new version of Hope is better.

Flow:

1. A proposer creates a candidate change.
2. Candidate passes static validation and policy checks.
3. Candidate runs on search and validation tasks.
4. Scorecard compares candidate to the champion.
5. Hidden promotion evals may run in a sealed control zone.
6. Human approval is required for meaningful promotion at first.
7. Promoted candidate becomes the new champion manifest.

The customer runtime asks: can Hope solve this task?

The internal lab asks: is this Hope version better than the previous version?

## Project Configuration: `.hope/`

`.hope/` is the user/project-facing harness interface. It is not the proprietary harness itself.

Initial shape:

```txt
.hope/
  hope.md
  settings.json
  agents/
    coder.yaml
    researcher.yaml
    reviewer.yaml
  skills/
    fix-failing-test/
      SKILL.md
    research-brief/
      SKILL.md
  commands/
    implement.md
    research.md
    review.md
    overnight.md
  prompts/
    core.md
    voice.md
    modes/
      coding.md
      research.md
```

Public terms:

- agent: user-facing configured worker concept
- skill: reusable workflow loaded when relevant
- command: user-invoked workflow
- prompt: reusable project prompt fragment
- settings: project defaults and policy

Internal terms:

- profile/spec: static recipe for an agent
- task: unit of work
- run: execution of a task
- worker: live execution thread
- tool: callable local or remote capability

`AGENTS.md` may exist as a compatibility bootloader, but `.hope/hope.md` is the Hope-native project guide.

## Skills And Context Assembly

Hope should not dump all project guidance into every model call.

Context assembly should be selective:

```txt
core policy
+ active mode policy
+ selected agent spec
+ selected skill(s)
+ task context
+ retrieved memory
+ selected tool manifests
+ output contract
```

Skills are source material for the harness. They are not automatically trainable examples. When traces are transformed into training data later, skill text and meta-instructions should usually be stripped so the model learns behavior, not scaffolding.

## Runs API

The public SDK should expose runs, not completions.

Do not expose:

```txt
POST /chat/completions
```

Expose agentic operations:

```txt
POST /runs
GET /runs/:id
GET /runs/:id/events
POST /runs/:id/events
POST /runs/:id/approve
POST /runs/:id/cancel
POST /workspaces/:id/sync
POST /tool-calls/:id/result
```

Developers build on Hope by starting, streaming, approving, cancelling, and integrating runs. They do not bypass the harness to call tuned models directly.

## Event Streaming

Runs are long-lived and should be stream-first.

Use WebSockets for bidirectional runtime communication. Server-sent events may be useful for read-only dashboards, but the core client needs two-way streaming for tool calls, approvals, user interruptions, and resume.

Representative server-to-client events:

```txt
run.started
run.status
agent.message.delta
worker.spawned
tool.call.requested
approval.requested
artifact.created
run.paused
run.completed
run.failed
```

Representative client-to-server events:

```txt
tool.call.completed
tool.call.failed
approval.granted
approval.denied
user.message
run.pause.requested
run.cancel.requested
```

The event stream is transport. The durable append-only event log is the trace.

Reconnect should use `last_event_id` so clients can attach, detach, and resume without losing run state.

## Tool And MCP Model

Tool calling is cloud-decided and client-executed.

Flow:

1. Local client discovers available tools and MCP servers.
2. Client syncs tool manifests to the cloud.
3. Server caches manifests by hash.
4. Server selects a small relevant toolset for each run/step.
5. Model chooses from selected tools.
6. Server sends tool-call request to client.
7. Client validates local policy and executes.
8. Client returns observation.

The client can advertise capabilities. The server chooses capabilities. The client enforces local permission before execution.

Avoid exposing every MCP tool to every model call. Cache and index all tools; inject only the few that matter.

## Cache, Sync, And Project Identity

Cache exists to reduce retransfers, enable indexing, and make runs reproducible.

Use content addressing:

- file content hash
- `.hope` config hash
- skill hash
- tool manifest hash
- prompt fragment hash
- artifact hash

Client sends manifests first. Server requests missing content only when needed.

Project display name may be folder name, but project identity should use stronger fingerprints:

- org id
- repo remote URL hash when available
- workspace root hash for local-only folders
- root marker files
- `.hope` config hash

Same repo cloned twice should generally dedupe by remote URL and relevant workspace fingerprint.

## Memory

Memory is server-authoritative.

Memory types:

- user memory: preferences, working style, voice
- org memory: engineering norms, policy, conventions
- project memory: repo architecture, recurring lessons, useful paths
- run memory: current task state
- skill/tool index memory: searchable capability metadata

The local client may keep short-lived cache, but the server is the source of truth so Hope can work across machines.

Memory is not the same as cache.

Cache says: blob `abc123` is available.

Memory says: this project uses pnpm and parser tests live under `test/parser`.

## Storage And Retention

Do not store full project mirrors forever by default.

Store:

- run events
- tool calls and observations
- files/snippets actually read
- diffs generated
- command outputs needed for trace/debug
- final answers
- approvals
- errors
- `.hope` config snapshots
- project summaries
- file/content hashes

Default retention should be purpose-bound:

- active run data: while active
- hosted workspace snapshots: short-lived, often 1-7 days
- raw run traces/snippets/logs: shorter retention, configurable
- summaries, scorecards, diffs, audit records: longer retention
- project memory: expire after inactivity unless pinned/configured

A six-month inactivity window is a reasonable starting point for project-level memory cleanup. Raw payloads should usually expire earlier unless pinned or required by policy.

## Customer Data And Training

Recommended default:

> Glave does not train global models on customer content.

Customer content includes code, prompts, files, tool outputs, terminal logs, diffs, traces, and private project context.

Permitted by default:

- use data to serve the requested run
- retain operational traces according to policy
- provide audit/history/debugging
- build project memory for that customer/org if enabled
- use aggregate non-content telemetry for reliability and product improvement

Optional modes:

- private project memory: improves only the customer's project/org
- tenant-local learning: improves only that tenant
- learning partner: explicit opt-in contribution to global improvement after scrubbing/curation

Raw traces are never training data. Raw traces are source material.

Training data must pass through a transformation layer:

- classify provenance
- redact secrets/private content
- strip hidden eval material
- remove prompt scaffolding/meta-talk
- preserve good observable behavior
- create SFT examples, preference pairs, or action-policy examples
- keep train/validation/hidden splits clean

## Enterprise

Enterprise default should be trust-first.

Mandatory enterprise surfaces:

- org isolation
- admin controls
- audit logs
- retention controls
- permissions and approvals
- trace visibility
- data deletion/export
- SSO/SAML/SCIM later

Enterprise runtime evals are not mandatory. If present, they should be framed as customer-owned policy checks or verification checks, not as Glave's internal improvement evals.

Single-tenant managed cloud is the preferred enterprise isolation model:

- separate database
- separate object storage
- separate KMS keys
- separate queue
- separate trace/memory/index storage
- managed autoupdates

Customer VPC/on-prem is possible later, but it leaks more operational surface and should be premium.

## Hosted Workspace Future

The harness should support swappable execution substrates.

Today:

```txt
cloud harness -> local client -> user machine
```

Future:

```txt
cloud harness -> hosted sandbox client -> containerized repo
```

Define a runtime interface:

- list files
- read file
- write file
- apply patch
- search
- run command
- git status/diff
- call tool

Implementations:

- local-attached runtime
- hosted-ephemeral runtime
- hosted-persistent runtime later

The cloud planner should talk to a workspace runtime with capabilities, not hard-code "local machine."

## Champion / Challenger

Champion/challenger is release management.

States:

```txt
candidate -> validated challenger -> promoted champion
```

Candidate requirements:

- diff
- reason
- owner
- scope
- rollback path

Validated challenger requirements:

- schema valid
- forbidden files untouched
- no permission expansion without approval
- no hidden eval changes
- no scoring changes bundled with behavior changes
- smoke tests pass
- scorecard exists

Promotion requirements:

- beats champion on target metrics
- no critical regressions
- hidden promotion evals pass when applicable
- cost/latency within bounds
- security checks pass
- review complete
- human approval at first
- rollback plan recorded

Never bundle evaluator changes with behavior changes. Change the ruler and the runner separately.

Champion should be a manifest:

```yaml
champion_id: coding-agent-2026-05-16.3
components:
  core_prompt: core@sha256:abc
  coding_skill: fix-failing-test@sha256:def
  tool_policy: coding-tools@sha256:ghi
  model_route: prod-route@sha256:jkl
  final_answer_policy: concise-closeout@sha256:mno
promoted_by: chris
promoted_at: 2026-05-16T22:14:00Z
rollback_to: coding-agent-2026-05-16.2
```

## Hidden Evals

Hidden evals are for promotion, not teaching.

They must live in a sealed control zone:

- candidate cannot inspect them
- candidate cannot edit them
- candidate cannot see answer keys
- detailed hidden failure traces do not enter training data
- hidden eval output should be aggregate receipts only

Teaching happens on public/search evals, curated validation examples, internal dogfooding, public repos, synthetic tasks, and opted-in traces.

## Model Adapters

Adapter tuning is downstream of the harness.

The harness discovers reliable behavior. Curated traces teach stable habits to the model:

- house voice
- dynamic verbosity
- engineering discipline
- tool-use etiquette
- concise final answers
- uncertainty calibration
- good recovery behavior

Do not bake authority into weights:

- permissions stay in the harness
- hidden evals stay sealed
- current tool policy stays external
- security boundaries stay enforced by code

The adapter should learn the dance. The control plane keeps the doors locked.

## Out Of Scope For The First Slice

- local model runtime
- direct completions API
- automatic production promotion
- broad hosted container execution
- enterprise on-prem appliance
- global training on customer content
- full dashboard
- complex tenant-local fine-tuning

## First Implementation Slice

Minimum credible foundation:

1. Cloud run object and durable event log.
2. WebSocket event stream.
3. Local client as tool executor.
4. Workspace manifest sync with hashes.
5. `.hope/` loader for `hope.md`, `settings.json`, `agents`, `skills`, and `commands`.
6. Tool manifest sync and selected tool exposure.
7. One useful command, likely `research` or `implement`.
8. Trace persistence with provenance and retention metadata.
9. Manual pause/resume/cancel/attach.
10. Basic internal eval runner separated from customer runtime.

## Open Questions

- Exact `.hope/settings.json` schema.
- Whether public folder should be `.hope/agents` or `.hope/profiles`; current product instinct favors `agents`.
- Which data fields are retained by default and for how long.
- How much `.hope` content is uploaded eagerly versus lazily by hash.
- Whether standard cloud should have any explicit learning-partner tier at launch.
- What model routes power production at launch.
- What approval policies are default for shell, patch, git, and MCP actions.
- What is the first benchmark suite for internal improvement.
