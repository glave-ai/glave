# The model

What glave is made of, from an organization down to a thread, and where each part runs. The other specs hold the detail; this one holds the shape they agree on. Decided with the owner on 2026-10-16 (issue #294); projects, configuration and connections on 2026-10-23 (#633).

## Status

Decided, mostly not built. "Where the code disagrees" lists what changes, in the order it lands.

## The hierarchy

| Level | Is | Holds |
| --- | --- | --- |
| Organization | The tenant: one on a laptop (never shown), one in a company, many on Glave Cloud | Model providers and their catalogs, its policy file (its approved connections among it, `policy.md`), company skills, environments, the theme, the budget |
| Team | A flat group of people, made in glave by its admins ("Identity") | What it is granted on top of its people's roles (#645): models, connections, a budget ("Configuration is files"), and "may add their own" |
| Person | Someone who signs in, with a role in each organization they belong to ("Identity") | Their Chats and projects, their computer, their own files (`~/.glave`), their own sign-ins (git hosts, connections) |
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

- **Roles** (#645): an Owner can do everything, billing and transferring ownership included; an Admin manages people, policy, the approved connections, models, the budget and audit; a Member uses glave within what is granted. Groups from the identity provider add grants on top (the Team row). A person on their own is silently the owner of their organization of one. How roles are kept and checked is "Roles", below.
- **The screens:** `/admin` in the same web app, reached from the person's menu and shown only when `server/discover` says the caller is an admin. They come later (#222), and they only edit the organization's files: its policy and its skills ("Configuration is files").
- **The methods:** admin methods are part of the session API, checked against the caller's role on every call. The same methods serve `glave admin` and an admin MCP server.
- **The log is the single place to see everything:** usage, spend, activity, every tool call and a thread's trace are views over it.
  - Admins see activity and cost, not content.
  - Reading content needs an organization policy that says so, and each read is recorded.
  - Still to decide: whether the person is told, and legal holds.

### Identity

Decided with the owner 2026-10-29 (#423, #424). Who the people of an organization are, how they sign in, and how one comes to be.

- **Every glave has an organization.** On a laptop and a person's own server it is an organization of one, the local person's, and never shown. Organization UI appears only when it holds more than one person or a company login is configured, and the server says so (`server/discover`, `exists.organization`, worked out per caller: `session.md`, "Signing in"). There is no case of "no organization": the methods that need one find the caller's, and what can't happen for an organization of one is refused for that reason, in a sentence that says it.
- **One identity source per organization:** glave's own accounts, or the company's login (OIDC today, SAML later), never both. A self-hosted server has one organization, so one source per deployment. Holding the server's computer is not an identity source: the local person signs in there as today (`session.md`, "Signing in") whichever source the organization has.
- **glave accounts are passwordless.** A person signs in with a passkey, and may have several (one per device or passkey manager). Their email is for recovery and invites only. Recovery is a one-time link emailed to them that expires in minutes and leads to making a new passkey; their other signed-in browsers are then told, by the server and in its words, that a new passkey was added, so a stolen mailbox does not go unseen. No passwords anywhere, and no sign-in by an emailed code.
  - *Why not email codes or magic links for sign-in:* a mailbox becomes the key to every sign-in, and links and codes are phishable; a passkey is not. Email stays the way back in, and the notice to the other devices is the check on it.
- **A company login lets its people in.** Anyone the company's login lets into glave joins its organization as a Member on first sign-in (`person.joined`, "Roles"); admins set roles and groups in glave. For a broad issuer (Google, Microsoft's common tenant), the organization limits who may enter to its allowed email domains, or lets in only those with an invite. Someone removed is not let back in by signing in again: only an invite brings them back.
- **The same model for every organization:** people with a role (Owner, Admin, Member, "Roles"), groups made in glave, and grants per group (models, apps, a budget). Copying groups from the login service, SCIM and custom roles come later, as an enterprise tier (#642, #645).
- **Invites work the same for both sources** ("Invites"): the link makes a passkey, for glave accounts, or sends the person to the company login.
- **Remove is one admin action** that takes a person out of the organization and ends, at once, their sessions in it and every watch they hold there, as a browser that signs out does (`session.md`, "Watching", #918). Their projects stay the organization's ("The hierarchy").
- **Sign-up, on a server whose configuration offers it (Glave Cloud: `[serve] signup`, beside a public origin, `[email]` and no company login), creates a new organization with the person as its Owner.** They give an email, open the one-time link sent to it (good once, for 15 minutes), and make their first passkey; that makes their account, the organization, and its first Owner (`person.joined`, then `role.set` by glave, `storage.md`, "Many organizations in one store"), and keeps the email as the account's for getting back in: in the directory as its SHA-256, and as the person's in their organization. Until the link is opened no address is kept: the sign-ups ledger keeps the SHA-256 of its code and of the address, and the link carries the address, which its code's hash covers. An address that has an account is sent, through the same steps, a note to sign in instead, so the answer, and how long it takes, say nothing of who is here. An address is read one way (lowercase, no name around it), so a mailbox gets one account. The new organization's turns use the server's own models until Glave Cloud's credits come; the Owner can't connect accounts of their own from elsewhere yet (`session.md`, "Model accounts"): *gap*. Off unless the server's own file turns it on, so a person's own server takes no strangers. Glave Cloud's credits are a later issue.
- **One person may belong to several organizations,** with one sign-in, as in Slack: each request carries the organization it works in, and everything else stays apart, each organization's data, roles and grants its own. A person's account (their passkeys, email and browser sessions) is theirs, kept outside every organization, so leaving or being removed from one never signs them out of another.
- **Email is a plug, at most one message a minute to an address,** counted in the ledger that records what was sent (an invite's in the people ledger, a sign-up's in the sign-ups ledger), so every server on the store counts alike; limits per day and per network address are still to come. SMTP, its settings in the deployment's configuration and its password from the environment, with an in-memory double for tests. A deployment without it sends no email: recovery and sign-up are not offered there, and invites are shared as links.
- **The five deployments** (`deployments.md`):
  1. *Laptop:* an organization of one, never shown. No one else can reach the server, so nothing to invite anyone to.
  2. *Own server:* the same, until its owner invites someone, who makes a passkey of their own at its public origin; with two people, organization UI appears for them.
  3. *Glave Cloud, personal:* sign-up makes an organization of one with glave accounts; inviting someone makes it more.
  4. *Glave Cloud, enterprise:* sign-up, then the organization's company login; its people join on first sign-in.
  5. *Enterprise, self-hosted:* their company login, the server's one organization; the operator is its Owner from the server's computer ("Roles").

### Roles

Decided 2026-10-29 (#645).

- **Kept in the data store, one ledger per organization:** the people ledger, a stream of glave's own in the organization's scope rather than any person's project, sealed like every event. It records each person who joins (`person.joined`: through the company login on first sign-in, by an invite, or by signing up; for a company login its issuer and subject, and the name, email and groups the token gives, recorded again when they change), each person removed (`person.removed`) and each role given (`role.set`: whom, which role, by whom). Folded in order, a person's latest `role.set` is their role; one with none is a Member. One stream keeps listing people one read, and every server process on one store reads the same roles. Roles are data, not a file, because they change from the admin screens at run time and name people by the identity provider's ids ("Configuration is files").
- **The local person is the organization's owner, always,** and is never recorded, so no one can take it away and an organization is never without an owner. The local person is whoever holds the server's computer: its socket, or its local token from there (`session.md`, "Signing in"). They already hold the store's keys and the organization's files, so being owner gives them nothing they lack. On a laptop and a person's own server they are the only person, so they are silently owner and the word never appears. Where people sign in through an identity provider, the one who runs the server gives the first roles from there, to people who have signed in once (`glave admin`, below).
  - *Why not the first person to sign in:* behind a broad issuer (Google, Microsoft's common tenant) the first to sign in could be anyone, and two servers starting at once race for it.
  - *Why not a setting naming the owner:* the identity provider's subject is an opaque id no one knows before signing in, an email in a token is not always verified, and it would be a setting a laptop never needs.
- **Three fixed roles at launch; custom roles can come later without a rewrite.** A role is data, a name and the set of things it permits, and every admin method checks what the caller's role permits, never its name. Today the three roles' sets are fixed in glave; custom roles, and any screen for them, wait until asked (#645).
- **Who may give which role:** Owners and Admins give Admin and Member; only an Owner makes someone an Owner, or changes an Owner's role. An organization may have several Owners, and transferring ownership is making another Owner, then being made Admin by them.
- **Groups are made in glave** ("Identity"): an admin makes a group, puts people in it and grants it models, apps and a budget, the same in every organization. The company login's `groups` claim, a list of names as Okta, Keycloak, Entra ID and Auth0 send it, is kept on the person's record at each sign-in where it changed, and grants nothing; copying groups from it or from SCIM (#642) comes later, as an enterprise tier. `server/discover` says groups exist only once groups grant something.
- **Every admin method is checked on every call,** in the session's one check before anything is read or recorded (`session.md`, "Projects and who may do what"): the caller's role, read from the people ledger, must be Admin or Owner. Anyone else is refused with `-32012` and a plain sentence that only the organization's admins can do that, and nothing is recorded. The first methods are listing the people and setting a role; the screens (#222), `glave admin` and an admin MCP server all use them.
- **What a client is told:** `server/discover`, asked by someone signed in on a server with an organization (`exists.organization`), says the caller's role, so the app shows `/admin` only to Admins and Owners. On a server without one it says no role, so a laptop's answer is unchanged.
- **The five deployments** (`deployments.md`):
  1. *Laptop:* one person, the local person, owner and never told so. Nothing is recorded.
  2. *Own server:* the same. A passkey signs in the local person, so its browser is the owner too.
  3. *Glave Cloud, personal:* the person who signs up is the new organization's Owner, recorded as `role.set` by glave, the same rule as the local person who makes a server's ("Identity").
  4. *Glave Cloud, enterprise:* the person who signs the organization up is its first Owner, and gives the rest roles from the admin screens; its people join through its company login.
  5. *Enterprise, self-hosted:* whoever runs the server is owner, from its computer, and gives the first Admins from there to people who have signed in. Groups come from the token now, and from SCIM with #642.

### Invites

Decided 2026-10-29 (#424).

- **An invite is a one-time link that gives a role.** Someone whose role permits inviting (Admins and Owners) makes one with a role, Admin or Member; only an Owner invites an Owner. The server answers the invite and its code once; the link carries the code in the fragment, `<origin>/#invite=<code>`, as a sign-in link does (`session.md`, "Signing in"), so it reaches no server log.
- **Whoever opens it signs in, then accepts it,** the same for both identity sources ("Identity"). Where the organization has a company login, the link sends them to it, and accepting the invite records them as joined with its role. Where it has glave accounts, the link makes a passkey: a new account, or one more organization for someone already signed in, joined with its role. An invite may name an email, and where the deployment sends email, glave sends the link there and keeps the address with the invite: the admins see whom it waits for, and someone who joins with it joins with that email; otherwise the admin shares it where they choose. A role is only ever raised by an invite, never lowered: an Owner who opens a Member's invite stays an Owner.
- **The code is the same primitive as a sign-in link's** (#383): 32 random bytes, good once, kept only as its SHA-256 and compared in constant time, after the id of the organization it is to, so a server serving many reads one organization's ledger to find it; that ledger is read apart from the ones the server keeps, so a made-up id fills nothing. Invites live in the organization's people ledger, sealed with it (`invite.created`, `invite.accepted`, `invite.revoked`, `events.md`), so they are scoped to the organization by the store: another organization's code finds nothing here. Every server on one store sees the same invites, and two opening the same code at once let only one through.
- **An invite expires after 7 days.** It goes to a person through a chat or an email, who may not open it the same day or until after a weekend; minutes, a sign-in link's lifetime, would expire most invites before they are opened. Longer leaves a link forgotten in a channel working for weeks. A used, revoked, expired or guessed code is refused alike, with the same sentence, and so is another organization's for someone its company login signs in; a code is its holder's to see and, with a glave account, to join its organization with.
- **An invite gives no more than its maker may give when it is opened.** An Admin who is made a Member, or an Owner made an Admin, leaves their invites unable to give what they no longer could.
- **Invites are listed and revoked** by anyone whose role permits inviting: the ones waiting, with their role, who made them and when they expire, never their code. At most 1,000 wait at once.
- **The five deployments** (`deployments.md`):
  1. *Laptop:* no one else can reach the server, so an invite is refused, saying so; nothing appears.
  2. *Own server:* where it is reached from elsewhere (its public origin), the owner invites someone, who makes a passkey there.
  3. *Glave Cloud, personal:* invites with glave accounts, as 2; someone already signed in gains one more organization.
  4. *Glave Cloud, enterprise:* invites through the company login, as 5.
  5. *Enterprise, self-hosted:* invites through the identity provider. Anyone the provider lets in can still sign in as a Member without one; an invite is how someone arrives as an Admin, or before they have signed in once.

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
4. Roles are kept and checked (#645), but nothing grants from groups yet: the identity provider's groups are recorded and give no models, connections or budget, so every Member gets the same access, and the only spending limit is each person's per day, kept by the gateway when it runs as a service of its own (`gateway.md`, "Budgets"); the monthly budget groups set waits for them (#645, #662).
5. The admin methods are listing people, setting a role, removing someone and inviting (`session.md`, "The organization's people"); invites are emailed only where `[email]` names a mail server, and no screen opens a link yet. Of the admin screens (#222), Admin › People is built: the people and their roles, removing someone, inviting and the invites waiting; the organization's policy and skills have none yet, and there is no `glave admin`, so where people sign in through an identity provider, the first roles are given from the server's computer through the session API, or its browser signed in there.
6. There is no theme yet: no organization name, logo or accent, and no choosing an accent or the assistant's name (#283, #287).
7. Of "Identity", what is not built yet, each line edited by the pull request that builds it:
   - The daily sweep of blobs runs for the server's own organization alone, and no client names another organization yet (#423).
   - Of glave accounts for more than the local person: email recovery and the new-passkey notice (#424). Built: the email plug (SMTP, `[email]`, sending invites), an invited person's passkey and account, several passkeys each, sign-in with any of them.
   - Groups made in glave and their grants, and a broad issuer's allowed domains (#645).
8. The web app works out what happened in a conversation by itself, and reads an approval's question out of a summary, so the reason glave asks can look like part of the command; the terminal words approvals its own way. The server will send these as separate facts (#311).
9. There's no Connect page yet: adding a service means editing a file, and it can't sign in as each person (#42).
10. Skills come from the repository, the person's own folders and glave itself (`glave:spreadsheets`), and a folder's can replace another's of the same name. There are no company skills in `/etc/glave/skills`, no `acme:` names, and no way for a policy to turn personal skills off.
11. A company's policy file can take the agent's network away (#641), but nothing writes it for one yet: a company starts with the network on until its policy says otherwise, and Glave Cloud's starting policy comes with the admin screens (#222).
12. Where no sandbox can run (Windows, and a person's cloud computer until it has one inside it, #221), no zones are drawn: Auto and Ask first ask before every change and command instead, and only Full access runs without asking. An organization that caps the modes below Full access gets no glave there at all: it refuses to start.
