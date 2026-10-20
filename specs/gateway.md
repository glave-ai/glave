# Gateway

The single door out for model calls and remote tools: which model serves each purpose, with whose keys, within what budget, and what it cost; and the tools that reach the outside world (search, image generation, speech, an organization's own systems), each with credentials only the gateway holds. Runs inside glave for a person and as a service for an organization, from the same crates.

The seam: the harness owns the tools that act on the computer and the rules for using any tool; the gateway owns everything that leaves for the outside world.

## One default seam

Decided by the owner, 2026-10-19 (#392): **OpenAI's API is the one default seam for models.** glave speaks Chat Completions to OpenRouter, OpenAI, Azure, Bedrock's and Vertex's compatible endpoints, and any server that copies them (Ollama, LM Studio, vLLM, an organization's own).

- **"Add a model" offers OpenRouter and any OpenAI-compatible server up front** (and OpenAI's own). A model's capabilities and price come from the service's model list ("Catalogs"), so a new provider needs no code when it speaks the seam.
- **A native format is an exception that must justify itself:** only where the compatible path loses something material, such as prompt caching, reasoning sent back, or an enterprise route served no other way. Anthropic's Messages API is one today, for Claude on Bedrock and Vertex (served only that way) and a direct key; it stays an internal route, not offered to add, and is expected to go once the routes glave uses no longer need it.
- **No new native format** without that justification written here first.

## Status

Each piece built has one line, which the pull requests that change it edit.

- **In process** (`crates/gateway`): routes by purpose, capability-checked and reasoning-sticky fallback, reserve-then-settle budgets held in memory (organization, person, and each thread within the threads above it), and `model.responded` version 2 recording the route, the target and any reasoning stripped. The harness sends every call with its purpose and the threads it counts in; the judge's calls are recorded in the thread whose action they judged. The CLI builds the gateway from `GLAVE_MODEL` (`main`) and `GLAVE_JUDGE_MODEL` (when it names another model, a choice the judge's calls ask for by name, so a project's own judge is asked for the same way), with no budgets, so nothing changes by default.
- **As a service** (`crates/gateway-service`, `glave gateway serve`): the OpenAI and Anthropic surfaces below, streamed or whole; every request signed in with the session API's OIDC verifier (`session.md`, "Signing in"); each caller held to the organization's policy on every call (`[models]`) and to a budget of their own per UTC day (`[budget] person_per_day_usd`, read back from their stream after a restart); every call recorded, content-free, in the caller's stream; and the tool catalog at `/mcp`. glave reaches a gateway elsewhere with `[gateway] url` ("Client mode").
- **Catalogs** (#290): each model account's service's own model list, kept in the store and listed again daily and on demand ("Catalogs", below); the accounts' models follow it.
- **One entry per model** (#289): accounts' models matched across services, tried in route order, described in `model/list` from the catalogs ("One entry per model", below).
- **Cloud contracts:** built for Azure OpenAI, Amazon Bedrock and Google Vertex AI, through their OpenAI-compatible APIs and, for Claude on Bedrock and Vertex, Anthropic's Messages API, keyless on Kubernetes ("Cloud contracts", below): `[provider]` in `config.toml`, for glave and for `glave gateway serve`.
- **Not yet:** the Responses surface, grouping stateless calls into threads, credential sources besides the environment and workload identity (the OS keychain, a KMS, a secrets manager), more than one provider at once, typed TOML configuration of routes (the policy's `[models]` allow-list is kept by the CLI when it builds the targets, `policy.md`), space-level budgets, policies that tell callers apart (groups from the identity provider; the service's `Policy` already gets each caller), budgets shared by several gateway processes, opaque reasoning through the surfaces, images, a sign-in command for client mode, and instruction-count gates for the overhead benchmark (`cargo xtask bench --bench overhead` measures wall-clock time today).

## Routes

A **route** names a purpose and maps it to an ordered list of **targets** (provider, model, region, credential reference), each with what it can do (tools, reasoning, images, context size).

- Purposes are a stable public vocabulary: `main`, `judge`, `summary` (compaction), `subagent`, `title` (a thread's title, `session.md`), and later `voice.listen`, `voice.speak`. A purpose with no route uses `main`.
- Routes are choices (nearest wins) and the models they may name are constraints (`policy.md`).
- **Choices.** Besides its routes, the gateway may offer targets a thread may choose (`session.md`, "Choosing a model and an approval mode"): in-process, the judge's model when it is another, and each of `[[models]]`. A call whose purpose uses `main`'s route and that asks for a choice by its name or model goes to that target alone; a purpose with a route of its own keeps it, and a call asking for `main`'s first model, or for any model that is not a choice, goes by its route as before.
- `model.responded` records the route and the target that actually served each call, so fallback is auditable.

## Fallback

- **Capability-checked:** a target that cannot honour the request (no tools, a smaller context than the conversation) is skipped, never sent a degraded request.
- **Thread-sticky:** a conversation that holds a provider's opaque reasoning falls back only to targets that accept it. Otherwise the reasoning is stripped explicitly and that is recorded.
- Failures that may pass (rate limits, overload, timeouts) move to the next target after the provider's own retries.

## Catalogs

What a model can do and costs is the provider's to say, never glave's: its context window, longest answer, whether it reads images, its price and the levels of reasoning effort it offers come from the service's own model list, read the same way from every service (OpenRouter's `GET /api/v1/models` says all of it, vLLM its context, most services little beyond the id). Nothing about a model is written into glave's code.

- **Kept in the store**, never in a cache server of its own: one catalog per service a person connected (its kind, and its URL for another server), each with when it was listed and its models as the list described them, in the catalog ledger (`events.md`). Only the latest `catalog.refreshed` is read.
- **Listed when an account is added**, by the call that checks its key; **daily**, when the server is next asked for its models or starts a turn after a catalog grew a day old, in a task no one waits for; and **on demand** (`provider/refresh`). A service that cannot be reached keeps its catalog and is tried again an hour later; one that answers the same is not written again.
- **An account's models are its service's catalog's**, those glave can talk with, newest first, at most a thousand: a model the service adds shows up from the next refresh, with no new glave and no reconnecting.

## One entry per model

The same model can be reached at several services (Claude Haiku from Anthropic or through OpenRouter, GPT from OpenAI or OpenRouter, an open model from OpenRouter or a person's own server). A person chooses the model; glave chooses the route (#289).

- **One name across services:** the id without its maker's prefix, in lower case, dots as hyphens, without a date at its end, so `claude-haiku-5-5-20261007` and `anthropic/claude-haiku-5.5` are one model. Ids that differ more show once per service.
- **Route order:** the settings' model (an organization's cloud contract) first, then the maker's own API (OpenAI, Anthropic), then an aggregator (OpenRouter), then any other server. A later service serves only when the one before fails in a way that may pass, by the fallback above; the entry's `id` is the first service's.
- **What `model/list` says of it** comes from the catalogs: its own name and maker (OpenRouter's `Anthropic: Claude Haiku 5.5`), the services in that order, its context, longest answer, images and reasoning levels from its first service's catalog, else from another's for the same model, and its price from the first service's alone, since that is where a call goes. The gateway checks requests against the same context and images, reserves budgets at that price, and records it as the price list (`<service> catalog <date>`) when the provider reports no cost of its own.

## Keys

- The gateway resolves credentials; nothing that calls it ever holds them.
- **Sources:** the OS keychain for a person; environment, a KMS, a secrets manager or workload identity for an organization; short-lived tokens refreshed outside the request path.
- **Cloud contracts are targets like any other:** Amazon Bedrock, Azure OpenAI, Google Vertex, and the company's own vLLM. That is how "data on our servers, models through our cloud contract" works (below).
- **How a provider is reached** (`glave_ports::auth::Auth`): nothing (a local server), a fixed key in the header the provider wants (`Authorization: Bearer`, Azure's `api-key`, Anthropic's `x-api-key`), or credentials that change: each request asks them for its headers, so one interface carries a bearer token refreshed before it expires (Entra, later Google) and a signature over each request (AWS SigV4). Credentials are kept until five minutes before they expire, and fetched outside any lock. A failure to get them is refused (`ProviderError::Credentials`) or, when the source could not be reached, passing.
- Keys and tokens never show: not in `Debug`, logs, errors or answers; header values are marked sensitive.

## Cloud contracts

`[provider]` in the person's or the server's own `config.toml` (never a project's: the organization's credentials go there) names a cloud's models in place of `GLAVE_BASE_URL`'s server; with both set glave refuses, naming both. Each model is reached through the cloud's OpenAI-compatible Chat Completions, so the requests are the ones glave sends any such server, except Claude on Bedrock and Vertex, which those clouds serve only through Anthropic's Messages API. The model is named as that cloud names it, and `[models]` rules see `azure/<deployment>`, `bedrock/<model id>` or `vertex/<model>`.

| | Azure OpenAI | Amazon Bedrock | Google Vertex AI |
| --- | --- | --- | --- |
| `config.toml` | `name = "azure"`, `resource` (a name, or an `https://` endpoint such as `https://acme.services.ai.azure.com`), optionally `scope` | `name = "bedrock"`, `region`, optionally `endpoint = "mantle"` and `api` | `name = "vertex"`, `project`, optionally `location` (`global` unless set; a multi-region `us` / `eu`, or a region such as `us-east5`) and `api` |
| Endpoint | `https://<resource>.openai.azure.com/openai/v1` (v1 API, no `api-version`) | `https://bedrock-runtime.<region>.amazonaws.com/openai/v1` (AWS's recommended), or `https://bedrock-mantle.<region>.api.aws/v1` for models only it serves; Claude at `…/anthropic/v1/messages` on the same host | `https://<host>/v1/projects/<project>/locations/<location>/endpoints/openapi` (host `aiplatform.googleapis.com`, `aiplatform.<us\|eu>.rep.googleapis.com` or `<region>-aiplatform.googleapis.com`); Claude at `…/locations/<location>/publishers/anthropic/models/<model>:streamRawPredict` |
| `model` | the deployment's name | the Bedrock model or inference profile id, such as `openai.gpt-oss-120b-1:0` or `us.anthropic.claude-sonnet-5` | as Vertex names it: `google/gemini-3.5-flash`, `claude-opus-5-5` |
| With a key | `GLAVE_API_KEY` or `AZURE_OPENAI_API_KEY`, sent as `api-key` | `GLAVE_API_KEY` or `AWS_BEARER_TOKEN_BEDROCK` (a Bedrock API key), sent as a bearer token, or in `x-api-key` to Claude | none: Vertex is reached with Google credentials |
| Keyless | AKS workload identity: `AZURE_FEDERATED_TOKEN_FILE`, `AZURE_CLIENT_ID`, `AZURE_TENANT_ID` (and `AZURE_AUTHORITY_HOST`), exchanged with Entra for a bearer token to `https://ai.azure.com/.default` unless `scope` says otherwise; the file read again for each exchange | SigV4 (signed as `bedrock`, or `bedrock-mantle`), with keys from `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`, else IRSA (`AWS_WEB_IDENTITY_TOKEN_FILE` + `AWS_ROLE_ARN`, through STS `AssumeRoleWithWebIdentity`), else EKS Pod Identity (`AWS_CONTAINER_CREDENTIALS_FULL_URI` + its token file) | Application Default Credentials, as Google's libraries find them: the file `GOOGLE_APPLICATION_CREDENTIALS` names (a service account key, `gcloud`'s user credentials, or workload identity federation), else `gcloud`'s own file, else the metadata server, which on GKE with Workload Identity gives the pod's identity; a bearer token to `https://www.googleapis.com/auth/cloud-platform` |
| Which API | Chat Completions | Claude (an id with an `anthropic.` part) by Anthropic's Messages API, signed or keyed as above; others by Chat Completions; `api = "anthropic"` or `"openai"` sends every model one way | Claude (an id starting `claude`) by Anthropic's Messages API on Vertex; others by Chat Completions; `api` as for Bedrock |

- A key in the environment wins over the workload's identity; with neither, glave refuses to start and says which to set.
- **Claude** is served by Bedrock and Vertex only through Anthropic's Messages API, so glave asks it through `glave-provider-anthropic`, with the same credentials, and every other model through Chat Completions; one provider either way to the harness, choosing per request by the model id. On Bedrock, `<endpoint base>/anthropic/v1/messages` takes Anthropic's own request (`anthropic-version: 2023-06-01`, the model in the body) and answers in server-sent events, signed for the same service as the endpoint's other APIs (`bedrock` or `bedrock-mantle`); AWS's binary event stream belongs to `InvokeModelWithResponseStream`, which glave does not use. SigV4 on the runtime's `/anthropic` route is not yet confirmed against AWS; `endpoint = "mantle"` is the documented signed path.
- `glave-provider-anthropic` speaks Vertex AI's Messages API (`Api::Vertex`): `POST <location base>/publishers/anthropic/models/<model>:streamRawPredict`, the same streamed request with `"anthropic_version": "vertex-2023-10-16"` in the body, no `model` there and no `anthropic-version` header; answered with the same server-sent events. A model id that cannot be part of the URL is refused before anything is sent, as an unknown model.
- The crates: `glave-azure` (the endpoint, and Entra tokens by a hand-rolled exchange), `glave-aws` (the endpoints, and SigV4 by AWS's own `aws-sigv4`, with a thin reader of the three key sources instead of the AWS SDK) and `glave-google` (Vertex's endpoints, and Application Default Credentials by hand-rolled exchanges: the metadata server, which on GKE gives the pod's workload identity, a service account key's RS256 assertion, `gcloud`'s user credentials, and workload identity federation from a file or URL token).

## Budgets

- Stacked: organization ⊇ project ⊇ person ⊇ thread ⊇ worker, matching "a worker's budget is within its parent's".
- **Reserve, then settle.** Before sending, reserve the worst case (input tokens plus the output limit, at the target's price); when the call ends, settle to the actual cost. Parallel calls can therefore never overshoot a limit.
- A call that would exceed a budget is refused before it is sent, as a typed error naming the budget.

## Metering

- Every call produces the content-free `model.responded` event, with usage, cost, the price list it was computed from, route and target. There is no separate spend database: cost reports are views over the log.
- Through the service, each caller's calls are recorded in a stream of their own in the gateway's scope of the log (`events.md`, "Gateway streams"): `model.responded` for each model call, `gateway.tool.called` for each catalog tool call. Who called is the stream itself. A caller's daily budget is read back from their stream when they first call in a day, so a restart never resets it.
- Calls glave makes on its own (the judge, compaction summaries) are metered and attributed to the thread that caused them.
- Provider prompt caching is passed through and its savings recorded. A response cache is off by default.

## Surfaces

- **In-process:** the `ModelProvider` port; the harness never knows a gateway is there.
- **As a service** (`glave gateway serve --http <address> --issuer <url> --audience <name> [--policy <file>]`): OpenAI Chat Completions (`POST /v1/chat/completions`) and Anthropic Messages (`POST /v1/messages`) in, streamed as server-sent events or answered whole; so any SDK or agent (including other coding agents) can be pointed at it. Responses is planned. Every request needs a signed-in caller, never a bare shared key: an OIDC access token as `Authorization: Bearer`, or in `x-api-key`, where Anthropic's SDKs put their key. Without one the answer is 401 with `WWW-Authenticate: Bearer`. The gateway's models and keys come from its own environment, as glave's do (`GLAVE_BASE_URL`, `GLAVE_API_KEY`, `GLAVE_MODEL` for `main`, `GLAVE_JUDGE_MODEL` for `judge`), or from a cloud contract, `[provider]` in the `config.toml` in its glave folder ("Cloud contracts").
- **Model names** map onto routes: a purpose (`main`, `judge`, `summary`, `subagent`, `title`) is that route, its targets in order; a target's name (`provider/model`) or model is that target alone; any other name is that model at the main route's first provider. Then the caller's policy keeps only the targets it allows; with none left the answer is 403 naming the rule.
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
