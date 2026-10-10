# Events

glave records everything that happens as events in an append-only log. One format serves every model provider, every surface (CLI, desktop, web, workers) and every deployment (a personal computer, a company's servers, a company's cloud account). The event written to disk is the event sent over the wire.

The reference implementation is the `glave-event` crate. Where this document and the crate disagree, that is a bug in one of them.

## Status

Envelope version 1. Draft until glave 0.1 ships: until then, version 1 payloads may still change. From 0.1 on, every published payload version is frozen.

## The envelope

Every event is one JSON object with exactly these eight members:

```json
{
  "glave": 1,
  "id": "0192f5c2-7c1e-7b3a-9d4f-2a6c1e8b5d07",
  "thread": "0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c10",
  "seq": 42,
  "at": "2026-10-07T21:14:03.512Z",
  "type": "tool.call.requested",
  "v": 1,
  "data": { "call_id": "call_7", "tool": "bash", "arguments": { "json": { "command": "ls" } } }
}
```

| Member | Type | Meaning |
| --- | --- | --- |
| `glave` | integer | Envelope version. Currently `1`. A reader rejects envelope versions it does not know. |
| `id` | UUID v7 string | Unique everywhere. Time-ordered, but never used for ordering. |
| `thread` | UUID v7 string | The thread (conversation or task) the event belongs to. |
| `seq` | integer ≥ 1 | Position in the thread. Assigned gap-free by the thread's single writer. This is the order of events. |
| `at` | RFC 3339 UTC timestamp | When the event was recorded, always written with three fractional digits (`2026-10-07T21:14:01.250Z`) so each instant has one spelling. For people; never for ordering. |
| `type` | string | What happened: lowercase dotted segments, such as `tool.call.requested`. At most 64 bytes. |
| `v` | integer ≥ 1 | Version of this type's payload. |
| `data` | object | The payload. Its shape is fixed by `type` and `v`. |

The envelope carries no content. Content lives only in `data`, which storage encrypts.

## Versioning

There are two levels.

- **The envelope** (`glave`) changes only to add or change envelope members. Readers reject unknown envelope versions instead of guessing.
- **Each payload** (`v`) has its own version. Every change to a payload's shape or meaning bumps `v`, including adding an optional member.

Writers always write the latest version of each type. Stored events are never rewritten to a newer version. Readers upgrade old events as they read them, through a chain of pure conversions: `v1 → v2 → v3`.

A new version must be convertible from the one before it. If it cannot be, it is not a new version: it is a new event type.

### Changing a payload

1. Keep the current struct, renamed with its version (for example `ToolCallRequestedV1`).
2. Write the new struct under the type's normal name and bump its `VERSION`.
3. Implement `From<OldVersion> for NewVersion`. It must be total: no failure path.
4. Route the old version through that conversion in the type's decoder.
5. Add the new version to the known-version list and add its fixture (`fixtures/<type>/v<N>.json`).
6. Never edit an existing fixture. CI decodes every fixture and fails if any known version lacks one.

### Unknown events

A reader keeps events it does not understand, whether an unknown type or a newer version of a known type. It passes them through untouched and never drops them.

## Ordering and writers

- Each thread has one writer at a time. The writer assigns `seq` as the previous `seq` plus one, and storage rejects a duplicate `(thread, seq)` in the same transaction as the write.
- A thread's first `seq` is normally `1`. It can be higher only after retention erased an earlier stretch; that erasure is itself recorded in the thread.
- Order events by `seq` within a thread. Never order by `at` or `id`.

## Event types

Types are at version 1 unless the table says otherwise.

