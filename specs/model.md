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
- **On a laptop, the computer is the laptop:** the person's own, persistent by nature. A project's folder is worked in where it is, with one thread changing its files at a time.
- **In the cloud, every thread gets a fresh computer:** a worker made from the project's environment, released when the thread goes quiet, and made again from the thread's branch when it continues (`computers.md`, "Workers"). This is the industry's default for agents: Codex cloud, Claude Code on the web, Cursor's background agents, GitHub's Copilot agent. Nothing lasting lives on a cloud computer, so there is nothing to wreck, to reset or to leave behind, and threads run side by side without waiting on each other.
  - Code comes in by clone and leaves as the thread's pushed `glave/` branch; unpushed work is saved to `glave/wip/` when the computer is released.
  - Files that aren't code (uploads, documents, results to download) are the project's files, kept on the server.
  - A project's prepared setup (installed dependencies, built tools) is cached after the first run, so a new thread starts in seconds.
  - A long-lived personal machine in the cloud is a different product, and an environment option later if an organization asks.
- **A project's folder** is a path on a laptop, or a repository: cloned once on a laptop, or into each cloud thread's computer. A chat works in a scratch folder.

## Settings are data

- **Everything an organization, team, person or project sets lives in the data store** with its owner, and is changed through the session API: model providers and accounts, connections, policy, budgets, the theme, the assistant's name and instructions.
- **Files and environment variables remain** for bootstrapping (the store's address, the identity provider, the key service) and for applying settings kept in git (`glave admin apply`).
- **A setting is managed by a file or in the app, never both.** A file-managed setting shows read-only in the app.

## Models

- **Providers are added in the app.** Each provider's catalog is cached in the data store, refreshed when the provider is added, daily, and on demand (#290).
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

1. **A server serves one folder,** its own space's, and every other space's threads get scratch folders (`session.md`, "Where a thread works"). Projects own folders instead.
2. **`glave app` serves in the folder it starts in,** so that folder becomes a project, and a folder no key leads to becomes a project without asking (`storage.md`, "Spaces"). The CLI becomes a client of the person's server, and a new folder asks once.
3. **The CLI's task host is a second server per folder** (`glave serve --socket`, `session.md`, "Tasks and schedules"). Tasks and schedules run in the server.
4. **Settings are read from files and environment variables,** model accounts are refused on an organization's server, and there is one cloud provider at a time (`policy.md`, `session.md` "Model accounts", `gateway.md`). Settings become data (#222, #289, #290).
5. **The gateway service is configured from its own environment** (`gateway.md`, "Surfaces"). It becomes a role of the server, reading the same data.
6. **There are no teams,** policy can't tell callers apart, and budgets live in one process's memory. Teams and grants fix all three.
7. **There is no admin API and no admin screens.** They come as described under "Admin".
8. **`ModelInfo.hope` exists and there is no theme.** The flag is removed (#289) and theming arrives (#283).
9. **Cloud computers are fresh per thread, but four pieces are missing:**
   - a project's setup is not cached;
   - a computer is kept until its session ends, not released when the thread goes quiet;
   - a continued thread does not get a new computer from its branch;
   - files that aren't code have no home on the server.
