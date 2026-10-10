# Session API

How every surface talks to the harness: the CLI, a desktop or web app, an editor, another program that embeds glave. Tracks issue #34. The events it carries are in `events.md`; what asks the person is in `approvals.md`.

## Principles

- **One protocol on every transport.** JSON-RPC 2.0, the format of MCP, A2A, ACP and LSP, over: in-process (the CLI), stdio, a Unix socket, HTTP with server-sent events, and WebSocket. The same conformance tests run over each. A transport changes how messages travel, never what they mean.
- **No feature needs a particular transport.** Anything a client can do over WebSocket it can do over HTTP, perhaps with one more round trip.
- **No state lives in a connection.** The log is the truth. A client that drops reconnects anywhere and asks for everything after the last `seq` it saw.
- **Stateless hosting is real.** A server may keep an active thread in memory, or rebuild it from the log for every request; clients cannot tell the difference.
- **The CLI is a client like any other.** It gets nothing the API does not offer.

## Messages

Standard JSON-RPC 2.0: requests carry `jsonrpc`, `id`, `method` and `params`; responses carry the same `id` and `result` or `error`; notifications have no `id`. Batches are not used.

Every request may carry `params._meta.protocol`, the protocol version the client speaks (today `2026-10`). Without it, the server assumes the oldest version it supports. There is no handshake, so a stateless server needs no memory of the client. A server that does not speak the version answers with error `-32010` and the versions it does speak.

Every request may also carry `params._meta.traceparent`, a W3C trace context. The server continues that trace, or starts one, and records the trace ID on each turn the request starts (`turn.started.trace`).

### Who is calling

Every request is made by a verified caller, a principal as `events.md` defines it. The transport establishes who: on in-process, stdio and Unix socket transports, the local person; on network transports, the person the server authenticated (OIDC or SAML, per request or per session). A caller is never taken from request parameters. The server stamps the caller into what it records: `origin.by` when a thread starts, `author` on a message from someone other than the thread's `for`, `by` on an approval decision, and `auth` on the turn's start.

## Methods

| Method | Params | Result |
| --- | --- | --- |
| `server/discover` | none | `name`, `version`, `protocols` (the versions spoken), `transports` |
| `thread/start` | none | `thread`: a thread summary |
| `thread/list` | optional `limit` (default 20) | `threads`: summaries, newest first |
| `thread/get` | `thread` | `thread`: its summary |
| `thread/events` | `thread`; optional `after` (a `seq`), `limit` (default 500) | `events`: stored events in `seq` order, exactly as `events.md` defines them; `next`, when there are more |
| `thread/watch` | `thread`; optional `after` | streams the thread's notifications from `after` on, until the client stops listening |
| `turn/start` | `thread`; `prompt` | streams notifications while the turn runs, then answers with `turn`: how it ended and what it cost |
| `turn/cancel` | `thread` | `{}` once the running turn, or compaction, has ended as cancelled |
| `thread/compact` | `thread` | `before_tokens`, `after_tokens` |
| `approval/decide` | `thread`; `approval` (its id); `decision`: `allow_once`, `allow_for_thread` or `deny` | `{}`; the paused turn continues |
| `question/answer` | `thread`; `question` (the `question` call's id); `answers`, one string for each question, in order | `{}`; the paused turn continues |

A **thread summary** has `id` and `status`: `idle`, `running` or `waiting` (a turn is paused for an approval or an answer). Titles, creation and update times arrive with a threads view built from the log.

A **turn result** has `outcome` (`completed`, `cancelled`, `failed`, `interrupted`, `limit_reached`), `model_calls`, `tool_calls`, `tokens` and `cost` as in `events.md`.

Each server serves one workspace and one scope in version 1. Choosing among several workspaces arrives with issue #29; choosing a space arrives with organizations (issue #22).

## Notifications

Two kinds, sent while a turn runs or a client watches:

| Method | Params | Kept |
| --- | --- | --- |
| `thread/event` | `thread`; `event`, a stored event with its `seq` | Yes: it is in the log, and `thread/events` returns it again |
| `turn/live` | `thread`; `update`: `text` or `reasoning` (a fragment), `tool_started`, `tool_finished`, `waiting`, `retrying`, `compacted`, `asked`, `subagent_started` (`subagent`, `description`), `subagent_ended` (`subagent`, `ok`) | No: live updates are for showing progress, never replayed |

A thread's notifications include its subagents': a child thread started by an `agent` call sends its events and progress to whoever follows its parent too, each under the child's own `thread`. Its first event, `thread.started`, names the parent call, and `subagent_started` gives its description. A client answers a subagent's approval with the subagent's thread.

A client that only shows the conversation needs `thread/event` alone. Rebuilding a thread from `thread/events` and then following `thread/watch` gives exactly the same state as having watched from the start.

## Approvals

A tool call that needs the person records `approval.requested` (`approval`, `call_id`, `tool`, `summary`, `secret`) and the turn pauses; the thread's status becomes `waiting`. Any client may answer with `approval/decide`, now or much later, from any transport. The answer is recorded as `approval.decided` (`approval`, `decision`), and the turn continues from the log, in whichever process receives it.

- A decision for an approval that is not pending is refused with `-32004`.
- `allow_for_thread` is recorded, so it survives resuming the thread.
- Cancelling a paused turn denies its pending approval and ends the turn as cancelled.
- With no client that can answer, as in a script, the server decides on its behalf as `approvals.md` describes ("no one at the keyboard").

These are the approval events `events.md` deferred; they arrive with this API at version 1.

## Questions

When a client can answer, the model is offered `question`: up to four questions, each with optional choices (`label`, `description`) and `multiple` when more than one may be picked. The questions are the `question` call's arguments, so the log keeps them in `tool.call.requested`. Once the turn is waiting for the answers it sends `turn/live` with `asked` (`question`, the call's id, and `questions`); any client may then answer with `question/answer`. The answers become the call's result.