| Type | Recorded when | `data` |
| --- | --- | --- |
| `thread.started` | A thread begins; always `seq` 1 | `producer` (`name`, `version`); optional `fork` (`thread`, `seq`) when the thread continues another thread's prefix; optional `parent` (`thread`, `call_id`) when an `agent` call in another thread started it, as a subagent (version 2); `for`, `agent` and `origin` (version 3, see "Who acted") |
| `turn.started` | A turn begins | `model` (`provider`, `name`); `config`, a blob reference to the turn's full configuration (system prompt, tools, parameters); optional `trace` and `auth` (version 2, see "Who acted") |
| `message.added` | A finished message from the person or the assistant | `role` (`user` or `assistant`); `parts`, an ordered list of `text`, `reasoning` and `file` parts; optional `author` (version 2, see "Who acted") |
| `tool.call.requested` | The model asks for a tool | `call_id`; optional `provider_call_id`; `tool`; `arguments`, either `{"json": {…}}` or `{"malformed": "<raw text>"}` |
| `tool.call.completed` | A tool call finished | `call_id`; `is_error`; `output` parts; optional `structured` result |
| `model.responded` | A model response finished | `model`; optional `response_id`; `stop`; optional `provider_stop`; `usage`; optional `cost`. Content-free |
| `turn.ended` | A turn ends | `outcome`: `completed`, `cancelled`, `failed`, `interrupted` or `limit_reached` |
| `approval.requested` | A tool call waits for the person | `call_id` (also the approval's id); `tool`; `summary`, as the person is shown it; `secret`, true when the call would show the model a file that usually holds secrets |
| `approval.decided` | The person, or glave on their behalf, answered | `call_id`; `decision`: `allow_once`, `allow_for_thread`, `deny`, `no_one_there` (nothing to ask, so the call did not run) or `cancelled` (the turn was cancelled while it waited); optional `by` (version 2, see "Who acted") |
| `thread.compacted` | The conversation was summarized to fit the model's context window | `through`, the last `seq` the summary covers; `parts`, the message the conversation restarts from (the summary and the person's latest request). Rebuilding a conversation starts from the latest one; earlier events stay in the log |

Details shared by several types:

- **Reasoning parts** carry an optional `text` (whatever readable reasoning the provider returned) and an optional `opaque` (`provider`, `model`, `kind`, `data`). `data` is stored byte-exact and replayed only to the same provider.
- **File parts** and the `config` reference use a blob reference: `sha256` (64 lowercase hex digits), `bytes`, `media_type`. Large outputs are stored once by hash and referenced.
- **Stop reasons** are normalized across providers: `end_turn`, `tool_use`, `max_tokens`, `stop_sequence`, `context_exceeded`, `content_filter`, `refusal`, `pause`, `malformed_tool_call`, `error`, `cancelled`, `interrupted`. The provider's exact value goes in `provider_stop`.
- **Usage** counts are inclusive totals with subsets: `input` includes `cache_read` and `cache_write`, and `output` includes `reasoning`. `estimated` is true when a stream ended before the provider reported usage. The provider's own usage object can ride along as `raw`.
- **Money** is an integer count of millionths plus an ISO 4217 currency code: `{"micros": 4200, "currency": "USD"}` is $0.0042. A cost also names the `price_list` it was computed from.
- **Deltas are never events.** Streaming fragments are sent live; the finished message, call or response is the event.

## Who acted

Every thread records, once, whose work it is and which agent does it. The few events where someone else may act say who. Together they answer, for any event, who did this and on whose behalf. Nothing here is in the envelope: the envelope stays content-free, and these fields live in `data`, sealed and erasable with the rest.

### Principals

A principal is anyone or anything that can act, written as a reference:

```json
{ "kind": "person", "id": "0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c10" }
```

| `kind` | Is | `id` |
| --- | --- | --- |
| `person` | a human | their person ID: created on first run for a local person; from sign-in for an organization |
| `agent` | an agent profile acting: the default agent, a subagent type, a named assistant | the profile's ID |
| `group` | a team acting as one | the group's ID |
| `glave` | glave itself, deciding on its own (a timeout, a cancellation, a rule) | none |

- `id` is a UUID v7, unique everywhere and never tied to one deployment. It is required for every kind but `glave`.
- `kind` is an open string. A reader keeps a reference with a kind it does not know, untouched, as it keeps unknown events.
- People, agents and groups share one ID space. Nothing in the log branches on whether a person or an agent acted; it records which.

### The fields

