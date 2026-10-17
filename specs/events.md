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
| `tool.call.completed` | A tool call finished | `call_id`; `is_error`; `output` parts; optional `structured` result; optional `untrusted` (version 2), `true` when the output holds content from outside that no one vouches for, such as a web page (`approvals.md`, "Untrusted content"): resuming a thread rebuilds from it that the thread has read such content |
| `model.responded` | A model response finished | `model`; optional `response_id`; `stop`; optional `provider_stop`; `usage`; optional `cost`; optional `route` and `target`, the gateway route (the call's purpose: `main`, `judge`, `summary`, `subagent` or `title`) and the target that answered, and `reasoning_stripped`, true when opaque reasoning was left out because that target could not accept it (version 2). Content-free |
| `turn.ended` | A turn ends | `outcome`: `completed`, `cancelled`, `failed`, `interrupted` or `limit_reached` |
| `approval.requested` | A tool call waits for the person | `call_id` (also the approval's id); `tool`; `summary`, what would run as the person is shown it (`bash: rm -rf ~/notes`), never why it asks; `secret`, true when the call would show the model a file that usually holds secrets; optional `caution` (version 2), why the call needs the person whatever the mode, in sentences the person reads ("It deletes /home/ana/notes, outside the project."), and when the judge let it run, the judge's reason after it (`approvals.md`, "The judge"); optional `outside_sandbox` (version 3), when the command would run outside the sandbox (`approvals.md`, "Running outside the sandbox"): an object with an optional `reason`, the model's own words for why, kept apart from the `caution`, which is glave's |
| `approval.decided` | The person, or glave on their behalf, answered | `call_id`; `decision`: `allow_once`, `allow_for_thread`, `deny`, `no_one_there` (nothing to ask, so the call did not run) or `cancelled` (the turn was cancelled while it waited); optional `by` (version 2, see "Who acted") |
| `goal.set` | The person set a goal: the thread keeps working across turns until it ends. It replaces any goal the thread had | `objective`, the person's words, which say how to verify it if they want that; `token_budget`, the tokens (in and out, subagents' included) its turns may use; optional `by` (see "Who acted"). Version 2: version 1 also had an optional `check` command, which is read as a sentence appended to the objective ("The goal is met when \`<check>\` passes.") |
| `goal.ended` | The thread's goal ended | `reason`: `met` (the model verified it and said so with `update_goal`), `paused` (the model cannot go on without the person and said so with `update_goal`; the goal can be resumed), `stopped` (the person stopped it or cancelled its turn), `budget` (its token budget ran out) or `stalled` (three turns in a row without a tool call); `tokens`, what its turns used; optional `by`; with `paused`, `note`, the model's words to the person: what it needs from them. Version 2: version 1's `met_by_check` and `met_per_model` are read as `met` |
| `goal.resumed` | The person resumed a paused goal: it is active again, its tokens and turns counting on from where it paused | optional `by` |
| `workspace.bound` | A thread is bound to the workspace its tools act in: right after `thread.started` for a thread the session API starts; resuming reads the latest one. Glave's own record: call ids (`call_<n>`) do not count it, so the model is sent the same either way | `workspace`, its id; `computer` (`id`, `kind`: `local`, or `worker` for computers glave makes elsewhere, reserved); `opened`: `{"kind": "folder", "path"}` (worked on in place), `{"kind": "worktree", "repository", "branch"}`, `{"kind": "clone", "url", "at"?}` or `{"kind": "scratch", "path"}` (a folder of the thread's own, for a thread with no project; version 2, `computers.md`); `root`, the absolute workspace root as the computer names it |
| `workspace.removed` | A cleanup removed the thread's worktree, or moved it to the Trash (`computers.md`). Its branch is kept, so resuming the thread checks it out again. Like `workspace.bound`, call ids do not count it | `root`, the worktree's folder; optional `branch`; `reason`: `archived` (its thread was archived or deleted), `idle` (no events for 30 days), `repository_gone` (moved to the Trash) `pruned` (`glave worktrees prune`) or `released` (a worker released when its session ended; `branch` is where unsaved work was pushed, if any); optional `trash`, where it went; optional `lost` (version 2), for a released worker whose work could not be checked or pushed before it was destroyed: why, so nothing it had not pushed is lost silently |
| `thread.archived` | The thread was archived: hidden from lists and search, kept in the log. The latest of `thread.archived`, `thread.unarchived` and `thread.deleted` decides whether a thread is shown. Like `workspace.bound`, call ids do not count it | optional `by` (see "Who acted") |
| `thread.unarchived` | An archived thread was brought back | optional `by` |
| `thread.deleted` | The thread was deleted: hidden for good, and marked for erasure of its content (`storage.md`). Nothing brings it back | optional `by` |
| `model.chosen` | Someone chose the model the thread's turns go to (`session.md`, "Choosing a model and an approval mode"); the latest one is the thread's choice, and a thread with none uses the default. Like `workspace.bound`, call ids do not count it | `model`, the model's id as `model/list` names it; optional `by` |
| `approval.mode.chosen` | Someone chose how much the thread's turns ask before acting (`session.md`, "Choosing a model and an approval mode"); the latest one is the thread's mode, and a thread with none runs in the deployment's. Choosing the mode the thread already has records nothing. Like `workspace.bound`, call ids do not count it | `mode`: `ask` or `auto`; optional `by` |
| `thread.renamed` | Someone gave the thread a title (`session.md`, `thread/rename`), in place of the one its first message gave it, or glave did after its first exchange (`by` `{"kind": "glave"}`, `session.md`, "Titles"). The latest one is the title. Like `workspace.bound`, call ids do not count it | `title`; optional `by` |
| `hook.ran` | A hook (`approvals.md`, "Hooks") ran at a point of a turn | `point`: `pre_tool_use`, `post_tool_use`, `user_prompt_submit` or `stop`; `command`, as configured; optional `call_id`, the tool call it ran for; `result`: `passed`, `blocked` (it stopped the call or the prompt, or after a tool sent the model a reason) or `failed` (it could not run, timed out, or exited with neither 0 nor 2, and things went on as if it passed); optional `exit_code`; optional `reason`, the block's reason as the model or person was shown it, or why it failed. A `PreToolUse` block is also in the call's `tool.call.completed`, and a `PostToolUse` one in its output, so the conversation rebuilds from those alone |
| `task.started` | A thread began running as a task, with no one attached (`session.md`, "Tasks"). In the task ledger | `thread`; `title`; `model`; optional `budget` (tokens); optional `schedule` (`schedule`, its id, and `due`, when that run was due); optional `by` |
| `task.ended` | A task ended. In the task ledger | `thread`; `outcome`, as `turn.ended`'s (`cancelled` when stopped, `limit_reached` when its budget ran out, `interrupted` when no one ran it any more); `tokens` (usage counts, subagents' included); optional `cost`; optional `by`, who stopped it |
| `schedule.created` | A schedule was made. In the task ledger | `schedule` (a UUID v7); `when`: `{"cron": "<five fields>"}` (UTC) or `{"at": "<time>"}`; `prompt`; `goal` (true when each run works toward `prompt` as a goal; absent is false); `budget`, tokens each run may use; optional `by`. Version 2: every run works on a copy of its own (`session.md`, "Tasks"), so version 1's optional `worktree` is dropped on reading |
| `schedule.deleted` | A schedule was deleted; it starts no more runs. In the task ledger | `schedule`; optional `by` |
| `thread.compacted` | The conversation was summarized to fit the model's context window | `through`, the last `seq` the summary covers; `parts`, the message the conversation restarts from (the summary and the person's latest request). Rebuilding a conversation starts from the latest one; earlier events stay in the log |
| `gateway.tool.called` | A gateway serving many forwarded a call of a catalog tool (`gateway.md`, "Remote tools"). In the caller's gateway stream | `server`, the catalog server as the gateway's policy names it; `tool`, its name there; `is_error`; `millis`, how long the server took. Content-free: never the arguments or the result |
| `memory.saved` | A memory was saved (`memory.md`). In the space's memory ledger | `memory` (a UUID v7); `scope`: `personal` or `space`; `text`; optional `source` (`thread`, `seq`), where it was saved; `untrusted_read`, true when that thread had read untrusted content or a secrets file (absent is false); `by`. Sealed under the memory's own key |
| `memory.edited` | A memory's text changed. In the memory ledger | `memory`; `text`; `by`. Sealed under the memory's own key |
| `memory.forgotten` | A memory was forgotten. In the memory ledger | `memory`; `erase_at`, when its key is destroyed (now, or 30 days on when glave forgot it); optional `source` (`thread`, `seq`), where it was saved, kept here because the saved event's text is erased with it (since version 2; `recall` never returns that turn again); `by`. Content-free |
| `memory.restored` | A change glave made to a memory was undone. In the memory ledger | `memory`; `undone`, the id of the `memory.edited` or `memory.forgotten` undone; `by` |
| `memory.tidied` | Tidy-up ran over the ledger (`memory.md`, "Tidy-up"); its changes are `memory.edited` and `memory.forgotten` events of their own. In the memory ledger | optional `questions`: pairs of memory ids that disagree where tidy-up could not tell which holds, kept and asked about until either changes (absent is none); `by` (glave). Content-free |