- An answer for a question that is not waiting is refused with `-32006`.
- Cancelling the paused turn ends the call unanswered, and the turn as cancelled.
- With no client that can answer, `question` is not offered at all.

## Turns

- One turn runs per thread at a time. A second `turn/start` is refused with `-32002`. Queueing and steering a running turn come later.
- A turn's notifications go to the client that started it and to every client watching the thread.
- If the server stops mid-turn, the log ends without `turn.ended`. Resuming records the turn as `interrupted` and gives every unanswered tool call a result, so the thread continues.

## Errors

Standard codes, `-32700` to `-32603`, for malformed messages. glave's own:

| Code | Meaning |
| --- | --- |
| `-32001` | No such thread |
| `-32002` | A turn is already running in this thread |
| `-32003` | No turn is running in this thread |
| `-32004` | No such approval is pending |
| `-32005` | The operation was cancelled, such as a compaction stopped by `turn/cancel` |
| `-32006` | No such question is waiting |
| `-32010` | Unsupported protocol version; `data.supported` lists the spoken ones |
| `-32011` | The thread is running in another server of this deployment |
| `-32012` | The caller may not do this |
| `-32013` | No valid sign-in (network transports) |
| `-32020` | The model provider failed; `data` has `failure` (its kind), `message` and, when the person can fix it, `fix` |

## Transports

| Transport | Framing | Who uses it | Authentication |
| --- | --- | --- | --- |
| In-process | Typed messages over a channel; nothing is serialized | The CLI; programs that link glave's crates | None: same process |
| stdio | One JSON message per line (`glave serve --stdio`) | Programs that run glave as a child process | None: the parent started it |
| Unix socket | One JSON message per line | Local clients of a long-running glave | File permissions on the socket |
| HTTP + SSE | `POST` one request; the answer is JSON, or for streaming methods an `text/event-stream` of notifications ending with the response. The method is also sent as the `Glave-Method` header, so proxies and logs can see it | Web, desktop and remote clients | Required |
| WebSocket | One JSON message per text frame | Clients that want one connection for many threads; later, realtime features | Required |

A network listener never runs without authentication: there is no "add it later". Closing a streaming HTTP response stops only that stream; the turn goes on, and `thread/watch` picks it up again.

## Over the network

What changes when a client is on another machine: who is calling, how a server that may restart keeps a paused turn, and how several servers share threads.

### Signing in

- **Local transports need no sign-in:** in-process, stdio and a Unix socket act as the local person.
- **Network transports always need it.** A server accepts:
  - **OIDC access tokens** (`Authorization: Bearer …`), checked against the identity provider's published keys (JWKS), with `iss`, `aud` and expiry verified. The CLI gets one by the device flow; browsers and the desktop app by authorization code with PKCE.
  - **SAML,** for organizations that use it: a browser signs in through the identity provider, and glave issues its own short-lived session token, signed by the organization's key.
  - **glave session tokens,** the same signed tokens, so any server of the organization can verify a caller without shared memory.
- **Who the caller is:** the token's `iss` and `sub` map to a person through the identity source (SCIM provisions the mapping in an organization). The verified person is the caller for every request on that connection or HTTP request, and is stamped as "Who is calling" describes. A token never enters the log: `turn.started.auth` records the method, issuer, subject and session only.
- **Expiry:** a stream outlives its token only until the token expires; the client reconnects with a fresh one and continues with `thread/watch` from its last `seq`.

### Turns that outlive a server

- **A paused turn is durable.** Its `approval.requested` (or the `question` call) is already in the log. Any server of the deployment can take a `approval/decide` or `question/answer` for it, rebuild the thread from the log, record the decision and continue the turn. A server that stops while a turn waits no longer leaves it interrupted.
- **One harness at a time per thread.** A server running or continuing a turn holds a lease on the thread in the data store, renewed while it works. Another server asked to act on the thread answers error `-32011` (running elsewhere) until the lease ends. The log's unique `(thread, seq)` remains the final guard, so a lost lease can never corrupt a thread.
- **Retrying is safe.** `turn/start` may carry `params._meta.idempotency_key`. A server that sees the same key for the same thread returns the turn already started instead of starting another, so a client retrying after a dropped connection never runs a turn twice.

### Several clients

- Any number of clients may `thread/watch` the same thread; each sees every event once, in `seq` order.
- Who may do what (start a turn, answer an approval, read a thread) is checked against the caller on every request, from the space's owner and the organization's policy (issue #22). A request from someone who may not make it is refused with error `-32012` and never recorded; a network request without a valid sign-in is refused with `-32013`.

## Status

Version 1, in-process (`glave-session`, used by the CLI) and over stdio (`glave serve --stdio`, the same server through `rpc::serve`; one conformance suite runs over both): every method but `thread/watch`, whose in-process form is `Server::watch`. Turns pause for approvals in memory: a server that stops while a turn waits leaves it to be resumed as interrupted. "Over the network" above is specified ahead of the code (issue #64): sign-in, durable paused turns, leases and idempotent turns.

## Later

- WebSocket-only realtime channels, such as voice (issue #58), as optional features.
- Steering and queueing turns.
- ACP and A2A adapters, when a real client needs them.
- A JSON Schema of every message, generated from the Rust types, and a TypeScript client generated from it.
