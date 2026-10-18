# The model

What glave is made of, from an organization down to a thread, and where each part runs. The other specs hold the detail; this one holds the shape they agree on. Decided with Chris on 2026-10-16 (issue #294).

## Status

Decided, mostly not built. "Where the code disagrees" lists what changes, in the order it lands.

## The hierarchy

| Level | Is | Holds |
| --- | --- | --- |
| Organization | The tenant: one on a laptop (never shown), one in a company, many on Glave Cloud | Model providers and their catalogs, the connections catalog, environments, policy, the theme, the budget |
| Team | A flat group of people, mirrored from the identity provider's groups where there are some | What it is granted: models, connections, budgets |
| Person | Someone who signs in | Their Chats, their computer, their own sign-ins (git hosts, MCP servers), their own choices |
| Project | A named place for work. It belongs to the organization, is managed by a person or a team, and is private to its manager, shared with people or teams, or open to everyone in the organization | Folders, instructions, variables, secrets, members |
| Thread | A conversation or task, in a project or in the person's Chats | Its events, and a worktree of the project's folders |

- **One rule at every level** (`policy.md`): choices, the nearest wins; constraints, they only narrow. A level grants only what it was given.
- **No project needs a team.** Anyone can make one, private to them at first. The organization always owns its data, so when a person leaves, the projects they managed pass to an admin, and nothing is lost or orphaned.
- **Words:** people see Chats and projects. "Space" stays the code's word for either: a project space, or a person's personal space (`storage.md`).
- **A laptop is an organization of one:** no teams, one person. The organization's screens are that person's Settings, and the word "organization" never appears there (AGENTS.md, "Keeping principle 4").

## Two roles: the server and the computer

- **The server** holds history, projects, settings and keys, runs the harness and the gateway, and answers every client: the web app, the CLI and outside agents. **The computer** holds files and runs commands. They talk through the computer protocol (`computers.md`): in-process on a laptop, over TLS elsewhere.
- **One server for a person on a laptop; one organization's server (replicated) in a company or on Glave Cloud.** No server is tied to the folder it started in, and nothing runs once per folder.
- **History never lives only on a computer,** so a computer can sleep, be reset or be replaced without losing a thread.
- **People work with glave in the browser; the CLI is commands.** It runs one turn (`glave "…"`, approvals asked in place, for scripts and CI), goals, tasks and schedules, and serves. `glave` alone opens the browser at the current folder's project, or prints the link where there's no browser. A terminal UI comes later (#308), laid out like the app (Chats and projects, a thread, what needs the person), as another thin client of the server; until then the CLI is commands and has no chat.
- **The server says what happened; each client draws it** (AGENTS.md, "Keeping principle 4"). A thread's items come from the server as typed facts with one plain sentence each, so the web app, the CLI, a notification and an outside agent say the same thing; how they look stays in each client. Items are worked out from the thread's events, which stay the record (#311).
- **A person's computer is one machine that's theirs:** their laptop, or a server or cloud machine they run (and, later, one Glave Cloud gives them). glave works on it the same way everywhere, as on a laptop today:
  - projects are folders on it, and a thread works in its project's folder in place, one thread changing a folder's files at a time;
  - "work on a separate copy" gives a thread a worktree for parallel work;
  - chats work in a scratch folder.
- **The agent's reach is drawn by the machine's own sandbox,** the operating system's (Seatbelt on macOS; bubblewrap, Landlock and seccomp on Linux), never by asking the model (`sandbox.md`). Three zones:
  - **Hidden:** secrets (keys, cloud and git sign-ins, browser profiles, keychains, shell history, `.env` files) and glave's own data.
  - **Read, never written:** the rest of the machine, and the machinery inside a project that runs code later (git hooks and config, `.glave/`, `.mcp.json`).
  - **Read and written:** the project folder or the chat's scratch folder, temp folders, and any folder the person grants for a thread ("Allow writing to ~/Downloads?").

  Approvals sit on top: a risky action asks even inside the writable zone. The zones bind the agent, never the person.
- **The machine's files live on a disk that's the person's;** the running machine is replaceable. It can be reset or moved without losing their files, and history is never on it.
- **Where glave runs the machine, its system comes from an image glave updates without asking;** the person's home (`/home/<name>`) is on their kept disk. What a person installs for the whole system goes on a setup list that runs again on each new image, so an update never loses it.
- **Fresh machines per task are an upgrade, not the default.** They're workers (`computers.md`, "Workers"), used for parallel or untrusted work where the machine can run them: Docker (rootless, or Podman, since a Docker socket is root on its host) or a company's Kubernetes. Code comes in by clone and leaves as a pushed `glave/` branch.
- **Isolating each project on its own machine** (a consultant keeping clients apart) is an option to add when someone needs it, never the default.
- **Not at launch:** browser sessions kept logged in on a machine. A logged-in browser driven by an agent that reads untrusted pages is the riskiest piece, and it comes later, with its own design.
- **Untrusted travels with work:** a thread an assistant starts after reading untrusted content inherits that label, so its risky actions need approval.
- **A person's files on a machine are personal data:** sealed under their key where the store keeps them, erased with it, and exportable.

## Settings are data

- **Everything an organization, team, person or project sets lives in the data store** with its owner, and is changed through the session API: model providers and accounts, connections, policy, budgets, the theme, the assistant's name and instructions.
- **Files and environment variables remain** for bootstrapping (the store's address, the identity provider, the key service) and for applying settings kept in git (`glave admin apply`).
- **A setting is managed by a file or in the app, never both.** A file-managed setting shows read-only in the app.

## Models

- **Providers are added in the app.** Each provider's catalog is cached in the data store, refreshed when the provider is added, daily, and on demand (#290).
- **A person's own model accounts are an organization's setting:** on for a person on their own, off by default in a company, where the organization's providers serve everyone.
- **One entry per model, and glave picks the route** (#289): the organization's cloud contract first, then the model maker's API, then aggregators, each later one only as fallback. Policy can pin a route. Every call records where it ran.
- **The picker lists** the default, the person's recent picks, the most used in the organization, then everything else through search.

## Admin

- **The screens:** `/admin` in the same web app, reached from the person's menu and shown only when `server/discover` says the caller is an admin.
- **The methods:** admin methods are part of the session API, checked against the caller's role on every call. The same methods serve `glave admin` and an admin MCP server.
- **The log is the single place to see everything:** usage, spend, activity and a thread's trace are views over it.
  - Admins see activity and cost, not content.
  - Reading content needs an organization policy that says so, and each read is recorded.
  - Still to decide: whether the person is told, and legal holds.

## The look

- **glave is monochrome by default.**
- **An organization's theme** sets its name, logo and one accent (#283). A person always chooses light or dark, and chooses an accent where the organization allows.
- **The assistant** has no name or icon unless an organization gives it one (#287).

## Where the code disagrees

1. **A server still keeps a little of the folder it started in:** its skills and commands reach every thread, `memory/*` acts in its space, and only its threads go to workers (`session.md`, "Where a thread works"). Otherwise every project owns its folder, and its settings, servers and hooks reach its threads alone; a request that names no space acts in the caller's Chats.
2. **`glave app` serves in the folder it starts in,** so that folder becomes a project, and a folder no key leads to becomes a project without asking (`storage.md`, "Spaces"). The CLI becomes a client of the person's server, and a new folder asks once. `glave` alone starts a chat in the terminal, one turn per line; it opens the browser instead.
3. **`glave memory` still opens the store itself** rather than asking the person's server, since `memory/*` acts in the server's own space (item 1); the CLI's other commands for tasks, schedules, threads and git hosts are clients of the server (`one-server.md`, 2c).
4. **Settings are read from files and environment variables,** model accounts are refused on an organization's server, and there is one cloud provider at a time (`policy.md`, `session.md` "Model accounts", `gateway.md`). Settings become data (#222, #289, #290).
5. **The gateway service is configured from its own environment** (`gateway.md`, "Surfaces"). It becomes a role of the server, reading the same data.
6. **There are no teams,** policy can't tell callers apart, and budgets live in one process's memory. Teams and grants fix all three.
7. **There is no admin API and no admin screens.** They come as described under "Admin".
8. **`ModelInfo.hope` exists and there is no theme.** The flag is removed (#289) and theming arrives (#283).
9. **A person on a server or cloud machine reaching it from a phone needs a sign-in that isn't a company identity provider.** Tasks already work on the person's one machine, with workers as the upgrade (`computers.md`, "Placement defaults").
10. **The web app works out a thread's items itself,** and finds an approval's sentence by parsing the summary the harness wrote, with the reason it asks folded into the same string, so that reason shows as if it were part of the command. The CLI words approvals its own way. Thread items come from the server as typed fields (#311).