| `provider.connected` | A model account was connected (`session.md`, "Model accounts"). In the person's accounts ledger | `provider` (a UUID v7); `kind`: `openai`, `anthropic`, `openrouter` or `compatible`; optional `url`, the server's API, for `compatible`; `key`; `models`, the models it offered when it was connected, the default first; `by`. Sealed under the account's own key |
| `provider.removed` | A model account was removed, its key destroyed. In the accounts ledger | `provider`; `by`. Content-free |
| `credential.connected` | A git host was connected (`session.md`, "Git hosts"). In the accounts ledger | `credential` (a UUID v7); `forge`: `github`, `gitlab`, `gitea`, `bitbucket` or `other`; `url`, the host's origin; `username`, sent with the token; `token`; optional `login`, whom the host said the token is for; optional `scopes`, what the host said the token may do (a classic GitHub token's or a GitLab token's); optional `renewal` (`expires`, `refresh`) for a token that expires, as a GitHub App's does; `by`. Sealed under the credential's own key |
| `credential.removed` | A git host was removed, its token destroyed. In the accounts ledger | `credential`; optional `renewed`, the credential that renewed it in its place (the same sign-in, so its id still names it); `by`. Content-free |
| `space.folder.set` | A project's folder was set: added, or where it comes from or is checked out changed (`storage.md`, "Spaces"). In the space ledger; for each folder, the latest one holds | `folder`, its id (a UUID v7); `source`: `{"kind": "repository", "url"}`, where it is cloned from, or `{"kind": "local"}`, a folder with no repository behind it; `checkout` (`computer`, the computer's id, kept across runs; `path`, absolute, links resolved); optional `by` |

