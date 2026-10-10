# Computers

Where work runs: the machines, the recipes they are made from, and the working copies threads act on. Tracks issue #29; the sandbox around commands is in `sandbox.md`.

## Status

First cut built (#116). The computer port (`Computers` in `glave-ports`) and its conformance suite; `glave-computer` with this machine as a computer (`LocalComputers`), `glave computer serve` (stdio, and TCP on 127.0.0.1 behind a token) and the client that reaches it (`RemoteComputers`), both passing the same suite; environments read from devcontainer.json or environment.toml. The harness still runs its tools in the folder glave was started in: moving its tools onto the port, `tool/run`, sleep, wake, resize, credentials, cloud computers, `workspace.bound` events and placement come next.

## The nouns

| Noun | Is | Lives |
| --- | --- | --- |
| **Computer** | a machine: a laptop, a microVM, a server, a pod | created, slept, woken and destroyed; disposable |
| **Environment** | the recipe a computer is made from | versioned; belongs to a space |
| **Workspace** | a working copy of a space's sources on a computer, where one thread's tools act | as long as its thread is open |

- One computer holds many workspaces. In the cloud it is usually one per person, holding that person's workspaces for one space at a time per sealed policy.
- A workspace lives on exactly one computer at a time, and has its own id: threads reference it, so a subagent shares its parent's.
- A computer never holds two organizations' work, and a sealed space's work only on computers dedicated to it.

## The rules

1. **The harness never runs where untrusted code runs, and never picks a machine.** It asks the computer port for "a workspace for this thread, in this space, from this environment"; placement happens behind the port, under policy (`policy.md`).
2. **Tools run on the computer, next to the files.** The same tool code runs in-process on a local computer and inside `glave computer serve` (the same binary) on a remote one, so tools behave identically everywhere. One conformance suite checks every computer adapter.
3. **Work moves between computers only through the sources' own history:** git for code (a `glave/<thread>` branch pushed and fetched), and for a plain folder, glave keeps a hidden git repository inside it. There is no other transfer mechanism.
4. **Every binding is an event.** `workspace.bound` (thread, workspace, computer, root, sources) when a thread first acts somewhere; `workspace.moved` (from, to) when it moves. The workspace root is recorded once; tool output and glave's own fields use paths relative to it.
5. **The agent is told where it is, every turn:** the computer, operating system, workspace root, and what came with a move ("dependencies need installing; the dev server is not running"). It never relies on memory for location.
6. **One writer.** A workspace has one writing thread. Moving a thread ends the old binding before the new one starts.
7. **Processes and memory are never promised.** A computer that sleeps or moves keeps files, not running programs.
8. **Secrets never enter a remote computer.** Git and cloud credentials reach it through a credential helper that asks the harness for each use; values are never written to its disk.

## The protocol

A computer adapter answers, over the same JSON-RPC as the session API (in-process locally, TLS remotely, authenticated by a short-lived token the harness side mints):

| Method | Does |
| --- | --- |
| `computer/create` | makes a computer from an environment; returns its id |
| `computer/status` | running, sleeping or gone, with its size |
| `computer/sleep`, `computer/wake` | frees compute and keeps the disk; brings it back |
| `computer/resize` | changes CPU and memory within policy (in place, or by sleeping and waking on a bigger host) |
| `computer/destroy` | removes it and its disk |
| `workspace/open` | checks out a space's sources (at a ref) as a new workspace; returns its id and root |
| `workspace/close` | removes a workspace |
| `tool/run` | runs one tool call in a workspace, streaming a command's output; the sandbox is a property of the call |
| `credentials/request` | the computer's credential helper asking the harness for a credential, which may refuse |

Coarse calls only: a search runs inside the computer and returns results, never a remote filesystem.

Built so far, beside `computer/create`, `computer/status`, `computer/destroy`, `workspace/open` and `workspace/close` (wire version `2026-10`):

| Method | Does |
| --- | --- |
| `server/discover` | the server's name, version and protocol versions |
| `connection/authenticate` | the first message on a network transport: the token; anything else ends the connection |
| `command/run` | runs a bash command at the workspace root, its output (standard output and error together) streamed as `command/output` notifications before the answer: exit code, signal, timeout or cancel, and how long it took |
| `command/cancel` | stops a running command and everything it started |
| `files/read`, `files/write`, `files/list` | one file read whole (at most 8 MiB, as base64), files written all-or-nothing after every path is checked, a folder listed (at most 10,000 entries); every path confined to the workspace, symlinks followed before the check |
| `workspace/save` | commits everything to a branch and pushes it to `origin`; a plain folder keeps the commit in a hidden history (`.glave/history.git`, up to 100 MB of files), never a `.git` |

A repository opens as a clone under `~/.glave/workspaces/<computer>/<workspace>`, removed on close; a folder opens in place and is kept. A computer on the host refuses what it cannot honour (an image, setup commands, a limited network) rather than ignoring it.

devcontainer.json honours `image`, `containerEnv` and `remoteEnv` (written values only, no `${...}`), `onCreateCommand`, `updateContentCommand` and `postCreateCommand` (as setup, in that order) and `hostRequirements` (cpus, memory, storage). It refuses by name what glave cannot carry out (building images, Compose, non-empty `features`, `initializeCommand`, `postStartCommand`, `postAttachCommand`, mounts, container options, GPUs, unknown keys), and lists the editor and preview settings it leaves alone (`customizations`, `forwardPorts` and the like). environment.toml refuses unknown keys.

## Environments

An environment is a file in the space, `.glave/environment.toml`, or the repository's `.devcontainer/devcontainer.json` when present (standards first):

```toml
image = "ghcr.io/acme/rust:1.99"     # an OCI image; "host" for the local machine
setup = "scripts/setup.sh"           # run once; its result is cached as a snapshot
cpus = 2
memory_gb = 8
disk_gb = 50
network = "allowlist"                # "off", "allowlist" or "on"
allow = ["crates.io", "github.com"]
secrets = ["GITHUB_TOKEN"]           # names only: values come through the credential helper
idle_minutes = 30
```

Sizes and counts are choices within policy ceilings (`policy.md`): the largest size, how many computers per person, burst budgets.

## Placement defaults

- **On a laptop:** the computer is the laptop. The thread the person is driving works in the project folder itself; background threads, goals and scheduled work get their own copy beside it (a git worktree, or a copy-on-write clone where the filesystem supports it), under `~/.glave/workspaces/`.
- **In the cloud:** one persistent computer per person by default, sleeping when idle; a space can be pinned to a computer of its own; short-lived burst computers start from a snapshot of the person's computer for parallel work, later.
- **Removal:** a copy is removed once its thread's changes have landed or the thread is archived, after a warning; reopening an archived thread checks its branch out again.
