# Gateway

The single door out for model calls and remote tools: which model serves each purpose, with whose keys, within what budget, and what it cost; and the tools that reach the outside world (search, image generation, speech, an organization's own systems), each with credentials only the gateway holds. Runs inside glave for a person and as a service for an organization, from the same crates.

The seam: the harness owns the tools that act on the computer and the rules for using any tool; the gateway owns everything that leaves for the outside world.

## Status

In-process gateway built (`crates/gateway`): routes by purpose, capability-checked and reasoning-sticky fallback, reserve-then-settle budgets held in memory (organization, person, and each thread within the threads above it), and `model.responded` version 2 recording the route, the target and any reasoning stripped. The harness sends every call with its purpose and the threads it counts in; the judge's calls are recorded in the thread whose action they judged. The CLI builds the gateway from `GLAVE_MODEL` (`main`) and `GLAVE_JUDGE_MODEL` (`judge`, when it names another model), with no budgets, so nothing changes by default.

The gateway as a service is built too (`crates/gateway-service`, `glave gateway serve`): the OpenAI and Anthropic surfaces below, streamed or whole; every request signed in with the session API's OIDC verifier (`session.md`, "Signing in"); each caller held to the organization's policy on every call (`[models]`) and to a budget of their own per UTC day (`[budget] person_per_day_usd`, read back from their stream after a restart); every call recorded, content-free, in the caller's stream; and the tool catalog at `/mcp`. glave reaches a gateway elsewhere with `[gateway] url` ("Client mode").

Not built yet: the Responses surface, grouping stateless calls into threads, credential sources beyond the environment key, Bedrock, Azure and Vertex targets, typed TOML configuration of routes (the policy's `[models]` allow-list is kept by the CLI when it builds the targets, `policy.md`), space-level budgets, policies that tell callers apart (groups from the identity provider; the service's `Policy` already gets each caller), budgets shared by several gateway processes, opaque reasoning through the surfaces, images, a sign-in command for client mode, and instruction-count gates for the overhead benchmark (`cargo xtask bench --bench overhead` measures wall-clock time today).

## Routes

A **route** names a purpose and maps it to an ordered list of **targets** (provider, model, region, credential reference), each with what it can do (tools, reasoning, images, context size).

- Purposes are a stable public vocabulary: `main`, `judge`, `summary` (compaction), `subagent`, and later `voice.listen`, `voice.speak`. A purpose with no route uses `main`.
- Routes are choices (nearest wins) and the models they may name are constraints (`policy.md`).
- **Choices.** Besides its routes, the gateway may offer targets a thread may choose (`session.md`, "Models"): in-process, the judge's model when it is another, and each of `[[models]]`. A call whose purpose uses `main`'s route and that asks for a choice by its name or model goes to that target alone; a purpose with a route of its own keeps it, and a call asking for `main`'s first model, or for any model that is not a choice, goes by its route as before.
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
- Through the service, each caller's calls are recorded in a stream of their own in the gateway's scope of the log (`events.md`, "Gateway streams"): `model.responded` for each model call, `gateway.tool.called` for each catalog tool call. Who called is the stream itself. A caller's daily budget is read back from their stream when they first call in a day, so a restart never resets it.
- Calls glave makes on its own (the judge, compaction summaries) are metered and attributed to the thread that caused them.
- Provider prompt caching is passed through and its savings recorded. A response cache is off by default.

## Surfaces

- **In-process:** the `ModelProvider` port; the harness never knows a gateway is there.
- **As a service** (`glave gateway serve --http <address> --issuer <url> --audience <name> [--policy <file>]`): OpenAI Chat Completions (`POST /v1/chat/completions`) and Anthropic Messages (`POST /v1/messages`) in, streamed as server-sent events or answered whole; so any SDK or agent (including other coding agents) can be pointed at it. Responses is planned. Every request needs a signed-in caller, never a bare shared key: an OIDC access token as `Authorization: Bearer`, or in `x-api-key`, where Anthropic's SDKs put their key. Without one the answer is 401 with `WWW-Authenticate: Bearer`. The gateway's models and keys come from its own environment, as glave's do (`GLAVE_BASE_URL`, `GLAVE_API_KEY`, `GLAVE_MODEL` for `main`, `GLAVE_JUDGE_MODEL` for `judge`).
- **Model names** map onto routes: a purpose (`main`, `judge`, `summary`, `subagent`) is that route, its targets in order; a target's name (`provider/model`) or model is that target alone; any other name is that model at the main route's first provider. Then the caller's policy keeps only the targets it allows; with none left the answer is 403 naming the rule.
- **Fields.** What the neutral request carries is honoured: the system prompt (system and developer messages, or `system`), text messages, tool calls and their results (`is_error` too), function tools, `tool_choice` `auto` or `none`, the output limit (`max_tokens`, `max_completion_tokens`) and `stream`, with `stream_options.include_usage`. Fields that only describe the caller or say where to cache are accepted with no effect: `user`, `metadata`, `store`, `n = 1`, `parallel_tool_calls = true`, Anthropic's `cache_control` markers. Thinking blocks a client sends back are left out: the gateway cannot tell which model wrote them. Anything else is refused with 400 naming it (sampling settings such as `temperature`, `stop`, `response_format`, `thinking`, images, server tools), so a caller never gets an answer to a question it did not ask.
- **Answers** follow each API: Chat Completions chunks ending in `[DONE]` with tool calls in one chunk, the finish reason, and usage (cached and reasoning tokens as details, the cost in US dollars as an exact decimal when known); Messages events from `message_start` to `message_stop`, readable reasoning as `thinking` blocks, tool calls as `tool_use` blocks, usage split as that API counts it. Nothing is sent until the first fragment, so a call refused before it starts gets its own status: 402 for a budget, 403 for a policy, 404 for an unknown model, 429 when the provider is limiting requests, 502 when the provider refused the gateway's own key or failed, 504 when it went silent. A failure after the stream began ends it with that API's error event. A client that hangs up cancels the call.
- **Keys** never leave the gateway: every message it sends or logs has the provider key and the catalog's credentials replaced with `[redacted]`.
- Calls through the service that carry no thread are grouped into conversations by their shared message prefixes (hashed, as git finds shared history), so they too land in the log as threads with branches. Not built yet.

## Remote tools: the catalog

- Personally, the gateway's remote tools are the person's own MCP servers (`mcp.json`), as today: nothing new to set up.
- In an organization, the gateway serves its **catalog** over MCP (Streamable HTTP, stateless) at `/mcp`: the servers its policy's `[mcp] catalog` names, from the gateway's own `mcp.json` (every one there when the policy names none), each with credentials only the gateway holds (`${VAR}` filled from the gateway's environment). Every harness pointed at the gateway gets those tools, with no keys on people's machines.
- A catalog tool is named `<server>__<tool>`: the double underscore glave, Claude Code and Codex already put between a server and its tools, and within what every model API accepts in a name (letters, digits, `_`, `-`), so tools never collide across servers. glave shows each to the model as `mcp__<server>__<tool>`, exactly as if the server were the person's own.
- Everything else about a tool passes through as its server lists it: its description, schema and annotations (`readOnlyHint`, `openWorldHint`, …), which the harness uses to decide when to ask.
- Each caller sees and may call only the tools their policy allows (`Policy::refuses_tool`, asked with the signed-in caller); a call to another is refused as a tool error and never reaches the server.
- Every call is metered in the caller's stream as `gateway.tool.called`: the server, the tool, how long it took and whether it failed. Never its arguments or its result.

## Client mode

- `[gateway] url = "https://…"` in the person's `config.toml` (`policy.md`) sends glave's model calls to that gateway's `/v1/chat/completions`, with the person's sign-in from `GLAVE_GATEWAY_TOKEN` as the bearer token, and adds its `/mcp` catalog as one MCP source, before the person's own servers, so a catalog tool wins a clash of names.
- The model asked for is the person's choice (`--model`, `GLAVE_MODEL`, `model`), or the gateway's `main` route when they chose none; a judge model, when set, is asked for by name. The gateway keeps `[models]` and budgets for each caller, so glave does not check them itself in this mode.
- Only the person's own `config.toml` may set it, never a project's: the person's sign-in goes there. It must be `https`, or `http` only to this computer.
- Without `[gateway]`, glave builds its in-process gateway as before and sends exactly the requests it sent before.

## Targets

- Overhead under 1 ms at p99 in-process, measured by `cargo xtask bench` against the fake provider with instruction-count gates.
- Bounded memory; one static binary; typed TOML config validated at start, with errors naming the field.
- No content in gateway logs, ever.