**The task ledger.** A space's tasks and schedules are glave's own records about the space, not a conversation, so they live in a stream of their own: the reserved thread id `00000000-0000-7000-8000-000000000000` (a UUID v7 at time zero with nothing random, which no thread can have). Its events are only `task.started`, `task.ended`, `schedule.created` and `schedule.deleted`; it has no `thread.started`. Thread lists and reads leave it out. Any glave process of the deployment may append to it: a writer that meets the `(thread, seq)` conflict reads what was written and appends after it.

**The memory ledger.** Each space's memories live in a stream of their own too (`memory.md`, "Events"): the reserved thread id `00000000-0000-7000-8000-000000000001`. Its events are only `memory.saved`, `memory.edited`, `memory.forgotten`, `memory.restored` and `memory.tidied`; it has no `thread.started`, thread lists leave it out, and any glave process may append to it as to the task ledger. Once a memory's key is destroyed, its `memory.saved` and `memory.edited` read with `data` `{}`: the envelope stays, the text is gone.

**The accounts ledger.** The model accounts and git hosts a person connects live in a stream of their own in their personal space: the reserved thread id `00000000-0000-7000-8000-000000000002`. Its events are only `provider.connected`, `provider.removed`, `credential.connected` and `credential.removed`; it has no `thread.started`, thread lists leave it out, and any glave process may append to it as to the task ledger. A connected account, key or token included, is sealed under a key of its own, destroyed when the account is removed or replaced; its `provider.connected` or `credential.connected` then reads with `data` `{}`.

**The space ledger.** A space's own settings live in a stream of their own in the space: the reserved thread id `00000000-0000-7000-8000-000000000003`. Its events are only `space.folder.set`; it has no `thread.started`, thread lists leave it out, and any glave process may append to it as to the task ledger.

