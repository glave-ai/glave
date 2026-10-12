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

Every request may also carry `params._meta.traceparent`, a W3C trace context. The server continues that trace, or starts one, and records the trace ID on each turn the request starts (`turn.started.trace`). Built: `turn/start` reads it (a value that is not a version-00 traceparent is ignored, as W3C says, never refused); each request runs in a `session.request` span named by its method, which joins the caller's trace when glave exports to OpenTelemetry (the `otlp` feature). Spans and metrics are content-free: ids, names, counts, durations and outcomes, never prompts, answers, arguments, output, file contents or secrets.

### Who is calling

Every request is made by a verified caller, a principal as `events.md` defines it. The transport establishes who: on in-process, stdio and Unix socket transports, the local person; on network transports, the person the server authenticated (OIDC or SAML, per request or per session). A caller is never taken from request parameters. The server stamps the caller into what it records: `origin.by` when a thread starts, `author` on a message from someone other than the thread's `for`, `by` on an approval decision, and `auth` on the turn's start.

## Methods

| Method | Params | Result |
| --- | --- | --- |
| `server/discover` | none | `name`, `version`, `protocols` (the versions spoken), `transports`; `approval`, how much the server asks before acting in a thread that chose no mode: `ask` or `auto` (`approvals.md`, "Modes"), from `--approval` or `approval` in `config.toml`; `approvals`, the modes a thread may choose, cautious first ("Approval modes") |
| `model/list` | none | `models`: the models this server can run, the default first ("Models") |
| `thread/start` | optional `space` (default: the server's own space, when the caller works in it; else the caller's personal space, made the first time, as for a one-off chat from the web); optional `worktree`: `true` for a git worktree of its own on a `glave/<id>` branch, `false` for the folder itself (default: the server's `worktree` choice, `policy.md`); optional `background`: `true` for a thread that will run as a task, which works on a worker when the server has a worker backend, even if other threads work here; optional `model`, one `model/list` names, for its turns (default: the server's default model); optional `approval`, `ask` or `auto`, one of the server's `approvals`, for its turns (default: the server's mode); optional `computer`: the id of a running computer to work on, such as a person's own laptop dialled in to this server (not available yet: refused with `-32022`, and no thread starts) | `thread`: a thread summary, which says nothing about where the thread works, so a client never assumes a worker. A model this server does not run is refused with `-32021`, and a mode it does not leave with `-32012`; either way no thread starts. A worktree that cannot be made (the folder is not in a repository) is refused with `-32009`, and no thread starts |
| `thread/list` | optional `space` (default as `thread/start` picks it); optional `limit` (default 20); optional `show`: `active` (default), `archived`, or `deleted` (waiting for erasure, for the operator that does it); optional `cursor`, the `next` of the page before | `threads`: summaries, most recently written first, read from the threads view a page at a time (its cost is the page's, however many threads or events the space has); `next`, the cursor of the following page, when there is one. A `cursor` this server did not make is refused with `-32602` |
| `thread/rename` | `thread`; `title`: 1 to 200 characters, trimmed, not only spaces | `thread`: its summary with the new title. Records `thread.renamed`, glave's own record, so the model is sent the same. A title that is empty or too long is refused with `-32602`; a thread with a turn running with `-32002` |
| `thread/archive` | `thread` | `{}`. Records `thread.archived`; the thread leaves `thread/list` and search but stays in the log and readable by id. A turn, goal or compaction in it is refused with `-32014`. Archiving an archived thread records nothing |
| `thread/unarchive` | `thread` | `{}`. Records `thread.unarchived` |
| `thread/delete` | `thread` | `{}`. Records `thread.deleted`: the thread is hidden for good, and every method but `thread/list` with `show: deleted` answers `-32001` for it |
| `thread/get` | `thread` | `thread`: its summary |
| `thread/events` | `thread`; optional `after` (a `seq`), `limit` (default 500) | `events`: stored events in `seq` order, exactly as `events.md` defines them; `next`, when there are more |
| `thread/watch` | `thread`; optional `after` | streams the thread's notifications from `after` on, until the client stops listening |
| `turn/start` | `thread`; `prompt`; optional `background` (see "Tasks"), and with it optional `budget` (tokens); optional `model`, one `model/list` names, for this turn and the thread's turns after it ("Models"); optional `approval`, one of the server's `approvals`, likewise ("Approval modes") | streams notifications while the turn runs, then answers with `turn`: how it ended and what it cost. With `background`, answers at once with `task` |
| `turn/cancel` | `thread` | `{}` once the running turn, or compaction, has ended as cancelled |
| `thread/compact` | `thread` | `before_tokens`, `after_tokens` |
| `approval/decide` | `thread`; `approval` (its id); `decision`: `allow_once`, `allow_for_thread` or `deny` | `{}`; the paused turn continues |
| `question/answer` | `thread`; `question` (the `question` call's id); `answers`, one string for each question, in order | `{}`; the paused turn continues |
| `goal/start` | `thread`; `objective`, with how to verify it in words; optional `budget` (tokens; default 1,000,000); optional `background`. Without `objective`, continues the thread's active goal, as after a restart, or resumes its paused one | streams every turn of the goal as `turn/start` streams one, then answers with `turn`: how its last turn ended, with every turn's calls, tokens and cost added up |
| `goal/stop` | `thread` | `{}` once the goal is recorded as stopped, its running turn cancelled |
| `goal/get` | `thread` | `goal`: the thread's latest goal (`objective`, `token_budget`, `tokens` used, `turns`, `ended`, the reason, once it has, and `note`, what the model needs, while it is paused), or `null` |
| `tasks/list` | optional `limit` (default 20) | `tasks`: running here first, then started elsewhere and not ended, then finished, newest first |
| `tasks/stop` | `thread` | `{}` once the task in it has ended as cancelled (a goal is recorded as stopped). `-32003` when no task runs in it in this server |
| `schedule/create` | `when`: `{"cron": "<five fields>"}` (UTC) or `{"at": "<RFC 3339>"}`; `prompt`; optional `goal` (`true`: each run works toward `prompt` as a goal); `budget` (tokens each run may use, required, more than 0); optional `worktree` | `schedule`: `schedule` (its id), `when`, `prompt`, `goal`, `budget`, `worktree`, `created_at`, `last` (when the latest run was due), `next` (absent once a one-off has run). A cron expression that does not parse is refused with `-32602` |
| `schedule/list` | none | `schedules` |
| `schedule/delete` | `schedule` | `{}`; no more runs start. `-32016` for no such schedule |
| `space/list` | optional `limit` (default 100) | `spaces`: the spaces the caller owns or is a member of, in id order. A space is `id`, `kind` (`project` or `personal`), `name`, `owner`, `members` (principals), `created_at`, `updated_at` (`storage.md`, "Spaces") |
| `space/get` | `space` | `space` |
| `space/create` | `name`; optional `members` (principals) | `space`: a new project space owned by the caller. A name of only spaces, or of more than 200 characters, is refused with `-32602` |
| `space/rename` | `space`; `name` | `space`, renamed. Only its owner renames it; anyone else is refused with `-32012` |
| `memory/list` | optional `scope` (`personal` or `space`) | `memories`: the person's and this space's memories (`memory.md`), each with `memory`, `version`, `scope`, `text`, `saved_at`, optional `edited_at`, `untrusted_read` and `source` |
| `memory/save` | `text`; `scope` | `memories`: the one saved. A credential, more than 500 characters, or a full scope is refused with `-32602` |
| `memory/edit` | `memory`; `text` | `memories`: the one edited |
| `memory/move` | `memory`; `scope` | `memories`: the one saved in the new scope; the old is forgotten |
| `memory/forget` | `memory`, or `all: true` with `scope` | `memories`: those forgotten. Their keys are destroyed at once |
| `memory/restore` | `memory` | `memories`: the one restored, when glave changed it in the last 30 days |

A **thread summary** is the thread's row of the threads view (`events.md`, "Storage"), with what it is doing now: `id`; `space`; `status`: `idle`, `running` or `waiting` (a turn is paused for an approval or an answer); `standing`: `active`, `archived` or `deleted`; `title`, when it has one: the latest `thread.renamed`'s (the person's own, or the one a model wrote after its first exchange, "Titles"), else the first line of what the person wrote in their first message (its last text part: the project's instructions a thread opens with come before it), trimmed and cut to 80 characters (an ellipsis marks the cut); `approval`, the mode its turns run in: its latest `approval.mode.chosen`, else the server's; `for` and `origin`, as `thread.started` records them; `parent`, for a subagent's thread; `created_at` and `updated_at`, when its first and latest events were recorded; `turns`, how many it has started; `outcome`, how the latest ended; `tokens`, what its own model calls used (a subagent's are in its own thread); and `cost`, while every call reported one in one currency. The title is read from the one event that gives it, so the view itself holds no content.

A **task** has `thread`, `kind` (`agent`; background commands come later), `title` (the prompt or objective's first line, shortened), `model`, `tokens` and `cost` so far (subagents' included), `budget`, `started_at`, `elapsed_ms`, `status` (`running` in this server, `elsewhere` when started and not ended by another, `finished`), `outcome` once finished, and `schedule` when one started it.

A **turn result** has `outcome` (`completed`, `cancelled`, `failed`, `interrupted`, `limit_reached`), `model_calls`, `tool_calls`, `tokens` and `cost` as in `events.md`.

Each server serves one organization's spaces, and one folder: its own space's. A thread in its own space works in that folder (or a worktree or worker, as above); a thread in any other space works in a scratch folder of its own (`computers.md`). The server's tasks, schedules and memory are its own space's: a task in a thread of another space is refused with `-32602` for now.

## Models

A server runs one default model and may offer others; a client lets the person choose among them, per thread and per turn. Codex's app server does the same (`model/list`; `model` on `thread/start`, and on `turn/start` "for this turn and subsequent turns"), as does OpenCode (a provider and model list, and a model per message).

- **`model/list`** answers `models`, each with `id` (what `model` takes: the name the gateway knows it by, as `--model` would name it), `name` when one is configured, `default` (exactly one is), and `hope`, true for Hope, glave's own model, so a client can show it as such. The default comes first. The list is the server's own: the default model, the judge's when it is another, and each of `[[models]]` in `config.toml` (`policy.md`), each checked against the policy's `[models]` rules as the default is. It is never paged.
- **Choosing.** `thread/start` or `turn/start` with `model` records `model.chosen` (`events.md`) in the thread, unless that model is already the thread's choice, and the turn, and every later turn, goal turn and subagent of the thread, goes to it. A thread that never chose one uses the server's default, whatever that is when its turn runs. The choice is in the log, so it survives a restart and any server of the deployment honours it.
- **Refusals.** A `model` this server does not run is refused with `-32021` before anything is recorded; so is a turn in a thread whose earlier choice the server no longer offers, which the client answers by choosing again. A gateway elsewhere (`[gateway] url`) may still refuse a model for its caller under its own policy: the turn fails with `-32020`.
- **Which model wrote what.** Every assistant `message.added` is followed in the same batch, before the next message, by the `model.responded` of the answer it came from: its `model` (`provider`, `name`) is the model that wrote it, and `target` the gateway target that served it. Each `turn.started` records the model the turn was sent to. A client labels each reply from these, never from the default.
- A task (`tasks/list`) shows the model its turn was sent to.

## Approval modes

A client lets the person choose, per thread, how much glave asks before acting (`approvals.md`, "Modes"), as Codex's app server takes `approvalPolicy` on `thread/start` and on `turn/start` ("for this turn and subsequent turns"), and Claude Code takes a permission mode when a session starts and changes it between turns.

- **Choosing.** `thread/start` or `turn/start` with `approval` (`ask` or `auto`) records `approval.mode.chosen` (`events.md`) in the thread, unless that mode is already the thread's choice, and the turn, and every later turn, goal turn and subagent of the thread, runs in it: the tool gate, the judge (auto only), what hooks are told (`permission_mode`) and the turn's recorded configuration. A thread that never chose one runs in the server's mode, whatever that is when its turn runs. The choice is in the log, so it survives a restart and any server of the deployment honours it.
- **Within policy.** A thread may choose only the server's `approvals` (`server/discover`): the modes the policy leaves (`[approval] modes`, `policy.md`), and `auto` only where commands run in the sandbox, unless the server runs in auto anyway because someone chose both `sandbox = "off"` and `approval = "auto"` or the policy leaves only auto. Any other is refused with `-32012` before anything is recorded; so is a turn or goal in a thread whose earlier choice the policy no longer leaves, which the client answers by choosing again. A choice never loosens what asks in every mode (`approvals.md`, "Commands read before they run").
- **What the thread shows.** The thread summary's `approval` is the mode its next turn runs in; each `turn.started` configuration records the mode that turn ran in.

## Titles

After a thread's first exchange, the person's first message and the first reply, a model writes the thread a short title, as Claude.ai and ChatGPT title a chat after its first answer and OpenCode's title agent titles a session from its first message.

- **When.** Once the first turn has answered, in a task of its own: the turn's answer never waits for it. Only then: a thread past its first exchange (such as one from before titles existed) keeps the title it has, and a subagent's thread gets none.
- **What the model sees.** Only the first message (what the person wrote, without the project's instructions), cut to 1,200 characters, and the first reply's text, cut to 800, each in a block it cannot close, under a prompt asking for 3 to 6 words in the conversation's language, with no quotes and no punctuation at the end. The answer may be at most 512 tokens (room for models that reason first); its first line is kept, without quotes, a `Title:` prefix or end punctuation, cut to 80 characters.
- **Which model.** The gateway's `title` route (`gateway.md`); with none, as by default, the thread's own model.
- **Recorded.** The call's `model.responded` (route `title`), so its usage and cost count in the thread's, then `thread.renamed` with `by` `{"kind": "glave"}`, in one append straight after the thread's last event. Recording a title is not work on the thread: it never holds the thread, so a `turn/start` at that moment is never refused. The log's `(thread, seq)` check orders the writes; a turn running meanwhile takes the two records in ("Records from outside"), and when another write lands first, the title is checked and appended again after it. No content is logged.
- **Never over the person's.** A thread the person has titled with `thread/rename` is not sent to the model at all, and a rename that lands while the model writes wins: the title is not recorded.
- **Failing quietly.** When the call fails, or the answer holds no title, nothing but its usage is recorded, and the thread keeps the title its first message gives it.
- **Off.** `titles = false` in `config.toml` (`policy.md`): no call is made, and titles come from first messages as before.

## Spaces and who may do what

Every request that acts on a thread (`thread/get`, `thread/events`, `thread/archive`, `thread/unarchive`, `thread/delete`, `thread/compact`, `turn/start`, `turn/cancel`, `approval/decide`, `question/answer`, `goal/*`, `tasks/stop`), on a space (`thread/start` and `thread/list` with a `space`, `space/get`, `space/rename`), or on the server's own space as a whole (`tasks/list`, `schedule/*`, `memory/*`) is checked against the caller before anything is read or recorded: they must be the space's owner or one of its members. Anyone else is refused with `-32012`, whether or not the thread exists; a thread no space here holds is `-32001`. The thread's space comes from the data store's one lookup by bare thread id (`storage.md`, "Scope"), remembered by the server, since a thread's space never changes.

- **Local transports** (in-process, stdio, the Unix socket) act as the local person, who owns the folder's space.
- **Network transports** act as the signed-in person. A thread they start is for them: `thread.started.for` is that person, so `author`, `by` and `origin.by` name someone only when it is not them (`events.md`, "Who acted").
- Members are set when a space is made (`space/create`), or by the host through the spaces port; managing them over the API comes later.

## Archiving, deleting and records from outside

- **Archive and delete are events** (`thread.archived`, `thread.unarchived`, `thread.deleted`), appended through the thread when the server holds it. A thread with a turn running is refused with `-32002`.
- **Records from outside.** Glave's own records about a thread may be appended by another glave process while a server holds the thread: `glave threads archive|unarchive|delete` and worktree cleanup (`glave worktrees prune`, the daily check, archiving) write straight to the log, as the server's own title task does ("Titles"). When the holding server's next append meets the log's `(thread, seq)` conflict, it reads what was written after its last event. If all of it is `workspace.removed`, `thread.archived`, `thread.unarchived`, `thread.deleted`, `thread.renamed` or `model.responded` (records that do not change the conversation: the first five are ones call ids do not count, and a `model.responded` on its own is a title's call, "Titles"), it takes them in and appends after them, so the model is sent the same. A removed worktree is opened again from its branch, as resuming does. If the thread is now archived the turn stops with `-32014`, deleted with `-32001`, and the server reads the thread from the log again next time, which closes what the turn left open. Anything else written from outside, such as a message, means another harness is running the thread, and stays a conflict.
- **Worktrees.** `glave threads archive|delete` removes the thread's worktree at once when that loses nothing (no uncommitted changes, every commit on a remote) and records `workspace.removed` with reason `archived`; its branch stays. A worktree that would lose work is kept and marked held, and the command says why. The session methods record the change only: worktree cleanup works on the computer's files, which the session crate reaches only through the computer port, and that port has no cleanup operations yet.

## Notifications

Two kinds, sent while a turn runs or a client watches:

| Method | Params | Kept |
| --- | --- | --- |
| `thread/event` | `thread`; `event`, a stored event with its `seq` | Yes: it is in the log, and `thread/events` returns it again |
| `turn/live` | `thread`; `update`: `text` or `reasoning` (a fragment), `tool_started`, `tool_finished`, `waiting`, `retrying`, `compacted`, `asked`, `subagent_started` (`subagent`, `description`), `subagent_ended` (`subagent`, `ok`), `memory` (`change`: `remembered`, `edited` or `forgot`; `memory`; `text`) | No: live updates are for showing progress, never replayed |
| `task/finished` | `thread`; `task`, as `tasks/list` shows it | In the ledger as `task.ended` |

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

## Goals

A goal keeps a thread working across turns until the model ends it, the person stops it, or its budget runs out. The person says in the objective how to verify it, if they want to ("... until `cargo test` passes"); glave runs no check of its own. Its first turn's prompt is the objective; each turn after it starts with a continuation glave writes (the objective again, the budget left, and a reminder to verify before completing), recorded with `author` glave. In every turn of an active goal, and in no other turn, the model is offered `update_goal`: `status` `complete` once it has verified the goal is met, with its own tools (commands run in the sandbox and ask as any would), or `paused` with a `reason` when it cannot go on without the person (a decision that is theirs, credentials, information only they have). A turn that reaches the budget gets a last step in text only. `goal.set`, `goal.ended` and `goal.resumed` (`events.md`) record it, so `goal/get` reads progress from the log while it runs, and a goal survives a restart: `goal/start` without an `objective` continues it. The same call resumes a paused goal, typically after the person has answered its `note` in a plain turn.

- A goal's turns are turns: `turn/cancel` cancels the running one, which stops the goal; a second `turn/start` or `goal/start` is refused with `-32002`.
- `goal/stop` on a thread with no active goal, or `goal/start` without `objective` on a thread with no active or paused goal, is refused with `-32007`.

## Tasks

A task is a thread working with no one attached: a turn, or a goal's turns, that the server runs after the client that asked has gone. The person can close their laptop when the server is not on it (Glave Cloud, a company's servers), or when its threads work on workers.

- **Starting one.** `turn/start` or `goal/start` with `background: true` starts the work as either would, and answers at once with the task. Everything is recorded as usual: `thread/watch` and `thread/events` follow it or read it afterwards, from any client.
- **Nobody is asked.** In a task, and in its subagents, approvals are answered as with no one at the keyboard (`approvals.md`): in auto mode the judge still lets through what the person asked for, and anything else that would ask is declined (`no_one_there`); `question` is not offered. A task never waits for an answer. Otherwise the model is sent exactly what it is sent in a turn someone follows.
- **Where it runs.** A thread started with `background: true` works on a worker when the server has a worker backend (`[workers]` in `config.toml`, within policy), else where the server puts any thread.
- **Budget.** A turn may carry `budget`: once its model calls (subagents' included) have used that many tokens, in and out, the turn is cancelled and the task ends as `limit_reached`; it may overshoot by the call that crossed it. A goal keeps to its own budget. A `budget` on a turn that is not a task is refused (`-32602`) rather than ignored.
- **Tokens and cost so far** come from the task's `model.responded` events as they are recorded; when it ends, from the turn's own totals.
- **Ending.** `tasks/stop` stops it; archiving or deleting its thread stops it first. When a task ends, its thread's watchers get `task/finished` (`thread`, `task`).
- **The ledger.** Each server records `task.started` and `task.ended` in the space's task ledger (`events.md`), so any server or client lists tasks, finished ones included, after a restart. A server that runs a space's tasks calls `recover_tasks` when it starts: tasks the ledger has open that no one runs any more are recorded as `interrupted`.
- **Policy.** `[tasks] max_running` caps the tasks one person has running in a server; one more is refused with `-32015`, naming the limit.

## Schedules

A schedule starts a task at set times: once (`at`), or whenever a five-field cron expression matches (`minute hour day month weekday`, in UTC; `*`, numbers, `a-b`, steps and lists; when both day and weekday are restricted, either matches, as in cron). Each run is a new thread (placed as `thread/start` with `background` places it) whose first prompt is `prompt`, or whose goal is `prompt` with `goal`, with the schedule's `budget`. Schedules live in the space's task ledger (`schedule.created`, `schedule.deleted`), so every server of the space sees the same ones.

- **The scheduler.** The server that runs a space's tasks fires due schedules (`Server::tick`, which `Server::run_scheduler` calls when the next run is due and at least every minute, so it finds schedules other processes made). Time comes from the server's injected clock.
- **Missed runs** while no server ran the schedule run once, as soon as one does, never once per missed time: systemd's `Persistent=true`, and Claude Code's desktop scheduled tasks. A run the task limit holds back starts on a later tick, once.
- **Budgets are required:** a schedule without a positive `budget` is refused.
- **Policy.** `[tasks] max_schedules` caps the schedules that will still run; one more is refused with `-32015`.
- One server per space fires schedules; two would each start the run. Leases (issue #64) make that safe across servers.

## Turns

- One turn runs per thread at a time. A second `turn/start` is refused with `-32002`. Queueing and steering a running turn come later.
- **Custom commands.** A `prompt` that starts with `/name` where `name` is one of the server's commands (Markdown files in `.glave/commands` or `.claude/commands` in the project, then the person's own) is sent as that command's text, with `$ARGUMENTS` replaced by the rest of the prompt and `$1` to `$9` by each word of it; the stored `message.added` holds the text sent. Any other prompt, `/usr/bin is slow` included, is sent as written. Listing the commands over the API comes later; `glave commands` lists them.
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
| `-32007` | The thread has no active goal (or, to resume, no paused one) |
| `-32008` | A `UserPromptSubmit` hook stopped the prompt; the message gives its reason, and the model never saw the prompt (`approvals.md`, "Hooks") |
| `-32009` | The thread's workspace could not be opened, such as a worktree outside a repository |
| `-32010` | Unsupported protocol version; `data.supported` lists the spoken ones |
| `-32011` | The thread is running in another server of this deployment |
| `-32012` | The caller may not do this, such as choose an approval mode the policy does not leave |
| `-32013` | No valid sign-in (network transports) |
| `-32014` | The thread is archived; unarchive it first |
| `-32015` | A policy limit was reached: `[tasks] max_running` or `max_schedules` |
| `-32016` | No such schedule |
| `-32017` | No such memory here |
| `-32018` | Memory is off for this server |
| `-32019` | No such space |
| `-32020` | The model provider failed; `data` has `failure` (its kind), `message` and, when the person can fix it, `fix` |
| `-32021` | No such model here, or not one this server may run |
| `-32022` | Not available yet, such as a thread on a computer of the person's own |

## Transports

| Transport | Framing | Who uses it | Authentication |
| --- | --- | --- | --- |
| In-process | Typed messages over a channel; nothing is serialized | The CLI; programs that link glave's crates | None: same process |
| stdio | One JSON message per line (`glave serve --stdio`) | Programs that run glave as a child process | None: the parent started it |
| Unix socket | One JSON message per line (`glave serve --socket`, the folder's task host) | Local clients of a long-running glave | File permissions on the socket (`0600`, in the person's private GLAVE_HOME) |
| HTTP + SSE | `POST` one request; the answer is JSON, or for streaming methods an `text/event-stream` of notifications ending with the response. The method is also sent as the `Glave-Method` header, so proxies and logs can see it | Web, desktop and remote clients | Required: the local token from this computer, or OIDC |
| WebSocket | One JSON message per text frame | Clients that want one connection for many threads; later, realtime features | Required |

A network listener never runs without authentication: there is no "add it later". Closing a streaming HTTP response stops only that stream; the turn goes on, and `thread/watch` picks it up again.

## Over the network

What changes when a client is on another machine: who is calling, how a server that may restart keeps a paused turn, and how several servers share threads.

### Signing in

- **Local transports need no sign-in:** in-process, stdio and a Unix socket act as the local person.
- **A browser on the person's own computer signs in with the local token.** `glave serve --http` makes a new random token (32 bytes) every time it starts, writes it to `GLAVE_HOME/serve.token`, readable only by the person, and deletes it when it stops. It accepts the token (`Authorization: Bearer glave_local_…`) only from a loopback address (`127.0.0.0/8`, `::1`), as the local person, recorded as `auth.method` `local`; from anywhere else it is refused with `-32013`. The web UI it serves gets the token once, from a link `glave serve` prints to the terminal, `http://127.0.0.1:<port>/#token=<token>`: in the fragment, which browsers never send, so it never reaches a server log or a query string. Without `--issuer`, the local token is the only sign-in, and the listener must be on this computer.
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
- Who may do what (start a turn, answer an approval, read a thread) is checked against the caller on every request, from the space's owner and members ("Spaces and who may do what"), and later the organization's policy. A request from someone who may not make it is refused with error `-32012` and never recorded; a network request without a valid sign-in is refused with `-32013`.

## Status

Version 1, in-process (`glave-session`, used by the CLI), over stdio (`glave serve --stdio`, the same server through `rpc::serve`), over a Unix socket (`glave serve --socket`, through `rpc::serve_connection`, where a client that leaves stops nothing) and over HTTP with server-sent events (`glave serve --http <address> [--issuer <url> --audience <name>]`, requests `POST`ed to `/`); one conformance suite runs over all three. Every method but `thread/watch`, whose in-process form is `Server::watch`.

- **The schema:** `specs/session.schema.json` is a JSON Schema (draft 2020-12) of every method's params and result, every notification and every error code, generated from the Rust protocol types (`glave_session::schema`, deriving `schemars` beside serde). `cargo xtask schema` writes it, and the TypeScript types of the web client (`web/client/src/generated/api.ts`) from it; `cargo xtask ci` fails when either no longer matches the code. `web/client` is a small typed client (no framework): `call` for any method, `stream` for a turn's server-sent events, errors as `ApiError` with their code. CI type-checks it with TypeScript, strict.
- **Models:** built. `model/list`, and `model` on `thread/start` and `turn/start`, recorded as `model.chosen`. Not yet: the provider endpoint's own model list (its `/models`) in `model/list`, a `model` on `goal/start` and `schedule/create`, and the thread's model in its summary (read it from its latest `model.chosen`, else the default).
- **Approval modes:** built. `approval` on `thread/start` and `turn/start`, recorded as `approval.mode.chosen`; `approvals` in `server/discover`; `approval` in the thread summary. Not yet: `approval` on `goal/start` and `schedule/create`, and changing the mode while a turn runs.
- **Titles:** built, for turns (`turn/start`, in the foreground or as a task). Not yet: a goal's first exchange.
- **The threads view** (issue #64): built in both stores (`storage.md`); `thread/list` and the summaries read it, and `thread/rename` titles a thread.
- **Local sign-in:** built, for the web UI that `glave serve` serves.
- **The web app:** `glave serve --http` serves the built app (`web/`) at `/` beside the API: sign-in from the link's fragment (kept in the tab's session storage, taken out of the address bar), the spaces and their threads, a thread with its turn streaming live (approvals and questions answered in place, each reply named by its `model.responded`, a model picker sending `model` on the next turn, the server's approval mode shown read-only), a tools panel (tasks, and the files the thread changed, read from its `write` and `edit` calls), tasks, memory and a read-only settings page. Without `thread/watch` over HTTP, a thread opened while another client runs its turn shows new events after the person acts, or on reopening; the side nav reads the threads again every ten seconds.
- **Serving:** every `GET` is one of its files, or its `index.html` for the app's routes, from `--web <folder>` or else the `web` folder beside the binary; `POST /` stays the API. Only files inside the folder are served (symbolic links are followed only to files inside it), and no HTML page but `index.html`. The folder is served without sign-in to anyone who reaches the listener, so it must hold only the build and must not be writable by an agent. Every answer carries a strict Content Security Policy (no inline script; Trusted Types; inline styles only with a nonce of 16 random bytes made for that load and put in `index.html`, and no page at all if the operating system gives no randomness), `Cross-Origin-Opener-Policy: same-origin`, `nosniff`, `no-referrer` and `DENY` framing. Hashed assets are cached for good; `index.html` never. Pages need no sign-in; the app signs in to the API with the local token from the link's fragment.
- **The CLI as a client of a server elsewhere** (issue #171): built. `glave --server <url>`, or `[server] url` in the person's `config.toml`, makes the terminal a client over HTTP (`glave_session::client`, the Rust counterpart of the web client): threads, turns, goals, approvals and questions go to the server and are followed exactly as in-process. Sign-in is a bearer token from `GLAVE_SERVER_TOKEN` (an OIDC access token for the server's audience, or the server's local token on its own computer); the server must be `https`, or `http` on the same computer. Flags that choose how glave runs here (`--model`, `--approval`, `--sandbox`, `--network`, `--on`, `--max-steps`, `--background`) are refused with `--server`. With no one at the terminal, an approval the server asks is declined. Not yet: sign-in from the terminal (OIDC device flow), and `--background` tasks on the server.

- **Sign-in on HTTP:** OIDC access tokens signed with RS256 or ES256, checked against the keys the issuer's discovery document points to, with `iss`, `aud`, `exp` and `nbf` verified and a minute of clock skew forgiven. Keys are cached and fetched again for a key not seen yet, at most every 30 seconds. An issuer must be `https`, or `http` on the same computer. The person is derived from the issuer and subject (a UUID v7 at time zero whose random bits are the start of a SHA-256 of both) until an identity source maps them. The caller is stamped as "Who is calling" describes. The listener has no TLS of its own: across machines it sits behind a proxy that has.
- **Retries:** `turn/start` honours `_meta.idempotency_key` for the 1,024 most recent keys, in memory.
- Turns pause for approvals in memory: a server that stops while a turn waits leaves it to be resumed as interrupted.
- **Archive, delete, cleanup:** built (issue #131). **Deleting does not erase yet:** today's keys are per space and month, so one thread's content cannot be destroyed alone (`storage.md`). `thread/delete` hides the thread for good and lists it (`thread/list` with `show: deleted`) for the erasure that arrives with data keys per thread. Archiving or deleting through the session API does not remove the thread's worktree yet (above); `glave threads` does, and the daily check removes it once the thread has been idle 30 days.
- **Tasks and schedules:** built (issue #44). The CLI's task host is `glave serve --socket`, one per folder, which `glave --background`, `glave tasks` and `glave schedule` start when it is not running and which exits once no task runs, no schedule waits and no client is connected; a session in the folder starts it when a schedule waits, and says which tasks finished since the last one. It keeps the settings it started with (model, config, AGENTS.md) until it exits. Not yet: background commands (the model starting a long-running shell process), a task's worker released when the task ends (it is released when the host exits), schedules in the person's time zone, the `origin` of a scheduled thread recorded as `schedule`, desktop notifications, running the scheduler in `glave serve --http`, and the operating system starting the host at login (launchd, systemd).
- **Spaces and authorization** (issue #64): built. The server serves many spaces, `space/*` and the `space` of `thread/start` and `thread/list`; a network caller's threads are for them; `-32012` is enforced on every request above, over every transport, from the space's owner and members. Not yet: promoting a one-off chat into a project (`storage.md`), managing members over the API, tasks and schedules in spaces other than the server's own, and the organization's policy in the check.
- Still to come from "Over the network" (issue #64): SAML and glave session tokens, durable paused turns, leases, WebSocket, and enforcing `-32011`.

## Later

- WebSocket-only realtime channels, such as voice (issue #58), as optional features.
- Steering and queueing turns.
- ACP and A2A adapters, when a real client needs them.
