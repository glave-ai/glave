# One server per person

How a person's glave runs as one server on their own computer: how it starts and stops, how every client finds it, what each project brings to its own threads, and what survives a restart. The server holds every project, thread, task and schedule; the web app, a phone, the CLI's commands and later a terminal UI are its clients. It is step 2 of #294 and the rest of #64, and it settles items 1 to 3 of `model.md`'s "Where the code disagrees", and the part of item 9 that concerns tasks.

## Status

Designed, not built. "The pull requests" gives the order. Each pull request moves the sections of `session.md`, `policy.md`, `computers.md`, `storage.md` and `events.md` that it changes, and AGENTS.md where a rule or a budget changes; this spec keeps the decisions that span them.

## How the others do it

- **Codex's app-server** ([docs](https://learn.chatgpt.com/docs/app-server), [remote connections](https://learn.chatgpt.com/docs/remote-connections)) is started by its client: the docs' example spawns `codex app-server` over stdio, and the desktop app hosts what a phone reaches, so remote access ends when that computer sleeps, goes offline or quits the app. `--listen unix://` serves a default control socket and `codex --remote` attaches the terminal UI to a running server. An approval is a request from the server to a connection, scoped by thread and turn; a pending request is cleared when its turn is interrupted. A thread with no subscriber stays loaded until it has had no activity for 30 minutes. Non-loopback WebSocket listeners accept unauthenticated connections by default for now.
- **OpenCode** ([server](https://opencode.ai/docs/server/), [web](https://opencode.ai/docs/web/), [CLI](https://opencode.ai/docs/cli/)): `opencode` starts a terminal UI and a server in one process, on a random port; `opencode serve` runs a headless server on 127.0.0.1:4096; `opencode web` opens the browser on a random port; `opencode attach` and `run --attach` reuse a running server so MCP servers don't start cold on every run. One server serves many folders: each request names its folder (`x-opencode-directory`, the server's own folder by default), and each folder's config, plugins and MCP servers are built on first use and kept with no idle limit (`idleTimeToLive: Duration.infinity` in [`location-services.ts`](https://github.com/anomalyco/opencode/blob/dev/packages/core/src/location-services.ts)). Without `OPENCODE_SERVER_PASSWORD` it asks no one to sign in. Permissions are answered with a `POST`; events stream over server-sent events.
- **Claude Code** ([agent view](https://code.claude.com/docs/en/agent-view), [Remote Control](https://code.claude.com/docs/en/remote-control), [desktop scheduled tasks](https://code.claude.com/docs/en/desktop-scheduled-tasks)): a supervisor, the "background service", runs background sessions; Claude Code starts it the first time a session is sent to the background or agent view opens, one per config folder (`CLAUDE_CONFIG_DIR` makes a separate instance), managed with `claude daemon status` and `stop`. Sessions are saved on disk and survive supervisor restarts and updates; an idle session's process is stopped after about an hour and resumes on reply. After a shutdown, a session waiting on the person stays under Needs input, and one that was working shows failed and continues only when the person replies. Remote Control keeps the local process running, uses outbound HTTPS only, and keeps permission prompts open until answered. Desktop scheduled tasks run only while the app runs and the computer is awake, with one catch-up run for the latest missed time.

What glave takes: Claude Code's start on first use, one server per glave folder, sessions that stay in Needs you after a restart, and interrupted work that goes on only when the person says; OpenCode's one server for every folder, each folder's tools built on first use, and a fixed default port; Codex's clients that are all clients of one protocol, and threads unloaded after 30 idle minutes. Where glave differs: a turn paused for an approval or a question is answered where it stopped after a restart, which none of them documents; a project's tools are bounded and closed when idle; and a listener never runs without sign-in.

## The person's server

### One per glave folder

- **One server per glave folder** (`GLAVE_HOME`, `~/.glave` by default). It opens the data store, holds the keys, runs the harness and the gateway, and is the store's one writer: every command asks it, except worktree cleanup, which records straight to the log until it moves into the server ("The CLI as a client"). A second glave folder (tests, a second profile) is a second person's server.
- **It has no folder of its own.** Nothing depends on where it started: no start folder's project, settings, tools or instructions. Today `wire` reads all of them from the current folder.
- **An organization runs the same program** (`glave serve` in a unit or a container), its people reaching it over HTTP; replicas are under "Durable turns".

### Starting

- **On first use.** A command that needs the server and finds none starts it: `glave serve` in the background, in a session of its own so neither Ctrl-C nor closing the terminal reaches it, its output in `run/server.log`. The command waits until the socket answers, at most 20 seconds as the task host's starter does today, then goes on. Claude Code starts its supervisor the same way.
- **In the foreground**, `glave serve` is the same server for a systemd unit, a launchd agent, a container, a test, or an organization (`--http <address>` with `--issuer` and `--audience`). Its flags are its choices, as today.
- **At login: offered, never installed unasked** (owner decision 1). `glave server --at-login on|off` writes or removes a launchd agent on macOS or a systemd user unit on Linux that runs `glave serve`; the app asks once, when the person makes their first schedule ("Schedules run only while glave runs. Start glave when you log in?"). Only schedules need it, macOS tells the person a background item was added, and neither Codex, OpenCode nor Claude Code installs one. A server the login item starts has no shell's variables ("Its settings"), and on macOS it asks for folders such as Documents in its own name rather than the terminal's.
- **Fast first.** It listens as soon as the store is open; the person's MCP servers and the daily work start after.
- Considered: socket activation by launchd or systemd, rejected because a schedule needs a running process at its time and it needs the login item anyway; a server per client, as Codex's stdio app-server, rejected because tasks, schedules and waits must outlive every client.

### Finding it

- **The socket** is `GLAVE_HOME/run/server.sock`, `0600` in a `0700` folder; where that path is too long for a socket, `/tmp/glave-<uid>/<hash of the glave folder>.sock` in a `0700` folder, as the task host's today. A client checks the folder's owner and mode and the listener's user before it sends anything (`daemon::Host::connect` today). A local transport acts as the local person, with no token.
- **One copy.** The server holds an exclusive lock on `run/server.lock` for its whole life (`File::try_lock` in the standard library), taken before it listens. The operating system lets go of it when the process ends, however it ends, so a crash leaves no stale lock; only the holder removes a socket file a crash left. A second `glave serve` exits saying where the first one is. Two commands racing to start one each spawn a server; one gets the lock, the other exits, and both commands connect to the winner.
- **The browser.** The person's server always listens on loopback HTTP, at 127.0.0.1:4180, or any free port when that is taken (owner decision 2), written in `serve.url` as today; `glave serve --http` listens where it is told instead. The same port across restarts keeps the same origin, which keeps a browser's stored state, an installed app and push subscriptions (#333).
- **A link carries a one-time code, never a token.** `glave` asks the server over the socket for a link; its fragment carries a code good once, for 5 minutes, which the app trades for a browser session. Today's link carries the local token itself, tolerable while a server lived as long as one terminal; a person's server lives for days, and a link printed into scrollback, a terminal log or a shared screen must not stay a key.
- **A browser's session outlives a restart.** The server signs it with a key kept in glave's folder; it is accepted only from loopback, as the local token is, lasts 30 days from its last use (owner decision 6), and "Sign out other browsers" replaces the key. An upgrade or a crash signs no tab out, and a notification opens the app signed in. #306's passkey sign-in from elsewhere will issue the same sessions.
- **The local token** (`serve.token`) stays for programs on this computer (`GLAVE_SERVER_TOKEN`), made at each start as today.
- **Loopback is a second line, never the only one.** A tunnel or proxy on this computer makes every request it forwards come from loopback, so each request still needs its token or session, and #306 decides how a request through one is told apart from the person at this computer.
- **Out of the agent's reach.** Sandboxed commands reach no Unix socket (seccomp on Linux; on macOS the profile allows only `remote ip` and mDNSResponder's socket), and glave's folder, with the token and the session key, is hidden (`sandbox.md`). Loopback HTTP is reachable with the network on, but nothing there answers without a sign-in. A command run outside the sandbox reaches the server as the person can, which is why running outside the sandbox asks every time (`approvals.md`). `glave` run inside glave's sandbox cannot reach its server, and says so.

### When it isn't running

- A command starts it. When it cannot, the command says why in one sentence and where the log is.
- A browser tab says glave isn't running on this computer and to open glave again; a browser cannot start a process. A phone says it cannot reach the person's computer, and pushes stop until it is back. Those words are each client's own.
- Nothing is lost: history, waiting approvals and questions ("Durable turns"), tasks' records and schedules are in the store. Schedules missed meanwhile run once, as soon as the server runs again (`session.md`, "Schedules"); tasks that were running are recorded as interrupted.

### Stopping

- **It runs until** the person logs out, the computer shuts down, or `glave server stop`. There is no idle exit (owner decision 3): schedules need it, and a browser link must keep working. It lets go of what it does not use instead ("Bounds"). Today's task host exits when idle because there is one per folder.
- **A graceful stop** (`SIGTERM`, `SIGINT`, `glave server stop`, logging out) takes no new requests, gives running turns up to 10 seconds to reach a wait or end, then cancels the rest and records each `interrupted`, not `cancelled`, since the person did not stop them. Waiting turns stay waiting. Running tasks are recorded `interrupted`, MCP servers are closed, workers released, the socket removed and the lock let go.
- **A crash** leaves the store as any crash does, and the next start repairs it ("Durable turns"). Commands a crashed server started end with it on Linux (bubblewrap's `--die-with-parent`); on macOS they may run to their end.
- **`glave server status`** says whether it runs, its version, its address, and what is running and waiting; **`glave server restart`** stops it and starts it from this shell.
- **Upgrades.** A command compares its version with the server's (`server/discover`). When they differ but share a protocol version, it works and says in one line that `glave server restart` moves to the new one, which is safe whenever no turn is running, since waits survive it. It never restarts the server itself: a build run from a checkout would replace the person's server with itself. The login item starts the installed binary at the next login.

### Its settings

- **Read again, never frozen.** Today each run reads `config.toml`, `mcp.json`, the person's AGENTS.md and the policy afresh. A server that lives for days reads each again at the start of a turn when it changed, by modification time and size, as it reads a project's trust today; a choice applies from the next turn. What only a start applies (the store's address, the key root, identity providers, telemetry, the gateway's address, the workers' backend) the server reports as waiting for a restart, and clients show it.
- **A client's environment never reaches the server.** Sending it would send a shell's secrets; reading it from whichever shell happened to start the server would surprise every other one (Claude Code documents the same trap for its supervisor). The server reads its own environment once, when it starts: `GLAVE_HOME`, `GLAVE_STORE_URL`, the model variables (`GLAVE_BASE_URL`, `GLAVE_MODEL`, `GLAVE_API_KEY`) until settings are data (#222), and the `${VAR}`s the person's `mcp.json` passes to their servers until secrets are kept in the store (#223). A command whose shell names a model the server does not run says so once; `glave server restart` starts the server from that shell. Model accounts connected in the app (`session.md`, "Model accounts") need neither.
- **What a terminal printed at start, the server now says.** That commands run without the sandbox here and why, that connected accounts were set aside, that tasks were interrupted: `server/discover` and the summaries carry them as facts with a sentence, and every client shows them (AGENTS.md, "The server says what happened"), since no one reads the server's log.

### What runs in it on its own

The scheduler; task recovery when it starts; the daily worktree check (today at each run's start, `computers.md`); memory's tidy-up and key rotation, daily (today when a session ends, which a long-lived server never reaches, `memory.md`); renewing git hosts' tokens (as today).

### Bounds

- **Threads in memory**: a thread is unloaded after 30 minutes with no turn running, as Codex unloads one, and at most 256 are held, the least recently used let go first. Any thread is rebuilt from the log on demand (`session.md`, "Stateless hosting is real"); a waiting turn holds nothing in memory ("Durable turns").
- **Projects' tool servers**, **tasks** and **watches**: in their sections below.
- **Connections**: 1,024 at once over HTTP, as today, and the same on the socket.

### What a person sees

Nothing about a server. `glave` opens the browser and `glave "…"` answers in the terminal; the first command after a login takes a moment longer while the server starts. A tab whose server stopped says to open glave again. Settings says glave keeps working in the background so tasks and schedules run, beside the choice to start it at login. No port, socket, process or file is named in the app (AGENTS.md, "Plain words").

## The CLI as a client

Every command is a client of the person's server over its socket, or of a server elsewhere with `--server` or `[server] url` as today, in which case none is started here; it uses nothing the API does not offer (`session.md`, "Principles"). No command but `glave worktrees` opens the data store.

| Command | What it asks the server |
| --- | --- |
| `glave` | the project this folder leads to, asking once when there is none (below); then a sign-in link to it, opened in the browser, or printed where there is no screen |
| `glave "…"` | the project, as above; `thread/start` in it and `turn/start`, followed; approvals, questions and the project's trust ask answered in place with `approval/decide`, `question/answer` and `trust/answer` |
| `glave -c "…"` | the project's newest thread (`thread/list`), then `turn/start`; `glave -c` alone opens that thread in the browser |
| `glave --goal …` | `thread/start`, then `goal/start`, followed |
| `glave -b "…"` | `thread/start` and `turn/start` (or `goal/start`) with `background`; prints the task and exits |
| `glave tasks [stop <id>]` | `tasks/list` across every space the person works in; `tasks/stop` |
| `glave schedule add\|list\|rm` | `schedule/create` in this folder's project (in Chats where the folder is none), `schedule/list`, `schedule/delete` |
| `glave threads …` | `thread/list` in the project, `thread/archive`, `thread/unarchive`, `thread/delete` (today it writes the log itself) |
| `glave memory …` | `memory/*` (today it opens the store itself) |
| `glave connect`, `glave connections` | `credential/*` (today through the folder's task host) |
| `glave serve --stdio` | relays standard input and output to the socket when a server runs; otherwise it is the server, in the foreground, until its input ends |
| `glave server status\|stop\|restart\|--at-login` | the process itself: its lock, its socket, a signal, the login item |

- **Local still**: `glave policy explain`, `glave skills` and `glave commands` read this folder's and the person's files, as the server reads them, and change nothing. `glave worktrees` works on this machine's worktrees as today; its records reach a thread the server holds through "Records from outside" (`session.md`), and moving cleanup into the server comes later. `glave computer serve` and `glave gateway serve` are other roles.
- **A program's own server.** `glave serve --stdio` started where no server runs is the person's server until its program ends, as today, so the tasks it runs are interrupted then; programs that want work to outlive them start the person's server first, as any command does.
- **Following.** The socket carries a turn's notifications as HTTP's stream does, so the CLI follows the person's server with the code that follows a server elsewhere today (`glave_session::client`): one path, here or there.
- **Ctrl-C stops the turn** (`turn/cancel`), as today. **Closing the terminal does not** (owner decision 4): the turn goes on in the server, its approvals wait for any client, and `glave -c` picks the thread up. Today the turn dies with the process; Claude Code's background sessions go on with no terminal attached.
- **No one at the terminal** (a pipe, a script, CI): the CLI starts its turn answered by no one, a new choice on `turn/start` and `goal/start`. Anything that would ask is declined at once and `question` is not offered, as today and as for a task, so a script never hangs (owner decision 5). Whether anyone answers becomes a property of a turn, not of the server (`Answering` today).
- **The flags.** `--model` and `--approval` go to the server as the thread's choice, as `--server` sends them today; a model must be one the server offers, so one nothing sets up is refused, naming those it does. `--worktree` is `thread/start`'s. `--sandbox`, `--network`, `--no-network`, `--on` and `--max-steps` choose how the server runs, so they leave the turn commands: they are settings, and the foreground `glave serve` still takes them. `--events` exports the thread as today; `--log` records the client's own spans.
- **The terminal chat goes** (decided), and `/compact`, `/memory` and the banner with it: `glave` alone opens the browser, where both live (`glave memory` stays). **`glave app` goes**: `glave` does what it did. A terminal UI (#308) comes later as another client of the socket.

### A folder's project

- **Found by the server**: the project whose folder is this folder; else, inside a git repository, the project of the repository's top folder or of its `origin` (`storage.md`, "Spaces"); else none. A subfolder of a repository opens its project and never moves the project's folder; a folder that is its own repository is never taken for a project above it.
- **A new folder asks once** (decided), in the terminal: "Make ~/code/app a glave project?" Yes makes it, as the app's New project does (`space/create` with the folder); No runs nothing and keeps nothing, so it asks again next time, as trust's "Not now" does. The lookup that asks first makes nothing; it is new. With no one at the terminal the folder becomes a project without asking, as today: a script that runs glave in a folder has chosen it, and nothing a project brings runs until it is trusted.
- **Never a project**: the home folder, the folders above it, and the temporary folder. `glave` there opens Chats, and `glave "…"` starts a chat in a scratch folder of its own (`computers.md`).
- **No server's own space.** A request that names no space acts in the caller's Chats; today it acts in the space of the folder the server started in.

## Each project's own tools, hooks and settings

Every project's trusted servers and hooks already reach only its threads (`policy.md`, "Project trust"), except the folder the server started in, whose servers and settings reach every thread. That exception goes.

- **The person's own** (servers in their `mcp.json`, hooks in their `config.toml`, their AGENTS.md) are for their threads everywhere: in every project and in Chats. Their servers start once, in the home folder, after the server starts: most are services behind an API, and one process each keeps a laptop light. A server that needs a project's folder belongs in that project's `.mcp.json`. Considered: one per project, in its folder, as each Claude Code session starts its own; rejected for the number of processes, to revisit if a person's server turns out to need the folder.
- **A project's own** (servers in its `.mcp.json`, hooks and settings in its `.glave/config.toml`) are for its threads alone, once trusted. `Started`, which made the start folder's servers and settings the whole server's, goes, and with it the rule that a project's server gives way to a server-wide one of the same name: now only the person's own can share a name, theirs wins as today, and the model is told the project's is off (#305).
- **Lifetimes.** A project's servers start in its folder the first time one of its turns needs tools, and stay up for its later turns, as OpenCode keeps a folder's. New: they close after 30 minutes no turn of the project used them; as today, at most 16 projects' run at once, the least recently used closed first, and a set a running turn still holds closes when the turn lets go. One that fails to start leaves the turn without it, and the model and the person are told, as today.
- **A project's settings** apply to its threads alone, read at the start of each turn as its trust is: what glave run in that folder applies today. Its model, `network = false`, a tighter approval or sandbox, `worktree`, `titles` and `space_memory` apply at once; looser approval and sandbox, `judge_model` and a wider `[web]` once trusted. The nearest choice wins: the thread's own, the project's, the person's, glave's (`policy.md`); constraints only narrow. A project's model must be one the server offers; one it does not is set aside with a sentence, and its threads use the default. What only the person's or the server's file sets (store, gateway, provider, GitHub, telemetry, server) a project never does, as today. Settings move into the store with #222; the order stays.
- **Anyone signed in from elsewhere** gets none of a project's, trusted or not, as today, and none of the person's own servers and hooks, which run with that person's sign-ins. Today the server's own tools reach every thread.
- **Threads on workers** run the person's own hooks alone, as today (#305 decides the rest).

## Durable turns

### Every wait is in the log

- **A turn waits** at its first unfinished tool call when that call is an approval asked and not decided (`approval.requested`, no `approval.decided`), a `question` call with no result, or an `agent` call whose child's turn waits; and, with "One writer per folder", before its first step, for the folder. Nothing else needs keeping: the log already says it.
- **The threads view** keeps, content-free, whether a turn is open and, when it waits, the wait's kind and `seq`, folded from each append as the rest of the view is (`events.md`, "Storage"): set when the turn's first unfinished call becomes a wait, cleared by the turn's next event. A parent waiting on its child is found through the child, whose view names its parent. The fold's version goes up, so stores rebuild views when they open. So a list, Needs you and a summary's `waiting` are right after a restart without opening the thread; today `waiting` comes from the server's memory.
- **A waiting turn is a running turn** for everything else: a second `turn/start` is refused (`-32002`), and so are archiving and deleting its thread, until it ends or is stopped.
- **No wait expires.** The person ends one with Stop, which denies it, as today.

### After a restart

- **When it starts**, the server repairs every turn the view shows open and not at a wait, its own or a child's: its dangling calls get the interrupted result and the turn ends `interrupted`, as loading a thread does today, so nothing shows running that is not. A turn at a wait is left as it is, and loading its thread no longer closes it (`resume_thread` closes every open turn today).
- **A waiting thread holds nothing in memory**: it is rebuilt from the log when its answer comes.

### An answer continues the turn

- **`approval/decide` and `question/answer`** for a wait no running turn holds: the server checks it against the log (refused as today, `-32004` or `-32006`, when nothing waits), records the answer exactly as the running turn would (`approval.decided` with `by`, or the question's result), and continues the turn from the log. The first answer wins; another for the same wait is refused as not pending. The answered call runs or is declined, the rest of that response's calls run in order, and the loop goes on to the model. It is one code path: the turn's own loop, entered at "the rest of this response's calls". `PreToolUse` hooks that ran before the ask do not run again; `hook.ran` recorded them. A goal goes on to its next turns as it would have.
- **What a running turn kept only in memory** is rebuilt or let go: its tallies from the turn's `model.responded` events; its model, mode and caller from `turn.started` and the thread's choices; the count that refuses a call repeated many times starts again. A task never waits, since no one answers it, so no budget is held across a wait.
- **The client that answered** follows the continued turn as one it started, and every watcher sees it.
- **Subagents.** A parent waits while its child waits. Answering the child continues the child's turn; when that ends, its report becomes the parent's `agent` result and the parent goes on. Children that ran side by side and had finished give their reports from their own logs, and a finished child never runs again.

### A turn that dies mid-call

- **Interrupted, never redone on its own.** When the server dies during a model call or a tool call, not at a wait, the next start ends the turn `interrupted`, its open calls closed with the interrupted result, as today. A call that may have half run (a push, a migration, a payment) must not run twice without the person. The thread says so; the person goes on with a message, or `goal/start` for a goal. A task is recorded `interrupted` and does not run again; a schedule runs at its next time. Claude Code does the same after a shutdown.
- Considered: continuing such turns from the log on their own, the model told which call was interrupted; rejected for side effects no one is watching. Worth revisiting for a turn whose open call only read.
- **Queued messages** for a running turn (#320, #334) live in memory, so a restart loses those not yet recorded; keeping them is #335's.

### Replicas, briefly

Nothing above depends on this. In one process a thread's slot is its lease. With replicas, a server takes a lease on a thread in the data store before it runs or continues a turn, renews it every 10 seconds while it works, and lets it lapse after 30; another replica answers `-32011` meanwhile (`session.md`, "Turns that outlive a server"). A wait holds no lease, so any replica may take its answer. One replica at a time holds the organization's scheduler lease. Idempotency keys move from memory into the log. A watch on one replica of a turn running on another reads new events from the store (Postgres `LISTEN`/`NOTIFY`); live updates reach only the running replica's watchers. The log's `(thread, seq)` stays the last guard.

## Watching

### A thread

- **`thread/watch`** (`thread`, optional `after`, a `seq`) on every transport: the thread's events after `after` from the log, then its events as they are recorded and its live updates, until the client ends the watch, its connection closes, or the thread is deleted. Who may watch is checked as for `thread/events`.
- **No gap, no repeat.** The server subscribes before it reads the log and sends each `seq` once, in order. A watcher that falls behind the backlog (1,024 notifications) is caught up from the log by the server, so a client never sees a gap in events; live updates it missed are gone, since they are never replayed.
- **Every wait shows in events**: an approval by its `approval.requested`, a question by its `question` call, the folder by its record. A client that connects in the middle of a wait shows it; `turn/live`'s `asked` stays a nudge.
- **Over stdio and the socket**: notifications on the connection, each naming its thread, so one connection watches many, at most 64. A watch answers its request when the client sends `thread/unwatch` for it or the thread is deleted.
- **Over HTTP**: a `POST` answered with `text/event-stream`, as `turn/start` is today: an event per notification, its `id` the `seq` of a recorded event, a comment every 15 seconds so proxies keep it open, and the response last. A client reconnects by sending it again with `after`, the last `seq` it received; browsers stream it with `fetch`, since `EventSource` cannot send the sign-in header. One watch per request, within the 1,024 connections.
- **`turn/start`'s stream** is a watch that ends with the turn's result.

### What a list shows

- **`work/watch`** (the name is the schema's to settle): one stream per client, across every space the caller works in, of what lists show: a thread's summary when it starts or its status (running, waiting, idle), title or standing changes; a task when it starts or ends; a schedule when it is made or deleted. It carries only what `thread/list`, `tasks/list` and `schedule/list` answer, and it replaces the web app's polling (threads every 10 seconds, tasks every 5, #286).
- **Watch, then list.** A client starts the watch, then lists, and applies both by id, the newer winning, so nothing between the two is lost; after a reconnect it does the same.
- Considered: a stream of tasks alone, #286's ask; one stream for what every list shows costs the same and also serves Needs you.

### Ready for thread items

Items are a fold of events (#311), so `after` stays a `seq`. When #311 lands, a watch asks for items beside or instead of events on the same request, with the same order and the same conformance rule: rebuilding from `after` and then watching equals watching from the start. Needs you reads the view's wait now; #311 adds the ask's sentence and choices. Nothing here waits for #311.

## Tasks and schedules in the server

- **In every space.** A task runs in any space its caller works in; today only in the server's own. Each space's task ledger keeps its tasks and schedules, as today. The server reads every space's ledger when it starts, to recover tasks and find schedules, and keeps what it read, updated by its own writes.
- **Owned by the server.** Each task is a job the server owns and awaits; `tasks/stop`, archiving or deleting its thread, or a graceful stop ends it. `[tasks] max_running` counts a person's running tasks across spaces; with no rule, a person's server runs at most 4 at once (owner decision 8), since each is a worktree, model calls and commands on one computer. One more is refused (`-32015`); a schedule's run held back starts on a later tick, once, as today. `max_schedules` as today.
- **The scheduler** is one loop over every space's schedules, in UTC as today; it was the task host's.
- **When it starts**, tasks the ledgers have open are recorded `interrupted` (`recover_tasks`, now over every space), and missed runs run once.
- **Always on a copy** (decided). A task or a schedule's run in a project with a folder works in a git worktree of its own (`computers.md`), whatever `worktree` says, never in the folder itself: it never waits for the folder and never changes files under the person. Its work is on its `glave/` branch, and the app shows what it changed. A request for a task in place is refused rather than quietly moved. In a folder that is not a git repository, or one with no commit, a task is refused, with a sentence saying it needs one to work on a copy (owner decision 7). Considered: copying the folder, rejected for size and for secrets copied along; copy-on-write clones come later (`computers.md`, "Placement defaults"). In Chats a task works in its own scratch folder.
- **On a worker only when chosen**: when the person or the project chooses workers (`on = "cloud"`), not merely because a backend is set up as today, so the person's own computer comes first (`model.md`, item 9).
- **When one finishes**, its thread's watchers get `task/finished`, every `work/watch` gets the task, and later a push goes out (#333).

## One writer per folder

- **The folder's writer** is the thread with an open turn working in the folder itself, not in a worktree, scratch folder or worker, as the log says: running, or waiting for an answer, since it will go on writing. Subagents share their parent's place.
- **A second turn waits.** While another thread writes in a folder, a turn that would work there in place records that it waits for the folder (a small content-free record naming the writer, new in `events.md`) and waits, durably like any wait. Its thread is `waiting`, and the server says what for in a sentence ("Waiting for “Fix login” to finish changing files in this folder"). It starts when that turn ends, the first to wait first.
- **Or works on a copy.** The person may answer the wait by moving the thread to a worktree of its own, made from the folder's current commit; it starts there at once, and the model is told where it now is (`computers.md`, rules 5 and 6). Uncommitted changes in the folder stay in the folder.
- **In the terminal** the CLI prints the server's sentence and its own keys ("c works on a copy; Ctrl-C stops"). With no one at the terminal it waits: a script's turn works in place because the script wants its folder changed.
- **Tasks** never wait for a folder and never hold one, since they always work on a copy.
- Considered: holding the folder only from a turn's first change, which allows more at once but makes a wait in the middle of a turn, harder to show and to resume; and refusing the second turn so each client waits, which puts the queue in every client and loses it when a phone closes. A whole turn's hold, kept in the server and in the log, is the simplest that survives a restart.

## What breaks

There are no users yet; each pull request still says what it breaks.

- `glave` alone opens the browser: the terminal chat, `/compact`, `/memory` and the banner are gone, and so is `glave app`.
- The folder's task host is gone: `glave serve --socket`, its socket, log and marks in `run/`, and the word at a session's start about tasks that finished since.
- `--sandbox`, `--network`, `--no-network`, `--on` and `--max-steps` leave the turn commands; `--model` must name a model the server offers.
- `GLAVE_BASE_URL`, `GLAVE_MODEL` and `GLAVE_API_KEY` in a command's shell no longer choose a running server's model.
- A turn started in the terminal outlives the terminal; Ctrl-C still stops it.
- The folder a server started in no longer lends its tools, settings or space to every thread; a request without a space acts in Chats.
- Tasks and schedules' runs always work on a copy, refuse a folder that is not a repository, and go to workers only when chosen; `glave schedule add --worktree` goes.
- `glave threads` and `glave memory` go through the server.
- A sign-in link carries a one-time code; a browser stays signed in across restarts.
- A graceful stop records a running turn `interrupted`, not `cancelled`.
- The threads view's fold changes, so stores rebuild their views once.
- "`glave` cold start to prompt" has no prompt to measure (owner decision 9).

## The pull requests

Six, in order, each leaving main working. Where one grows past AGENTS.md's 1,500 lines, it splits where its entry says.

1. **Every project alike** (`feat(session)`). The server stops depending on the folder it started in: `Started` goes; a project's servers, hooks and settings reach its threads alone, read at each turn, with servers closed when idle; the person's own servers start in the home folder; a request without a space acts in Chats; tasks and schedules run in every space, each on a copy, on workers only when chosen. The CLI still runs in-process and the task host is still per folder, now just another client. Splits with tasks and schedules first.
   *Tests*: two projects in one server, each with a trusted server, hook and model: each thread gets only its own, and the start folder nothing more; a project's `[web]` and judge reach only its threads; an idle project's server closes and starts again; a task in each of two spaces works in a worktree; a task in a folder that is not a repository is refused; schedules in two spaces fire; `recover_tasks` covers both. *Evals*: yes, `--repeat 3`, since what a thread opens with and its tools are now assembled per project for every thread.
2. **The person's server** (`feat(cli)`). The lock, the socket, starting on first use, `glave serve` in the foreground, `glave server status|stop|restart`, loopback HTTP always on with one-time links and browser sessions, the graceful stop, the daily work, settings read again when they change. `--background`, `glave tasks`, `schedule`, `threads`, `memory`, `connect` and `connections` go to it, and the folder's task host goes; `glave "…"` still runs in-process, two writers on the store as with the task host today. Splits with the sign-in links and browser sessions first.
   *Tests*: two starts at once make one server; a stale socket is replaced only by the lock's holder; a socket in a folder others may open, or another user's, is refused; the server outlives the terminal that started it; a sandboxed command cannot reach the socket nor read the token or session key (macOS and Linux CI); a link's code works once and expires; a browser session survives a restart and ends after signing out; an edited `config.toml` changes the next turn's model; a stop records running tasks interrupted. *Evals*: no; the model sees nothing new.
3. **The CLI as a client** (`feat(cli)`). Turns, `-c` and goals through the socket, followed with the code that follows a server elsewhere; turns answered by no one where no one is at the terminal; Ctrl-C cancels and closing does not; `glave` opens the browser at the folder's project (the app gains a project route), asking once; the chat, `glave app` and the turn commands' server flags go; `serve --stdio` relays; README, AGENTS.md and the startup budget follow (owner decision 9 first).
   *Tests*: the binary against a real server in a fresh home with a scripted model: a turn; an approval answered in place; one declined with no terminal; Ctrl-C; a client killed mid-turn while the turn finishes; a new folder asked once; the home folder opening Chats; a trust ask answered in the terminal; `-c` finding the folder's thread; the consumer journey test if #251 has landed. *Evals*: no; evals run the harness in-process, and the model is sent the same.
4. **Durable waits** (`feat(session)`). The view's wait; repair when the server starts; loading no longer closes a waiting turn; answers continue a turn from the log, through subagents too; a graceful stop records `interrupted`.
   *Tests*, in the conformance suite on every store: a server stopped while an approval waits; a new one on the same store lists it waiting, the answer continues it to the end, and its events equal an uninterrupted run's but for ids and times; the same for a question, a denial, a subagent's approval, and subagents side by side with one finished; a property test stopping a scripted turn at every event boundary ends it waiting or interrupted, never with a call run twice; Needs you right after a restart. *Evals*: yes, `--repeat 3`: the loop gains an entry.
5. **Watching** (`feat(session)`, `feat(web)`). `thread/watch` and `thread/unwatch` on every transport, and `work/watch`; the web app follows threads it did not start, live, and stops polling.
   *Tests*: on all four transports, rebuilding from `after` then watching equals watching from the start; a watcher pushed past the backlog still gets every event; a reconnect with `after` misses nothing; `work/watch` brings a task's start and end and a thread's status; in the browser (Playwright, axe), a second tab sees a turn and its approval live, and the tasks row runs no timer. *Evals*: no.
6. **One writer per folder** (`feat(session)`). The folder's wait, its record and the view's kind, "work on a copy", and its words in the CLI and the app.
   *Tests*: in one folder, a second thread's turn waits and starts when the first ends; it still waits after a restart; "work on a copy" moves it and it starts; a first turn waiting for an approval keeps the folder; tasks never wait. *Evals*: yes, `--repeat 3`: a moved thread is told where it is.

Starting at login follows once owner decision 1 is made.

## Owner decisions

1. **Start at login.** Recommended: offered, never installed unasked: `glave server --at-login`, and one question when the person makes their first schedule; a Settings switch with settings as data.
2. **The port.** Recommended: 4180 always, so a browser's origin stays the same; any free port only when 4180 is taken, said once; configurable.
3. **No idle exit.** Recommended: the server runs until logout, shutdown or `glave server stop`, and lets go of memory and tool servers instead.
4. **A turn outlives its terminal.** Recommended: yes; Ctrl-C stops it, closing the terminal does not.
5. **Scripts and CI.** Recommended: a turn with no one at the terminal declines what would ask, as today, and never waits for another client.
6. **A browser's session.** Recommended: 30 days from its last use, loopback only, renewed as used, with "Sign out other browsers".
7. **A task in a folder that is not a repository.** Recommended: refused with a sentence, until copy-on-write clones.
8. **Tasks at once.** Recommended: 4 per person when the policy sets none.
9. **The startup budget.** Recommended: replace "cold start to prompt" with "a command answered by a running server, < 50 ms", and budget the server's own start from its first measurement.
