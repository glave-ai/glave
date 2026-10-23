# The model

What glave is made of, from an organization down to a thread, and where each part runs. The other specs hold the detail; this one holds the shape they agree on. Decided with the owner on 2026-10-16 (issue #294); projects, words and powers, and connected apps on 2026-10-23 (#633).

## Status

Decided, mostly not built. "Where the code disagrees" lists what changes, in the order it lands.

## The hierarchy

| Level | Is | Holds |
| --- | --- | --- |
| Organization | The tenant: one on a laptop (never shown), one in a company, many on Glave Cloud | Model providers and their catalogs, its list of approved apps (`policy.md`, "Connected apps"), its directives and skills, environments, policy, the theme, the budget |
| Team | A flat group of people, mirrored from the identity provider's groups where there are some | What it is granted: models, apps, budgets, and whether its people may add their own apps |
| Person | Someone who signs in | Their Chats, their computer, their own sign-ins (git hosts, apps), their own instructions, skills and choices |
| Project | A named place for work: one folder. It belongs to the organization, is managed by a person or a team, and is private to its manager, shared with people or teams, or open to everyone in the organization | Its folder, whose files are its words; the apps, tool servers, hooks, settings and secrets accepted into it; its members, each one's chats and project memory in it their own (#631) |
| Thread | A conversation or task, in a project or in the person's Chats | Its events, and the project's folder or a worktree of it |

- **One rule at every level** (`policy.md`): choices, the nearest wins; constraints, they only narrow. A level grants only what it was given.
- **A project is one folder, and its files are its words:** the project's AGENTS.md and skills are that folder's ("Words and powers"). A thread may be given other folders to read or write ("Two roles", below; `sandbox.md`, #307), but they are access only: they bring no instructions, tools or settings. So what a thread reads and runs has one source, and nothing needs a rule for merging two folders' words; projects of several folders were dropped. A project with no folder works in scratch folders, as a chat does, with the organization's and the person's words alone.
- **No project needs a team.** Anyone can make one, private to them at first. The organization always owns its data, so when a person leaves, the projects they managed pass to an admin, and nothing is lost or orphaned.
- **Words:** people see Chats and projects, and the code says the same: Chats is the person's project of kind `personal` (`storage.md`). Spaces became projects on 2026-10-19 (#435); only stored names that sealed content depends on keep the old word.
- **Organization-first: a person on their own is an organization of one,** on a laptop, their own server or Glave Cloud's personal plan: no teams, one person. The organization's screens are that person's Settings, and the word "organization" never appears there (AGENTS.md, "Keeping principle 4"). glave's own defaults, its list of apps and its skills, reach that person the way an organization's reach its people, so there is one mechanism and one code path (`deployments.md`), and only the defaults differ.

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
  - **Read and written:** the project folder or the chat's scratch folder, temp folders, and any folder the person grants for a thread ("Allow writing to ~/Downloads?"), for access only (above).

  Approvals sit on top: a risky action asks even inside the writable zone. The zones bind the agent, never the person.
- **The machine's files live on a disk that's the person's;** the running machine is replaceable. It can be reset or moved without losing their files, and history is never on it.
- **Where glave runs the machine, its system comes from an image glave updates without asking;** the person's home (`/home/<name>`) is on their kept disk. What a person installs for the whole system goes on a setup list that runs again on each new image, so an update never loses it.
- **Fresh machines per task are an upgrade, not the default.** They're workers (`computers.md`, "Workers"), used for parallel or untrusted work where the machine can run them: Docker (rootless, or Podman, since a Docker socket is root on its host) or a company's Kubernetes. Code comes in by clone and leaves as a pushed `glave/` branch.
- **Isolating each project on its own machine** (a consultant keeping clients apart) is an option to add when someone needs it, never the default.
- **Not at launch:** browser sessions kept logged in on a machine. A logged-in browser driven by an agent that reads untrusted pages is the riskiest piece, and it comes later, with its own design.
- **Untrusted travels with work:** a thread an assistant starts after reading untrusted content inherits that label, so its risky actions need approval.
- **A person's files on a machine are personal data:** sealed under their key where the store keeps them, erased with it, and exportable.

## Words and powers

What shapes the agent comes in two kinds, kept two ways.

- **Words are files:** instructions (AGENTS.md) and skills, kept with what they belong to and read where they are:
  - a project's, in its folder, read in place (`session.md`, "What a thread opens with");
  - the person's, in `~/.glave`;
  - the organization's: a few directives and skills for everyone, delivered read-only to every machine and named after the organization (`acme:…`), so no project's or person's can shadow them (#223).

  A project's words travel with its work in git, and the tools people already use read AGENTS.md and skills as files (principle 7). Words are text the model reads, so on their own they grant nothing.
- **Powers live in glave:** connected apps, tool servers, hooks, settings (the model, approvals) and secrets, and with them model providers and accounts, policy, budgets, the theme and the assistant's name. Each has an owner (an organization, team, person or project), a permission and a record in the data store, and changes through the session API. A power runs commands or reaches data, so who may add it, who did and when are glave's to keep, never a file's.
- **A repository's powers are suggestions.** Its `.mcp.json`, its hooks and its settings that add power (`.glave/config.toml`) are never read live: the project's owner accepts each into the project, where it becomes a power with that record and reaches every member's threads (`policy.md`, "A repository's suggestions"). Opening a clone adds nothing, and every power has someone on record who said yes. A setting that adds no power applies straight from the repository, as today.
- **Files and environment variables remain** for bootstrapping (the store's address, the identity provider, the key service) and for applying settings kept in git (`glave admin apply`).
- **A setting is managed by a file or in the app, never both.** A file-managed setting shows read-only in the app.

## Models

- **Providers are added in the app.** Each provider's catalog is cached in the data store, refreshed when the provider is added, daily, and on demand (#290).
- **A person's own model accounts are an organization's setting:** on for a person on their own, off by default in a company, where the organization's providers serve everyone.
- **One entry per model, and glave picks the route** (#289): the organization's cloud contract first, then the model maker's API, then aggregators, each later one only as fallback. Policy can pin a route. Every call records where it ran.
- **The picker lists** the default, the person's recent picks, the most used in the organization, then everything else through search.

## Admin

- **Admins govern powers:** the list of approved apps and who may add their own (`policy.md`, "Connected apps"), models, budgets and policy; of words, only the organization's own directives and skills ("Words and powers").
- **The screens:** `/admin` in the same web app, reached from the person's menu and shown only when `server/discover` says the caller is an admin.
- **The methods:** admin methods are part of the session API, checked against the caller's role on every call. The same methods serve `glave admin` and an admin MCP server.
- **The log is the single place to see everything:** usage, spend, activity, every tool call and a thread's trace are views over it.
  - Admins see activity and cost, not content.
  - Reading content needs an organization policy that says so, and each read is recorded.
  - Still to decide: whether the person is told, and legal holds.

## The look

- **glave is monochrome by default.**
- **An organization's theme** sets its name, logo and one accent (#283). A person always chooses light or dark, and chooses an accent where the organization allows.
- **The assistant** has no name or icon unless an organization gives it one (#287).

## Where the code disagrees

1. **A server still keeps a little of the folder it started in:** its skills and commands reach every thread, and where its threads go to workers, only that folder's project's do (`session.md`, "Where a thread works"). Otherwise every project owns its folder, and its settings, servers and hooks reach its threads alone; a request that names no project acts in the caller's Chats.
2. **Settled** (`one-server.md`, 3): the CLI is a client of the person's server, a new folder asks once before it becomes a project, and `glave` alone opens the browser at the folder's project.
3. **Settled** (`one-server.md`, 2c): tasks and schedules run in the person's server, and the CLI's commands for tasks, schedules, threads, memory and git hosts are its clients.
4. **Settings are read from files and environment variables,** model accounts are refused on an organization's server, and there is one cloud provider at a time (`policy.md`, `session.md` "Model accounts", `gateway.md`). Settings become data (#222, #289, #290).
5. **The gateway service is configured from its own environment** (`gateway.md`, "Surfaces"). It becomes a role of the server, reading the same data.
6. **There are no teams,** policy can't tell callers apart, and budgets live in one process's memory. Teams and grants fix all three.
7. **There is no admin API and no admin screens.** They come as described under "Admin".
8. **There is no theme.** Theming arrives (#283).
9. **A person on a server or cloud machine reaching it from a phone needs a sign-in that isn't a company identity provider.** Tasks already work on the person's one machine, with workers as the upgrade (`computers.md`, "Placement defaults").
10. **The web app works out a thread's items itself,** and finds an approval's sentence by parsing the summary the harness wrote, with the reason it asks folded into the same string, so that reason shows as if it were part of the command. The CLI words approvals its own way. Thread items come from the server as typed fields (#311).
11. **A repository's tool servers, hooks and settings are read live** from its folder (`.mcp.json`, `.glave/config.toml`) once the person on this computer trusts the project, and the person's own from their files (`policy.md`, "Project trust"). They become powers in glave, a repository's as suggestions the project's owner accepts ("Words and powers"), in a follow-up issue.
12. **Apps are a policy file's list:** `[mcp] catalog`, and `[mcp] own`, one answer for everyone the policy covers, since there are no groups to grant it to (item 6). glave ships no list or skills of its own for an organization of one; people add tool servers by editing `mcp.json`, not on a Connect page signed in as themselves (#42); knowledge bases are not apps yet (#144); and there are no organization directives or skills (#223).
