# The model

What glave is made of, from an organization down to a thread, and where each part runs. The other specs hold the detail; this one holds the shape they agree on. Decided with the owner on 2026-10-16 (issue #294); projects, configuration and connections on 2026-10-23 (#633).

## Status

Decided, mostly not built. "Where the code disagrees" lists what changes, in the order it lands.

## The hierarchy

| Level | Is | Holds |
| --- | --- | --- |
| Organization | The tenant: one on a laptop (never shown), one in a company, many on Glave Cloud | Model providers and their catalogs, its policy file (its approved connections among it, `policy.md`), company skills, environments, the theme, the budget |
| Team | A flat group of people, mirrored from the identity provider's groups where there are some | What it is granted on top of its people's roles (#645): models, connections, a budget ("Configuration is files"), and "may add their own" |
| Person | Someone who signs in, with a role in the organization ("Admin") | Their Chats and projects, their computer, their own files (`~/.glave`), their own sign-ins (git hosts, connections) |
| Project | One person's named place for work: one folder. It belongs to the organization and to that person alone | Its folder, whose files are its instructions, skills and setup; its threads; its memory, which is the person's |
| Thread | A conversation or task, in a project or in the person's Chats | Its events, and the project's folder or a worktree of it |

- **One rule at every level** (`policy.md`): choices, the nearest wins; constraints, they only narrow. A level grants only what it was given.
- **A connection** is a service the agent can use for a person through a tool server (MCP): Gmail, Linear, GovWin, Google Drive. How they are approved, added and signed in to is `policy.md`, "Connections".
- **Projects are personal:** no shared projects and no members. People work together through the repository (git) and the organization (its policy, its skills, its groups), which already share code and setup, so glave needs no second way to share. A project is one folder; other folders a thread is given ("Two roles", below) are access only and bring no instructions, tools or settings. The organization always owns its data, so when a person leaves, their projects pass to an admin, and nothing is lost or orphaned.
- **Words:** people see Chats and projects, and the code says the same: Chats is the person's project of kind `personal` (`storage.md`). Spaces became projects on 2026-10-19 (#435); only stored names that sealed content depends on keep the old word.
- **Organization-first: a person on their own is an organization of one,** on a laptop, their own server or Glave Cloud's personal plan: no teams, one person. The organization's screens are that person's Settings, and the word "organization" never appears there (AGENTS.md, "Keeping principle 4"). The same rules hold as in a company, through the same mechanism and one code path (`deployments.md`); only the defaults differ.

## Two roles: the server and the computer

- **The server** holds history, projects, sign-ins and keys, reads the configuration files ("Configuration is files"), runs the harness and the gateway, and answers every client: the web app, the CLI and outside agents. **The computer** holds files and runs commands. They talk through the computer protocol (`computers.md`): in-process on a laptop, over TLS elsewhere.
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

  Approvals sit on top: a risky action asks even inside the writable zone. The zones bind the agent, never the person. What the agent may do is one choice, its mode, per chat (`approvals.md`, "Modes"): Auto (the zones above, the default), Ask first (the project read, never written, without the person's yes) or Full access (no zones, and nothing asks, for work the person fully trusts); an organization caps the highest.
- **The machine's files live on a disk that's the person's;** the running machine is replaceable. It can be reset or moved without losing their files, and history is never on it.
- **Where glave runs the machine, its system comes from an image glave updates without asking;** the person's home (`/home/<name>`) is on their kept disk. What a person installs for the whole system goes on a setup list that runs again on each new image, so an update never loses it.
- **Fresh machines per task are an upgrade, not the default.** They're workers (`computers.md`, "Workers"), used for parallel or untrusted work where the machine can run them: Docker (rootless, or Podman, since a Docker socket is root on its host) or a company's Kubernetes. Code comes in by clone and leaves as a pushed `glave/` branch.
- **Isolating each project on its own machine** (a consultant keeping clients apart) is an option to add when someone needs it, never the default.
- **Not at launch:** browser sessions kept logged in on a machine. A logged-in browser driven by an agent that reads untrusted pages is the riskiest piece, and it comes later, with its own design.
- **Untrusted travels with work:** a thread an assistant starts after reading untrusted content inherits that label, so its risky actions need approval.
- **A person's files on a machine are personal data:** sealed under their key where the store keeps them, erased with it, and exportable.

## Configuration is files

Everything that configures glave is a file, read where it lives, and glave holds only what can't be a file. Files are what people and tools already write, review and keep in git (principle 7), and the agent changes them as it changes any other.

- **The repository's**, read in place, as today: AGENTS.md, skills (`.agents/skills`, `.glave/skills`, `.claude/skills`), `.mcp.json` and `.glave/config.toml`. What runs from them asks first (`policy.md`, "Project trust").
- **The person's own,** in `~/.glave` (AGENTS.md, skills, `mcp.json`, `config.toml`), with `~/.agents/skills` and `~/.claude/skills`.
- **The organization's:**
  - its policy file (`policy.md`): its approved connections, the models allowed, the network default, who may add their own connections, and the budget: one dollar budget per person per month, covering all AI use; groups set it, the larger applies (#662);
  - company skills in one fixed folder, `/etc/glave/skills`, as Codex keeps its own: baked read-only into the images glave runs, and put there on people's own machines by IT or a setup step.
- **glave's built-in skills** ship with glave.
- **Skills keep their names apart.** Company skills show as `acme:name` and glave's as `glave:name`; the person's and the repository's have no prefix. So a same-named skill never replaces a company's or glave's, and a policy can turn the person's own skills off.
- **glave's settings screens never change a repository's files;** they change only the person's own settings (Connect, their preferences, their instructions). A repository's setup is shown read-only: change it like code, or ask the agent. The agent still edits a repository's files when the person asks, as any other change, and shows the diff.
- **glave holds only what can't be a file:**
  - secret values and sign-ins, which files name (`${NAME}`, or a connection's name) and never hold;
  - people, groups and roles ("Admin");
  - memory;
  - the record of what happened (the log).
- **Secrets** (#267): their scopes are the person's and the organization's, the organization's granted to groups, and no project shares one. The machine holds only a placeholder, which the outbound proxy fills in for the hosts a secret is allowed to reach (#643).

## Models

- **Providers are added in the app.** Each provider's catalog is cached in the data store, refreshed when the provider is added, daily, and on demand (#290).
- **A person's own model accounts are an organization's setting:** on for a person on their own, off by default in a company, where the organization's providers serve everyone.
- **One entry per model, and glave picks the route** (#289): the organization's cloud contract first, then the model maker's API, then aggregators, each later one only as fallback. Policy can pin a route. Every call records where it ran.
- **The picker lists** the default, the person's recent picks, the most used in the organization, then everything else through search.

## Admin

- **Roles** (#645): an Owner can do everything, billing and transferring ownership included; an Admin manages people, policy, the approved connections, models, the budget and audit; a Member uses glave within what is granted. Groups from the identity provider add grants on top (the Team row). A person on their own is silently the owner of their organization of one.
- **The screens:** `/admin` in the same web app, reached from the person's menu and shown only when `server/discover` says the caller is an admin. They come later (#222), and they only edit the organization's files: its policy and its skills ("Configuration is files").
- **The methods:** admin methods are part of the session API, checked against the caller's role on every call. The same methods serve `glave admin` and an admin MCP server.
- **The log is the single place to see everything:** usage, spend, activity, every tool call and a thread's trace are views over it.
  - Admins see activity and cost, not content.
  - Reading content needs an organization policy that says so, and each read is recorded.
  - Still to decide: whether the person is told, and legal holds.

## The look

- **glave is monochrome by default.**
- **A person** chooses light or dark, an accent colour, and a name for their assistant, each unless their organization fixes it.
- **An organization's theme** sets its name, logo and accent, and its assistant's name and icon, and either lets people personalize the accent and the assistant's name or fixes them (#283, #287).
- **The assistant is called Hope by default,** with fine text saying it's the person's assistant.
- **A person on their own** sets all of it for themselves.

## Where the code disagrees

What today's code doesn't do yet, so no one takes it as built.

1. The folder the server was started in still gives every conversation its skills and commands, and only that folder's project can send its work to fresh machines (`session.md`, "Where a thread works"). Everywhere else, a project's settings and tools reach only its own conversations.
2. A company's server turns away people's own model accounts, and glave uses one cloud provider at a time. Using several, with glave picking the route for each model, comes with #289 and #290.
3. The gateway, when it runs as a service of its own, takes its settings from its own environment rather than the server's files (`gateway.md`, "Surfaces"). It will become part of the server.
4. There are no roles or groups yet, so everyone in a company gets the same access, and spending limits reset when the server restarts (#645, #662).
5. There are no admin screens or admin methods yet; they come as "Admin" describes (#222).
6. There is no theme yet: no organization name, logo or accent, and no choosing an accent or the assistant's name (#283, #287).
7. On Glave Cloud there is no way yet to sign in without a company's identity provider, as from a phone. On a person's own server a passkey already works (`deployments.md`, "Sign-in").
8. The web app works out what happened in a conversation by itself, and reads an approval's question out of a summary, so the reason glave asks can look like part of the command; the terminal words approvals its own way. The server will send these as separate facts (#311).
9. There's no Connect page yet: adding a service means editing a file, and it can't sign in as each person (#42).
10. Skills come only from the repository and the person's own folders, and one can replace another of the same name. There are no company skills in `/etc/glave/skills`, no skills built into glave, no `acme:` or `glave:` names, and no way for a policy to turn personal skills off.
11. A company's policy file can take the agent's network away (#641), but nothing writes it for one yet: a company starts with the network on until its policy says otherwise, and Glave Cloud's starting policy comes with the admin screens (#222).
12. Where no sandbox can run (Windows, and a person's cloud computer until it has one inside it, #221), no zones are drawn: Auto and Ask first ask before every change and command instead, and only Full access runs without asking. An organization that caps the modes below Full access gets no glave there at all: it refuses to start.