| Type and version | Member | Meaning |
| --- | --- | --- |
| `thread.started` v3 | `for` | the principal whose work this is (a person, or later a group): their budget, their authority, their record |
| | `agent` | the agent profile that runs the thread (kind `agent`) |
| | `origin` | what started the thread: `kind` (open string; known values `prompt`, `voice`, `schedule`, `webhook`, `chat`, `subagent`, `fork`, `assistant`), optional `ref` (an opaque pointer, such as a schedule or a chat thread), optional `by` (the principal who started it, when that differs from `for`) |
| `turn.started` v2 | optional `trace` | the W3C trace ID (32 lowercase hex digits) the turn belongs to, linking the event to OpenTelemetry spans |
| | optional `auth` | how the caller who started the turn signed in: `method` (`local`, `oidc` or `saml`), and for `oidc` and `saml` the `issuer`, `subject` and `session`. Never a token or any other secret |
| `message.added` v2 | optional `author` | who wrote the message. Absent: the thread's `for` for a user message, its `agent` for an assistant message |
| `approval.decided` v2 | optional `by` | who decided. Absent: the thread's `for`. `no_one_there` and `cancelled` are decided by `{"kind": "glave"}` |

Every other event (tool calls, model responses, compaction) is the thread's `agent` acting for the thread's `for`, and carries no actor of its own.

A subagent's thread has the parent's `for`, the subagent's profile as `agent` (the parent's `agent` until profiles have ids of their own), and `origin.kind` `subagent`; its `parent` keeps naming the call that started it.

### Who writes them

- `by`, `author` and `origin.by` come from the caller the session API verified (see `session.md`), never from request parameters. Locally, the caller is the local person.
- `for` and `agent` are set by the harness when the thread starts, from the space's owner and the chosen profile.
- The model never sets any of them: none is a tool argument.

### Reading older versions

Each conversion is total, and changes no meaning:

- `thread.started` v1 and v2 read as v3 with `for` and `agent` absent, which means the deployment's local person and its default agent (the only values possible before these fields existed), and with `origin.kind` `subagent` when `parent` is set, `fork` when `fork` is set, and `prompt` otherwise. In v3 itself, `for` and `agent` are always written.
- `turn.started` v1 reads as v2 with no `trace` and no `auth`.
- `message.added` v1 reads as v2 with no `author`.
- `approval.decided` v1 reads as v2 with no `by`, except that `no_one_there` and `cancelled` read with `by` `{"kind": "glave"}`.

## Storage

- The log is one table. The envelope members are real columns, with `(thread, seq)` unique. `data` is stored as the exact bytes written, encrypted, next to the id of the key that sealed it.
- SQLite (encrypted) on a personal computer; Postgres for organizations. One storage interface, two implementations.
- Every storage call takes a scope: organization and space. No call can cross scopes. Each organization's log lives in that organization's own storage.
- Views (threads, messages, tool calls, usage) are ordinary tables built from the log. They are disposable and can be rebuilt at any time, so their structure can change freely.
- JSON Lines, one event per line, is the export and import format.

## Retention

- Content is encrypted under keys scoped by organization, space and month (thread later). Destroying a space's key erases its content everywhere at once, including backups; erasing less than a space also rotates the space key (see `storage.md`).
- Envelopes and content-free events, such as `model.responded`, can outlive the content they describe, for audit and billing.
- Erasure is the only change ever made to a stored event, and it is itself recorded as an event in the same thread.
- A legal hold is an event, and it pauses all deletion in its scope.

## Not in version 1

Each of these is a later, separate decision, made when it is needed:

- A per-event hash chain (`prev`) that makes receipts provable.
- Trace ids on the envelope, for OpenTelemetry. (A turn already records its trace ID in `turn.started`; see "Who acted".)
- A `critical` flag, which lets older readers refuse an event they must not skip.
- Event types for receipts, retention and holds. They arrive with the features that write them. (Approval events arrive with the session API; see `session.md`.)

The envelope version exists so that any of these can be added without touching stored events.

## Export mappings

The stored format is glave's own. CloudEvents and OpenTelemetry are mappings applied at export: an event becomes a CloudEvent with `source` naming the thread and `type` carrying the glave type and version, or an OpenTelemetry log record with `EventName` set to the type. The OpenTelemetry generative-AI conventions are still in development, so they are mapped at export and never stored.
