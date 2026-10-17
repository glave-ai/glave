# One server per person

How a person's glave runs as one server on their own computer: how it starts and stops, how every client finds it, what each project brings to its own threads, and what survives a restart. The server holds every project, thread, task and schedule; the web app, a phone, the CLI's commands and later a terminal UI are its clients. It is step 2 of #294 and the rest of #64, and it settles items 1 to 3 of `model.md`'s "Where the code disagrees", and the part of item 9 that concerns tasks.

## Status

Designed, not built; reviewed adversarially on 2026-10-17. Decisions marked *decided* were made in that review; those marked (decided) were made with #294. "The pull requests" gives the order. Each pull request moves the sections of `session.md`, `policy.md`, `computers.md`, `storage.md` and `events.md` that it changes, and AGENTS.md where a rule or a budget changes; this spec keeps the decisions that span them.

## How the others do it

- **Codex's app-server** ([docs](https://learn.chatgpt.com/docs/app-server), [remote connections](https://learn.chatgpt.com/docs/remote-connections)) is started by its client: the docs' example spawns `codex app-server` over stdio, and the desktop app hosts what a phone reaches, so remote access ends when that computer sleeps, goes offline or quits the app. `--listen unix://` serves a default control socket and `codex --remote` attaches the terminal UI to a running server. An approval is a request from the server to a connection, scoped by thread and turn; a pending request is cleared when its turn is interrupted. A thread with no subscriber stays loaded until it has had no activity for 30 minutes. Non-loopback WebSocket listeners accept unauthenticated connections by default for now.
- **OpenCode** ([server](https://opencode.ai/docs/server/), [web](https://opencode.ai/docs/web/), [CLI](https://opencode.ai/docs/cli/)): `opencode` starts a terminal UI and a server in one process, on a random port (per the server docs); `opencode serve` runs a headless server on 127.0.0.1:4096; `opencode web` opens the browser on a random port; `opencode attach` and `run --attach` reuse a running server so MCP servers don't start cold on every run. One server serves many folders: each request names its folder in an `x-opencode-directory` header, the server's own folder (`process.cwd()`) when it names none ([`location.ts`](https://github.com/anomalyco/opencode/blob/dev/packages/server/src/location.ts)), and each folder's config, plugins and MCP servers are built on first use and kept with no idle limit (`idleTimeToLive: Duration.infinity` in [`location-services.ts`](https://github.com/anomalyco/opencode/blob/dev/packages/core/src/location-services.ts)). Without `OPENCODE_SERVER_PASSWORD` it asks no one to sign in. Permissions are answered with a `POST`; events stream over server-sent events.
- **Claude Code** ([agent view](https://code.claude.com/docs/en/agent-view), [Remote Control](https://code.claude.com/docs/en/remote-control), [desktop scheduled tasks](https://code.claude.com/docs/en/desktop-scheduled-tasks)): a supervisor, the "background service", runs background sessions; Claude Code starts it the first time a session is sent to the background or agent view opens, one per config folder (`CLAUDE_CONFIG_DIR` makes a separate instance), managed with `claude daemon status` and `stop`. Sessions are saved on disk and survive supervisor restarts; after an auto-update the supervisor restarts itself onto the new version and moves idle sessions over. An idle session's process is stopped after about an hour and resumes on reply. After a shutdown, a session waiting on the person stays under Needs input, and one that was working shows failed and continues only when the person replies. Remote Control keeps the local process running, uses outbound HTTPS only, and keeps permission prompts open until answered. Desktop scheduled tasks run only while the app runs and the computer is awake, with one catch-up run for the latest missed time.

What glave takes: Claude Code's start on first use, one server per glave folder, sessions that stay in Needs you after a restart, and interrupted work that goes on only when the person says; OpenCode's one server for every folder, each folder's tools built on first use, and a fixed default port; Codex's clients that are all clients of one protocol, and threads unloaded after 30 idle minutes. Where glave differs: a turn paused for an approval or a question is answered where it stopped after a restart, which none of them documents; a project's tools are bounded and closed when idle; a listener never runs without sign-in; and the server never restarts itself onto a new version (below).

## The person's server

### One per glave folder

- **One server per glave folder** (`GLAVE_HOME`, `~/.glave` by default), the holder of its lock. It opens the data store, holds the keys, runs the harness and the gateway, and is the only glave that fires schedules and recovers tasks, and the one whose tasks outlive the clients that start them. Other glaves may write the same store meanwhile, as today: a command run in-process where no server runs, `glave serve --stdio`'s own server, and worktree cleanup; the log's `(thread, seq)` guard and "Records from outside" (`session.md`) keep them apart. A second glave folder (tests, a second profile) is a second person's server.
- **It has no folder of its own.** Nothing depends on where it started: no start folder's project, settings, tools or instructions. Today `wire` reads all of them from the current folder.
- **An organization runs the same program** (`glave serve` in a unit or a container), its people reaching it over HTTP.

### Starting

- **On first use, from the person's own shell.** A command run with a terminal on its input and output that needs the server and finds none starts it: `glave serve` in the background, in a session of its own so neither Ctrl-C nor closing the terminal reaches it. The command waits until the socket answers, at most 20 seconds as the task host's starter does today, then goes on. Claude Code starts its supervisor the same way.
- **Never from a script** (*decided*). With no one at the terminal (a pipe, CI, Docker's `RUN`, `ssh host glave "…"`, the binary's own tests in throwaway homes), a command never starts the person's server: when one runs it is its client, and when none does it runs in-process, as today, so nothing is left behind.
- **In the foreground**, `glave serve` is the same server for a systemd unit, a launchd agent, a container, or an organization (`--http <address>` with `--issuer` and `--audience`). Its flags are its choices, as today.
- **At login: offered, never installed unasked** (owner decision 1, pending). `glave server --at-login on|off` writes or removes a launchd agent on macOS or a systemd user unit on Linux that runs `glave serve`; the app asks once, when the person makes their first schedule ("Schedules run only while glave runs. Start glave when you log in?"). Only schedules need it, macOS tells the person a background item was added, and neither Codex, OpenCode nor Claude Code installs one. A login start has no shell around it: launchd's `PATH` lacks nvm, uv and Homebrew, so `npx` and `uvx` servers would fail only then. The item records the person's login-shell `PATH` when it is installed, and Settings shows it. Such a server has no shell's variables either ("Its settings"), and on macOS it asks for folders such as Documents in its own name rather than the terminal's.
- **Fast first.** It listens as soon as the store is open; the person's MCP servers and the daily work start after.
- Considered: socket activation by launchd or systemd, rejected because a schedule needs a running process at its time and it needs the login item anyway; a server per client, as Codex's stdio app-server, rejected because tasks, schedules and waits must outlive every client.

### Finding it

- **The socket** is `GLAVE_HOME/run/server.sock`, `0600` in a `0700` folder; where that path is too long for a socket, `/tmp/glave-<uid>/<hash of the glave folder>.sock` in a `0700` folder, as the task host's today. A client checks the folder's owner and mode and the listener's user before it sends anything (`daemon::Host::connect` today). A local transport acts as the local person, with no token.
- **One copy.** The server holds an exclusive lock on `run/server.lock` for its whole life (`File::try_lock` in the standard library), taken before it listens. The operating system lets go of it when the process ends, however it ends, so a crash leaves no stale lock; only the holder removes a socket file a crash left. A second `glave serve` exits saying where the first one is. Two commands racing to start one each spawn a server; one gets the lock, the other exits, and both commands connect to the winner.
- **The browser.** The person's server always listens on loopback HTTP at 127.0.0.1:4180 (*decided*), or any free port when that is taken, written in `serve.url` as today; `glave serve --http` listens where it is told instead. The same port across restarts keeps the same origin, which keeps a browser's stored state, an installed app and push subscriptions (#333).
- **A link carries a one-time code, never a token.** `glave` asks the server over the socket for a link; its fragment carries a code good once, for 5 minutes, which the app trades for a browser session. Today's link carries the local token itself, tolerable while a server lived as long as one terminal; a person's server lives for days, and a link printed into scrollback, a terminal log or a shared screen must not stay a key. A link printed over SSH works only through a tunnel to the loopback port (`ssh -L 4180:127.0.0.1:4180`), since the server answers nowhere else.
- **A browser's session is a bearer header, never a cookie** (*decided*). A page elsewhere cannot make the browser send it, and a name rebound to 127.0.0.1 reaches a server that sees no session, so neither cross-site requests nor DNS rebinding can use it. It outlives a restart: signed with a key kept in glave's folder, accepted only from loopback as the local token is, valid for 30 days from its last use, and "Sign out other browsers" replaces the key. An upgrade or a crash signs no tab out, and a notification opens the app signed in. #306's passkey sign-in from elsewhere will issue the same sessions.
- **Only to the server that issued it** (*decided*). On a computer people share, another user's glave can hold 4180 while this one is down. The stored session names its server by an id that `server/discover` answers without sign-in, and the app presents the session only to that server.
- **A known limit: people you don't trust on the same computer.** A local user who takes the port while this server is down serves their own page in the tab's origin, and no check in the app can win against a script running there. On a computer shared with people you don't trust, use your organization's server, or keep glave to the Unix socket.
- **Host and Origin.** `http.rs` checks neither today. It should, on the person's loopback listener: a `Host` that is not the listener's own address (or #306's public origin) and an `Origin` other than the server's own are refused. That is a second line, keeping a rebound or foreign page from reading even the app's files, as Codex's listener refuses probes that carry an `Origin`; the bearer stays the boundary.
- **The local token** (`serve.token`) stays for programs on this computer (`GLAVE_SERVER_TOKEN`), made at each start as today.
- **Loopback is a second line, never the only one.** A tunnel or proxy on this computer makes every request it forwards come from loopback, so each request still needs its token or session, and #306 decides how a request through one is told apart from the person at this computer.
- **Out of the agent's reach.** Sandboxed commands reach no Unix socket (seccomp on Linux; on macOS the profile allows only `remote ip` and mDNSResponder's socket), and glave's folder, with the token and the keys, is hidden (`sandbox.md`). Loopback HTTP is reachable with the network on, but nothing there answers without a sign-in. A command run outside the sandbox reaches the server as the person can, which is why running outside the sandbox asks every time (`approvals.md`). `glave` run inside glave's sandbox cannot reach its server, and says so.

### When it isn't running

- A command from the person's shell starts it; when it cannot, the command says why in one sentence and where the log is. A command with no one at the terminal runs in-process.
- A browser tab says glave isn't running on this computer and to open glave again; a browser cannot start a process. A phone says it cannot reach the person's computer, and pushes stop until it is back. Those words are each client's own.
- Nothing is lost: history, waiting approvals and questions ("Durable turns"), tasks' records and schedules are in the store. Schedules missed meanwhile run once, as soon as the server runs again (`session.md`, "Schedules"); tasks that were running are recorded as interrupted.

### Stopping

- **It runs until** the person logs out, the computer shuts down, or `glave server stop`. There is no idle exit for the person's own server (*decided*): schedules need it, and a browser link must keep working. It lets go of what it does not use instead ("Bounds"). Today's task host exits when idle because there is one per folder.
- **A graceful stop** (`SIGTERM`, `SIGINT`, `glave server stop`, logging out) takes no new requests, gives running turns up to 10 seconds to reach a wait or end, then cancels the rest and records each `interrupted`, not `cancelled`, since the person did not stop them. Waiting turns stay waiting. Running tasks are recorded `interrupted`, MCP servers are closed, workers released, the socket removed and the lock let go.
- **A crash** leaves the store as any crash does, and the next start repairs it ("Durable turns"). Child processes a crashed server started, its MCP servers and on macOS its commands, can outlive it; nothing records their process ids, so the next start cannot find them. On Linux, bubblewrap's `--die-with-parent` ends the commands. Recording the ids comes later.
- **The log**, `run/server.log`, is capped at 10 MB and rotated, one older file kept.
- **`glave server status`** says whether it runs, its version, its address, and what is running and waiting; **`glave server restart`** stops it and starts it from this shell.
- **Upgrades.** A command compares its version with the server's (`server/discover`). When they differ but share a protocol version, it works and says in one line that `glave server restart` moves to the new one, which is safe whenever no turn is running, since waits survive it. The server never restarts itself: that is glave's choice, not the industry default (Claude Code's supervisor restarts itself after an auto-update), because a build run from a checkout would replace the person's server with itself. The login item starts the installed binary at the next login.

### Its settings

- **Read again, never frozen.** Today each run reads `config.toml`, `mcp.json`, the person's AGENTS.md and the policy afresh. A server that lives for days reads each again at the start of a turn when its content's hash changed, as it reads a project's trust today; a choice applies from the next turn. A changed `mcp.json` reconciles the running servers: added ones start, removed ones stop, changed ones restart. What only a start applies (the store's address, the key root, identity providers, telemetry, the gateway's address, the workers' backend) the server reports as waiting for a restart, and clients show it.
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

Every command is a client of the person's server over its socket, or of a server elsewhere with `--server` or `[server] url` as today, in which case none is started here; it uses nothing the API does not offer (`session.md`, "Principles"). Where no server runs and no one is at the terminal, the same requests go to a server in the command's own process, as today.

| Command | What it asks the server |
| --- | --- |
| `glave` | the project this folder leads to, asking once when there is none (below); then a sign-in link to it, opened in the browser, or printed where there is no screen. With no one at the terminal and no server running, it says to run it from a terminal |
| `glave "…"` | the project, as above; `thread/start` in it and `turn/start`, followed; approvals, questions and the project's trust ask answered in place with `approval/decide`, `question/answer` and `trust/answer` |
| `glave -c "…"` | the project's newest thread (`thread/list`), then `turn/start`; `glave -c` alone opens that thread in the browser |
| `glave --goal …` | `thread/start`, then `goal/start`, followed |
| `glave -b "…"` | `thread/start` and `turn/start` (or `goal/start`) with `background`; prints the task and exits. Refused, saying why, where no server runs and no one is at the terminal: a task needs the person's server |
| `glave tasks [stop <id>]` | `tasks/list` across every space the person works in; `tasks/stop` |
| `glave schedule add\|list\|rm` | `schedule/create` in this folder's project (in Chats where the folder is none), `schedule/list`, `schedule/delete` |
| `glave threads …` | `thread/list` in the project, `thread/archive`, `thread/unarchive`, `thread/delete` (today it writes the log itself) |
| `glave memory …` | `memory/*` (today it opens the store itself) |
| `glave connect`, `glave connections` | `credential/*` (today through the folder's task host) |
| `glave serve --stdio` | nothing: it is the calling program's own server (below) |
| `glave server status\|stop\|restart\|--at-login` | the process itself: its lock, its socket, a signal, the login item |

- **Local still**: `glave policy explain`, `glave skills` and `glave commands` read this folder's and the person's files, as the server reads them, and change nothing. `glave worktrees` works on this machine's worktrees as today, recording straight to the log; moving cleanup into the server comes later. `glave computer serve` and `glave gateway serve` are other roles.
- **A program's own server** (*decided*). `glave serve --stdio` is always the calling program's own server, in its process and independent of the person's, as today: it fires no schedules and recovers no tasks, and the tasks it runs end with it. Its waits are in the log like any; while it runs they are answered there, and the log's `(thread, seq)` guard keeps two servers from both continuing one.
- **Following.** The socket carries a turn's notifications as HTTP's stream does, so the CLI follows the person's server with the code that follows a server elsewhere today (`glave_session::client`): one path, here or there.
- **Ctrl-C stops the turn** (`turn/cancel`), as today. **Closing the terminal does not** (*decided*): the turn goes on in the server, its approvals wait for any client, and `glave -c` picks the thread up. Today the turn dies with the process; Claude Code's background sessions go on with no terminal attached.
- **No one at the terminal** (*decided*): the CLI starts its turn answered by no one, a new choice on `turn/start` and `goal/start`. Anything that would ask is declined at once and `question` is not offered, as today and as for a task, so a script never hangs or waits for another client. Whether anyone answers becomes a property of a turn, not of the server (`Answering` today).
- **The flags.** `--model` and `--approval` go to the server as the thread's choice, as `--server` sends them today; a model must be one the server offers, so one nothing sets up is refused, naming those it does. `--worktree` is `thread/start`'s. `--sandbox`, `--network`, `--no-network`, `--on` and `--max-steps` choose how a server runs, so they leave the turn commands: they are settings, and `glave serve` still takes them. `--events` exports the thread as today; `--log` records the client's own spans.
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

## Watching

- **`thread/watch`** (`thread`, optional `after`, a `seq`) on every transport: the thread's events after `after` from the log, then its events as they are recorded and its live updates, until the client ends the watch, its connection closes, or the thread is deleted. Who may watch is checked as for `thread/events`.
- **No gap, no repeat.** The server subscribes before it reads the log and sends each `seq` once, in order. A watcher that falls behind the backlog (1,024 notifications) is caught up from the log by the server, so a client never sees a gap in what the server recorded; live updates it missed are gone, since they are never replayed. What another glave appends to the store (worktree cleanup, an in-process run) reaches a watcher only when it next reads from the log; tailing the store for it comes later.
- **Every wait shows in events**: an approval by its `approval.requested`, a question by its `question` call. A client that connects in the middle of a wait shows it; `turn/live`'s `asked` stays a nudge.
- **Over stdio and the socket**: notifications on the connection, each naming its thread, so one connection watches many, at most 64. A watch answers its request when the client sends `thread/unwatch` for it or the thread is deleted.
- **Over HTTP**: a `POST` answered with `text/event-stream`, as `turn/start` is today: an event per notification, its `id` the `seq` of a recorded event, a comment every 15 seconds so proxies keep it open, and the response last. A client reconnects by sending it again with `after`, the last `seq` it received; browsers stream it with `fetch`, since `EventSource` cannot send the sign-in header. One watch per request, within the 1,024 connections.
- **`turn/start`'s stream is a watch** that ends with the turn's result: what the client follows is the thread's log and live updates, not the turn's place in memory. Durable turns depend on it.
- **Lists still poll.** A stream of what lists show (#286) waits for a need: polling every few seconds is fine for one person.
- **Ready for thread items.** Items are a fold of events (#311), so `after` stays a `seq`. When #311 lands, a watch asks for items beside or instead of events on the same request, with the same order and the same conformance rule: rebuilding from `after` and then watching equals watching from the start. Nothing here waits for #311.

## Durable turns

### Every wait is in the log

- **A turn waits** at its first unfinished tool call when that call is an approval asked and not decided (`approval.requested`, no `approval.decided`), a `question` call with no result, or an `agent` call whose child's turn waits. Nothing else needs keeping: the log already says it.
- **The threads view** keeps, content-free, whether a turn is open and, when it waits, the wait's kind and `seq`, folded from each append as the rest of the view is (`events.md`, "Storage"): set when the turn's first unfinished call becomes a wait, cleared by the turn's next event. Its fold's version goes up, so stores rebuild views when they open. So a list, Needs you and a summary's `waiting` are right after a restart without opening the thread; today `waiting` comes from the server's memory.
- **A parent finds its children through the view.** The parent's log never names a child's thread: `tool.call.requested` carries no child id, and only the child's `thread.started.parent` (thread and call) links them. The view keeps only the parent's thread today, and `ThreadQuery` (`crates/ports/src/view.rs`) cannot ask by it; the view gains the parent's call id, and the query a filter by parent thread and call, so a parent with an open `agent` call finds its child.
- **A waiting turn is a running turn** for everything else: a second `turn/start` is refused (`-32002`), and so are archiving and deleting its thread, until it ends or is stopped.
- **No wait expires.** The person ends one with Stop, which denies it, as today.

### After a restart

- **When it starts**, the server repairs every turn the view shows open and not at a wait, its own or a child's: its dangling calls get the interrupted result and the turn ends `interrupted`, as loading a thread does today, so nothing shows running that is not. A turn at a wait is left as it is, and loading its thread no longer closes it (`resume_thread` closes every open turn today).
- **A waiting thread holds nothing in memory**: it is rebuilt from the log when its answer comes, and a client following it keeps following, since `turn/start`'s stream is a watch.

### An answer continues the turn

- **`approval/decide` and `question/answer`** for a wait no running turn holds: the server checks it against the log (refused as today, `-32004` or `-32006`, when nothing waits), records the answer exactly as the running turn would (`approval.decided` with `by`, or the question's result), and continues the turn from the log. The first answer wins; another for the same wait is refused as not pending. The answered call runs or is declined, the calls after it in that response, which never started, run in order, and the loop goes on to the model. It is one code path: the turn's own loop, entered at "the rest of this response's calls". `PreToolUse` hooks that ran before the ask do not run again; `hook.ran` recorded them. A goal goes on to its next turns as it would have.
- **What a running turn kept only in memory** is rebuilt or let go: its tallies from the turn's `model.responded` events; its model, mode and caller from `turn.started` and the thread's choices; the count that refuses a call repeated many times starts again. A task never waits, since no one answers it, so no budget is held across a wait.
- **The client that answered** follows the continued turn as one it started, and every watcher sees it.

### Subagents never run twice

- **An open `agent` call is never run again after a restart.** Each gets its result from its child, found through the view: the child's report when its turn ended; the report an interrupted subagent gives today when its view says interrupted; nothing yet while it waits, so the parent waits too. A call whose child never started gets the interrupted result, as any dangling call does.
- **Side by side.** Child A waits for an approval while child B runs a command, and the server dies. At the next start B is repaired `interrupted`, and the parent is left waiting, since A waits. When A is answered and its turn ends, the parent records A's report and B's interrupted report, the results of the calls side by side, and goes on; B is never started again as a new child.

### A turn that dies mid-call

- **Interrupted, never redone on its own.** When the server dies during a model call or a tool call, not at a wait, the next start ends the turn `interrupted`, its open calls closed with the interrupted result, as today. A call that may have half run (a push, a migration, a payment) must not run twice without the person. The thread says so; the person goes on with a message, or `goal/start` for a goal. A task is recorded `interrupted` and does not run again; a schedule runs at its next time. Claude Code does the same after a shutdown.
- Considered: continuing such turns from the log on their own, the model told which call was interrupted; rejected for side effects no one is watching. Worth revisiting for a turn whose open call only read.
- **Queued messages** for a running turn (#320, #334) live in memory, so a restart loses those not yet recorded; keeping them is #335's.
- **Replicas** take the same waits through leases (`session.md`, "Turns that outlive a server"); nothing here depends on them.

## Tasks and schedules in the server

- **In every space.** A task runs in any space its caller works in; today only in the server's own. Each space's task ledger keeps its tasks and schedules, as today. The server reads every space's ledger when it starts, to recover tasks and find schedules, and keeps what it read, updated by its own writes.
- **Owned by the server.** Each task is a job the server owns and awaits; `tasks/stop`, archiving or deleting its thread, or a graceful stop ends it. `[tasks] max_running` counts a person's running tasks across spaces; with no rule, a person's server runs at most 4 at once (*decided*), since each is a worktree, model calls and commands on one computer. One more is refused (`-32015`); a schedule's run held back starts on a later tick, once, as today. `max_schedules` as today.
- **The scheduler** is one loop over every space's schedules, in UTC as today; it was the task host's. Only the lock's holder runs it.
- **When it starts**, tasks the ledgers have open are recorded `interrupted` (`recover_tasks`, now over every space), and missed runs run once.
- **Always on a copy** (decided). A task or a schedule's run in a project with a folder works in a git worktree of its own (`computers.md`), whatever `worktree` says, never in the folder itself: it never meets the one-writer rule and never changes files under the person. Its work is on its `glave/` branch, and the app shows what it changed. A request for a task in place is refused rather than quietly moved. In a folder that is not a git repository, or one with no commit, a task is refused, with a sentence saying it needs one to work on a copy (*decided*). Considered: copying the folder, rejected for size and for secrets copied along; copy-on-write clones come later (`computers.md`, "Placement defaults"). In Chats a task works in its own scratch folder.
- **On a worker only when chosen**: when the person or the project chooses workers (`on = "cloud"`), not merely because a backend is set up as today, so the person's own computer comes first (`model.md`, item 9).
- **When one finishes**, its thread's watchers get `task/finished`, and later a push goes out (#333).

## One writer per folder

- **The folder's holder** is the thread with an open turn working in the folder itself, not in a worktree, scratch folder or worker, as the log says: running, or waiting for the person. Any turn holds it, a read-only one too, since glave cannot know beforehand that a turn will only read. Subagents share their parent's place.
- **A second turn is refused first.** A turn that would work in place in a held folder is refused before anything is recorded, with the server's sentence naming the holder, saying when it waits for the person and where to answer it ("“Fix login” is working in this folder and waits for you to approve a command; answer it in that thread"), the holding thread as a field a client links, and a way to work on a copy instead. The client asks again for a copy: a new thread starts in a worktree of its own from the folder's current commit; one already working in place moves to one, and the model is told where it now is (`computers.md`, rules 5 and 6). Uncommitted changes in the folder stay in the folder.
- **In the terminal** the CLI prints the server's sentence and offers its own key for a copy; with no one at the terminal it exits with the sentence and a failing status.
- **Tasks** never hold a folder and are never refused for one, since they always work on a copy.
- **Later, only if needed.** A durable queue, where the second turn waits for the folder and starts when it frees, comes only when evals or use show the refusal is not enough (AGENTS.md, "Nothing before its second user"). Considered too: holding the folder only from a turn's first change, which allows more at once but needs a wait in the middle of a turn.

## What breaks

There are no users yet; each pull request still says what it breaks.

- `glave` alone opens the browser: the terminal chat, `/compact`, `/memory` and the banner are gone, and so is `glave app`.
- The folder's task host is gone: `glave serve --socket`, its socket, log and marks in `run/`, and the word at a session's start about tasks that finished since.
- `--sandbox`, `--network`, `--no-network`, `--on` and `--max-steps` leave the turn commands; `--model` must name a model the server offers.
- `GLAVE_BASE_URL`, `GLAVE_MODEL` and `GLAVE_API_KEY` in a command's shell no longer choose a running server's model.
- A turn started in the terminal outlives the terminal; Ctrl-C still stops it.
- `glave -b` with no one at the terminal and no server running is refused; `glave serve --stdio` runs no schedules and recovers no tasks.
- The folder a server started in no longer lends its tools, settings or space to every thread; a request without a space acts in Chats.
- Tasks and schedules' runs always work on a copy, refuse a folder that is not a repository, and go to workers only when chosen; `glave schedule add --worktree` goes.
- `glave threads` and `glave memory` go through the server when one runs.
- Clients of the person's server miss, live, what other glaves append (worktree cleanup, in-process runs, and from PR 2 until PR 3 every `glave "…"`), and see it when they next read.
- A sign-in link carries a one-time code; a browser stays signed in across restarts, and only with the server that signed it in.
- A turn in place in a folder another thread holds is refused.
- A graceful stop records a running turn `interrupted`, not `cancelled`.
- The threads view's fold changes, so stores rebuild their views once.
- "`glave` cold start to prompt" has no prompt to measure (owner decision 9).

## The pull requests

Six, in order, each leaving main working; the first two in named parts, each its own pull request.

1. **Every project alike** (`feat(session)`). *1a*: tasks and schedules in every space, each on a copy, on workers only when chosen. *1b*: no start folder (`Started` goes); a project's servers, hooks and settings reach its threads alone, read at each turn, its servers closed when idle; the person's own servers start in the home folder; a request without a space acts in Chats. The CLI still runs in-process and the task host is still per folder, now just another client.
   *Tests*: 1a: a task in each of two spaces works in a worktree; a task in a folder that is not a repository is refused; schedules in two spaces fire; `recover_tasks` covers both. 1b: two projects in one server, each with a trusted server, hook and model: each thread gets only its own, and the start folder nothing more; a project's `[web]` and judge reach only its threads; an idle project's server closes and starts again. *Evals*: 1a no; 1b yes, `--repeat 3`, since what a thread opens with and its tools are now assembled per project for every thread.
2. **The person's server** (`feat(cli)`). *2a*: one-time sign-in links and browser sessions: a bearer header bound to the id of the server that issued it, and the Host and Origin checks. *2b*: the lock, the socket, starting on first use from the person's shell only, `glave serve` in the foreground, `glave server status|stop|restart`, loopback HTTP always on, the graceful stop, the capped log. *2c*: settings read again by hash, with `mcp.json` reconciled; the daily work; `--background`, `glave tasks`, `schedule`, `threads`, `memory`, `connect` and `connections` go to the server, and the folder's task host goes. `glave "…"` still runs in-process until PR 3, so the person's clients see its appends only when they next read.
   *Tests*: 2a: a link's code works once and expires; a session survives a restart, ends after signing out, is never sent to a server with another id; a foreign `Host` or `Origin` is refused. 2b: two starts at once make one server; a stale socket is replaced only by the lock's holder; a socket in a folder others may open, or another user's, is refused; the server outlives the terminal that started it; no command without a terminal starts one; a sandboxed command cannot reach the socket nor read the token or keys (macOS and Linux CI). 2c: an edited `config.toml` changes the next turn's model; a server added to `mcp.json` starts and a removed one stops; a stop records running tasks interrupted. *Evals*: no; the model sees nothing new.
3. **The CLI as a client** (`feat(cli)`). Turns, `-c` and goals through the socket, followed with the code that follows a server elsewhere, and in-process where no server runs and no one is at the terminal; turns answered by no one there; Ctrl-C cancels and closing does not; `glave` opens the browser at the folder's project (the app gains a project route), asking once; the chat, `glave app` and the turn commands' server flags go; README, AGENTS.md and the startup budgets follow (owner decision 9 first).
   *Tests*: the binary against a real server in a fresh home with a scripted model: a turn; an approval answered in place; Ctrl-C; a client killed mid-turn while the turn finishes; a new folder asked once; the home folder opening Chats; a trust ask answered in the terminal; `-c` finding the folder's thread. Without a terminal: a turn runs in-process and leaves no server behind, an approval is declined, `-b` is refused. The consumer journey test if #251 has landed. *Evals*: no; evals run the harness in-process, and the model is sent the same.
4. **Watching** (`feat(session)`, `feat(web)`). `thread/watch` and `thread/unwatch` with `after` on every transport, `turn/start`'s stream made a watch, and the web app following threads it did not start, live.
   *Tests*: on all four transports, rebuilding from `after` then watching equals watching from the start; a watcher pushed past the backlog still gets every event; a reconnect with `after` misses nothing; a turn's stream goes on across the turn being let go and rebuilt; in the browser (Playwright, axe), a second tab sees a turn and its approval live. *Evals*: no.
5. **Durable waits** (`feat(session)`). The view's open turn and wait, and its parent's call id with the query by parent and call; repair when the server starts; loading no longer closes a waiting turn; waiting turns let go of memory; answers continue a turn from the log on the one code path; open `agent` calls take their children's outcomes and never run twice; a graceful stop records `interrupted`.
   *Tests*, in the conformance suite on every store: a server stopped while an approval waits; a new one on the same store lists it waiting, the answer continues it to the end, and its events equal an uninterrupted run's but for ids and times; the same for a question, a denial and a subagent's approval; side by side, one child waiting and one running when the server stops: the running one is reported interrupted, never started again; a property test stopping a scripted turn at every event boundary, alone and with two children side by side, ends it waiting or interrupted, never with a call run twice; Needs you right after a restart. *Evals*: yes, `--repeat 3`: the loop gains an entry.
6. **One writer per folder** (`feat(session)`). The holder from the log, the refusal with its sentence and the holding thread, working on a copy, and the words in the CLI and the app.
   *Tests*: in one folder, a second turn in place is refused naming the first, and says when it waits for the person; a read-only first turn holds the folder too; "work on a copy" starts a new thread in a worktree and moves one already in place; a task is never refused. *Evals*: yes, `--repeat 3`: a moved thread is told where it is.

Starting at login follows once owner decision 1 is made.

## Owner decisions

Pending:

- **1. Start at login.** Recommended: offered at the first schedule, never installed unasked (`glave server --at-login`), with the login-shell `PATH` recorded; a Settings switch with settings as data.
- **9. The startup budgets.** Recommended: replace "cold start to prompt" with "a one-turn command against a running server with a scripted model, under 50 ms end to end", and add "the server listening within N ms of its start", with N set from the first measurement.

Decided in review (2026-10-17):

- **2. The port**: 4180 always, any free port only when taken, the browser's session bound to its server.
- **3. No idle exit** for the person's own server; scripts never start one.
- **4. A turn outlives its terminal**; Ctrl-C stops it.
- **5. Scripts and CI**: a turn with no one at the terminal declines what would ask, and never waits for another client.
- **6. A browser's session**: 30 days from its last use, a bearer header bound to its server, never a cookie, with "Sign out other browsers".
- **7. A task in a folder that is not a repository** is refused with a sentence, until copy-on-write clones.
- **8. Tasks at once**: 4 per person when the policy sets none.