**Gateway streams.** A gateway serving many (`glave gateway serve`) records each caller's calls in a stream of their own in the gateway's own scope, whose thread id is the caller's person id (a UUID v7 at time zero, `session.md`, so it is never a thread's). Its events are only `model.responded`, one for each model call, and `gateway.tool.called`; it has no `thread.started`. Who called is the stream itself. Any number of gateway processes may append to it: an append that meets another's `(thread, seq)` reads what was written and tries again after it.

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
| `goal.set` v1 | optional `by` | who set the goal. Absent: the thread's `for` |
| `goal.ended` v1 | optional `by` | who ended it. `stopped`: the person who stopped it, absent meaning the thread's `for`. Every other reason is recorded by `{"kind": "glave"}` |
| `goal.resumed` v1 | optional `by` | who resumed it. Absent: the thread's `for` |

A goal's continuation, the message glave writes to start each of its turns after the first, is a `message.added` with `author` `{"kind": "glave"}`: it is never read as the person's request.

Every other event (tool calls, model responses, compaction) is the thread's `agent` acting for the thread's `for`, and carries no actor of its own. `hook.ran` is glave running a command the thread's `for` configured, or trusted their project to configure.

A subagent's thread has the parent's `for`, the subagent's profile as `agent` (the parent's `agent` until profiles have ids of their own), and `origin.kind` `subagent`; its `parent` keeps naming the call that started it.

### Who writes them

- `by`, `author` and `origin.by` come from the caller the session API verified (see `session.md`), never from request parameters. Locally, the caller is the local person.
- `for` and `agent` are set by the harness when the thread starts: `for` is the caller who started it (on a network transport the signed-in person; locally the local person), and `agent` the chosen profile. A subagent's thread has its parent's `for`.
- The model never sets any of them: none is a tool argument.

### Reading older versions

Each conversion is total, and changes no meaning:

- `thread.started` v1 and v2 read as v3 with `for` and `agent` absent, which means the deployment's local person and its default agent (the only values possible before these fields existed), and with `origin.kind` `subagent` when `parent` is set, `fork` when `fork` is set, and `prompt` otherwise. In v3 itself, `for` and `agent` are always written.
- `turn.started` v1 reads as v2 with no `trace` and no `auth`.
- `message.added` v1 reads as v2 with no `author`.
- `approval.decided` v1 reads as v2 with no `by`, except that `no_one_there` and `cancelled` read with `by` `{"kind": "glave"}`.
- `approval.requested` v1 reads as v2 with no `caution`: a caution's reason, and the judge's, stay folded into its `summary` in brackets, as they were written. v2 reads as v3 with no `outside_sandbox`.
- `workspace.bound` v1 reads as v2 unchanged: version 1 never has a `scratch` kind.
- `schedule.created` v1 reads as v2 without its `worktree`, the one conversion that drops a choice (allowed while version 1 is a draft): every run now works on a copy, so a schedule that said `false` runs on one too.

## Storage

- The log is one table. The envelope members are real columns, with `(thread, seq)` unique. `data` is stored as the exact bytes written, encrypted, next to the id of the key that sealed it.
- SQLite (encrypted) on a personal computer; Postgres for organizations. One storage interface, two implementations.
- Every storage call takes a scope: organization and space. No call can cross scopes. Each organization's log lives in that organization's own storage.
- Views (threads, messages, tool calls, usage) are ordinary tables built from the log. They are disposable and can be rebuilt at any time, so their structure can change freely.
- **The threads view** is built: one row per thread (`glave_ports::ThreadView`), folded from its events in the same transaction as each append, so it is never behind the log. It holds the thread's standing, `for`, `origin`, `parent`, first and latest times, the `seq` of the event its title comes from (never the title), turns, the latest outcome, and its own model calls' tokens and cost. Lists read it by standing, most recently written first, a page at a time after a cursor. A stream without `thread.started` at `seq` 1 (the ledgers, the gateway's streams) has no row. Every store must keep it, and the event log conformance suite checks that a view rebuilt from the log (`rebuild_thread_view`) equals the one kept by appends. A store rebuilds every view when it opens if they were built by an older fold (`THREAD_VIEW_VERSION`), as after the upgrade that made the table; a thread that cannot be read (its key erased) gets no row.
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
