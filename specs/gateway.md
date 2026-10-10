# Gateway

The single door out for model calls: which model serves each purpose, with whose keys, within what budget, and what it cost. Runs inside glave for a person and as a service for an organization, from the same crate.

## Status

In-process gateway built (`crates/gateway`): routes by purpose, capability-checked and reasoning-sticky fallback, reserve-then-settle budgets held in memory (organization, person, and each thread within the threads above it), and `model.responded` version 2 recording the route, the target and any reasoning stripped. The harness sends every call with its purpose and the threads it counts in; the judge's calls are recorded in the thread whose action they judged. The CLI builds the gateway from `GLAVE_MODEL` (`main`) and `GLAVE_JUDGE_MODEL` (`judge`, when it names another model), with no budgets, so nothing changes by default.

Not built yet: `glave gateway serve` and its OpenAI and Anthropic inbound surfaces, grouping stateless calls into threads, credential sources beyond the environment key, Bedrock, Azure and Vertex targets, typed TOML configuration of routes (the policy's `[models]` allow-list is kept by the CLI when it builds the targets, `policy.md`), space-level budgets, and instruction-count gates for the overhead benchmark (`cargo xtask bench --bench overhead` measures wall-clock time today).

## Routes

A **route** names a purpose and maps it to an ordered list of **targets** (provider, model, region, credential reference), each with what it can do (tools, reasoning, images, context size).

- Purposes are a stable public vocabulary: `main`, `judge`, `summary` (compaction), `subagent`, and later `voice.listen`, `voice.speak`. A purpose with no route uses `main`.
- Routes are choices (nearest wins) and the models they may name are constraints (`policy.md`).
- `model.responded` records the route and the target that actually served each call, so fallback is auditable.

## Fallback

- **Capability-checked:** a target that cannot honour the request (no tools, a smaller context than the conversation) is skipped, never sent a degraded request.
- **Thread-sticky:** a conversation that holds a provider's opaque reasoning falls back only to targets that accept it. Otherwise the reasoning is stripped explicitly and that is recorded.
- Failures that may pass (rate limits, overload, timeouts) move to the next target after the provider's own retries.

## Keys

- The gateway resolves credentials; nothing that calls it ever holds them.
- **Sources:** the OS keychain for a person; environment, a KMS, a secrets manager or workload identity for an organization; short-lived tokens refreshed outside the request path.
- **Cloud contracts are targets like any other:** Amazon Bedrock (Anthropic's Messages endpoint, AWS signing), Azure OpenAI (Entra tokens), Google Vertex, and the company's own vLLM. That is how "data on our servers, models through our cloud contract" works.

## Budgets

- Stacked: organization ⊇ space ⊇ person ⊇ thread ⊇ worker, matching "a worker's budget is within its parent's".
- **Reserve, then settle.** Before sending, reserve the worst case (input tokens plus the output limit, at the target's price); when the call ends, settle to the actual cost. Parallel calls can therefore never overshoot a limit.
- A call that would exceed a budget is refused before it is sent, as a typed error naming the budget.

## Metering

- Every call produces the content-free `model.responded` event, with usage, cost, the price list it was computed from, route and target. There is no separate spend database: cost reports are views over the log.
- Calls glave makes on its own (the judge, compaction summaries) are metered and attributed to the thread that caused them.
- Provider prompt caching is passed through and its savings recorded. A response cache is off by default.

## Surfaces

- **In-process:** the `ModelProvider` port; the harness never knows a gateway is there.
- **As a service** (`glave gateway serve`): OpenAI Chat Completions and Responses, and Anthropic Messages, in; so any SDK or agent (including other coding agents) can be pointed at it. Every request needs a signed-in caller or a scoped key bound to a principal, never a bare shared key.
- Calls through the service that carry no thread are grouped into conversations by their shared message prefixes (hashed, as git finds shared history), so they too land in the log as threads with branches.

## Targets

- Overhead under 1 ms at p99 in-process, measured by `cargo xtask bench` against the fake provider with instruction-count gates.
- Bounded memory; one static binary; typed TOML config validated at start, with errors naming the field.
- No content in gateway logs, ever.
