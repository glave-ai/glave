# Sandbox and approval modes

How glave decides whether an action may happen, and what an action can physically reach. Tracks issue #7.

## Status

Draft. Nothing here is built yet. Phases are at the end; each lands as its own pull request with live tasks that fail before it and pass after.

## Two settings, not one

Every action the model asks for meets two independent questions:

1. **Approval mode: should glave ask the person first?** Judgment, the person's or glave's.
2. **Sandbox: is the action possible at all?** Walls the operating system enforces, whatever anyone decided.

Approvals rely on judgment, and judgment fails: people click "yes" without reading, and models are talked into things by text they read. The sandbox cannot be talked out of anything. Each covers the other's weakness: the sandbox bounds what can go wrong, and approvals handle the gray area inside it.

### What each part protects against

The likeliest harm is not a clever escape but destruction inside the project: `rm -rf src`, `git checkout .` or `git reset --hard` throwing away uncommitted work, `git clean -fdx` deleting untracked files. The sandbox allows all of these: they are writes inside the project.

| Harm | Protection |
| --- | --- |
| Damage outside the project (`rm -rf ~`, shell rc files, other repositories) | the sandbox |
| Code planted to run later (git hooks, agent settings) | protected paths |
| Secrets sent to the model or the network | the secret rules and the sandbox's read and network limits |
| Destruction inside the project | **undo that covers commands**, not just glave's own edits |
| Anything else that needs judgment | the approval mode |

Lists of dangerous commands (`rm -rf`, `git reset --hard`) are a speed bump, not a protection: `python -c "shutil.rmtree(...)"`, `find -delete` and scripts slip past them. The protection is a snapshot to undo from.

### Approval modes

| Mode | Writes and edits in the project | Commands | Leaving the sandbox | Secret files |
| --- | --- | --- | --- | --- |
| `ask` | ask | ask | ask | ask |
| `edits` | allowed | ask | ask | ask |
| `auto` | allowed | allowed, inside the sandbox | ask | ask |
| `unattended` | allowed | allowed, inside the sandbox | **declined** | **declined** |

`read`, `search` and `glob` never ask, except for secret files (see [reads](#reads)).

`unattended` means no person is there to ask, so anything that would need one is declined. With a sandbox it is a real wall, which is what scripts, CI and the live suite need. `--yes` is `unattended`. It is deliberately not called "bypass": Claude Code's `bypassPermissions` runs sandbox escapes without asking, which is the opposite. Turning everything off is `unattended` with sandbox `full`.

### Sandbox levels

| Level | Writes | Network | Reads |
| --- | --- | --- | --- |
| `read-only` | the thread's temp folder only | off | everything except secret locations |
| `project` | the project and the thread's temp folder, except protected paths | off | everything except secret locations |
| `full` | anywhere the person can | on | everything the person can |

The sandbox applies to commands (`bash`). The file tools are confined to the project in glave's own code and refuse the same protected paths (below), so both routes obey one list.

### Defaults

| Situation | Approval | Sandbox |
| --- | --- | --- |
| Interactive | `edits` | `project` |
| `--yes`, scripts, CI, the live suite | `unattended` | `project` |
| No OS sandbox on this computer | `ask` | `full`, and the banner says so |

`edits` until undo covers commands: in `edits` every command asks, so the person sees `rm -rf` before it runs. `auto` becomes the default once undo covers commands and the sandbox exists, since together they make the worst case "undo that turn". (Codex starts version-controlled folders in its equivalent of `auto` and other folders read-only.)

### Undo, in brief

Its own design document will follow; the parts that shape this one:

- Before each command, glave saves what git does not already have: uncommitted changes and untracked files that are not ignored. Committed files are already in git, which glave only reads. Outside git, the first command in a thread saves the project once, then only files whose size or modification time changed. The cost is the size of the uncommitted work, stored in glave's own store, never in the project or `.git`.
- Undo restores the project as it was before a turn, including what commands deleted. It shows what it will do first, merges around anything the person changed since, and says what it cannot reach: ignored and very large files, and effects of commands that left the sandbox.

Without a working sandbox, `auto` would mean "run any command without asking", so glave falls back to asking. It never silently runs unconfined.

## The project sandbox in detail

### Writes

Allowed: the project folder, a temp folder created for the thread (commands see it as `TMPDIR`), `/dev/null` and terminal devices. Everything else is denied, including the shared `/tmp` and the person's home folder.

**Protected paths** stay read-only even inside the project, because writing them makes code run later, outside the sandbox, without showing up in `git diff`:

- All of `.git`, including a `.git` pointer file and the folder it points to. Hooks, config (`core.fsmonitor`, `core.hooksPath`, filter drivers), worktree config: any of them runs code at the person's next git command. Committing therefore needs to leave the sandbox. Codex does the same.
- `HEAD`, `objects` and `refs` at the project root, which would turn the folder into a bare repository with its own config.
- `.glave/`, **even before it exists**: glave's own project settings. (A 2026 CVE created a missing agent settings file from inside a sandbox to plant hooks.)
- Files that tools run commands from: `.mcp.json`, `.vscode/`, `.idea/`, `.envrc`.

The rule: **the sandbox may change the project's files, never the machinery that runs code on them.**

Writable roots come from the session's settings, never from a tool argument. (CVE-2025-59532 let a model-supplied working directory become the writable root.)

### Reads

Allowed everywhere except secret locations. This goes further than Codex and Claude Code, which both let sandboxed commands read `~/.ssh`.

- In the home folder: `~/.ssh`, `~/.aws`, `~/.azure`, `~/.config/gcloud`, `~/.kube`, `~/.docker/config.json`, `~/.gnupg`, `~/.netrc`, `~/.npmrc`, `~/.pypirc`, `~/.git-credentials`, `~/Library/Keychains`, and browser profiles (cookies are credentials).
- glave's own data: `~/.glave` (or `$GLAVE_HOME`). A command must not read or rewrite the event log.
- In the project: files named like secrets, the same rule `read` uses (`.env` and its variants but not templates, private keys, credential files).

The environment commands start with has no secret-named variables (already shipped).

### Network

Off, including Unix sockets: `docker.sock` is root on the machine, and the ssh agent signs with the person's keys. Package installs and downloads fail inside the sandbox; the model asks to leave it. A per-domain allowlist (package registries) needs a filtering proxy, as Claude Code's has, and comes later. Such a proxy must refuse allowed names that resolve to loopback or link-local addresses (`169.254.169.254`).

### Leaving the sandbox

`bash` takes two optional parameters: `outside_sandbox: true` and `reason`. Under `ask`, `edits` and `auto`, glave asks the person, showing the command and the reason. Under `unattended` the request is declined. The model is told to use it only for what the sandbox blocks: network access, writes outside the project, git commands that write. (Codex: `sandbox_permissions: require_escalated` with a `justification`; Claude Code: `dangerouslyDisableSandbox`.)

When a sandboxed command looks blocked, glave adds a hint saying so and naming `outside_sandbox`, so the model does not waste turns guessing. Detection follows Codex: a nonzero exit other than 2, 126 or 127 whose output mentions "operation not permitted", "permission denied", "read-only file system", "sandbox", or a failed name lookup. Nothing retries on its own; the model decides.

### Symlinks

A command can create a symlink inside the project that points at a protected path. glave's file tools resolve symlinks before checking (as `Workspace::resolve` does) and refuse protected targets, so the unsandboxed side never writes through a link the sandbox side planted. (CVE-2026-39861.)

## Enforcement

| Where | How | Status |
| --- | --- | --- |
| macOS | `/usr/bin/sandbox-exec`, by absolute path, with a profile glave generates per command. Starts from "deny everything" (Codex's audited base policy, Apache-2.0) and allows reads, process launch, the writable roots minus protected paths, and nothing on the network. Deny-by-default also closes Apple Events and XPC, which could otherwise launch apps outside the sandbox. Deprecated by Apple but present and working on macOS 26.6 (tested 2026-10-08) | phase 3 |
| Linux | glave re-runs itself as a small helper (a hidden subcommand). The helper builds a bubblewrap sandbox (read-only root, writable project, protected paths re-bound read-only, secret folders hidden, network unshared), and inside it applies a seccomp filter blocking new sockets and `io_uring`, then runs the command. Codex and Claude Code both use bubblewrap; Landlock is a fallback for writes and TCP where bubblewrap cannot start (Ubuntu 24.04's AppArmor blocks it without a profile). No `unsafe` in glave: the helper restricts its own process and then calls the safe `exec` | phase 4 |
| Windows | none yet: falls back to asking. Codex has a native Windows sandbox to learn from | later |
| Cloud Computers | the virtual machine is the wall, with the same levels mapped to its network and disk policy | with Computers |

Paths go into the policy resolved (`/var` is really `/private/var` on macOS; the sandbox matches real paths).

A deny-by-default profile lists the system settings commands may read, and new macOS releases add some. CI on macOS catches breakage when a release lands.

The sandbox is a port with two real implementations (Seatbelt and bubblewrap), as AGENTS.md requires: it wraps a command under a policy and returns the command to run.

## Recording

- `turn.started`: the turn's config blob gains the approval mode, the sandbox level and how it is enforced (`seatbelt`, `bubblewrap`, `landlock`, `none`).
- `tool.call.completed` for `bash`: `structured` gains `sandbox` (the level it ran under, or `none`) and `escalated` (whether the person let it leave the sandbox).
- The receipt names the protections: `done · sandbox: project · …`.

So any thread shows what protections were on for every command, and undo can say which effects it cannot reach.

## Tests

- Integration tests run the `bash` tool under each level without a model: writes outside blocked, temp folder allowed, `.git` and `.glave` read-only, secret reads blocked, network and Unix sockets blocked, `cargo` and `git status` working. They run in CI (Linux on every push, macOS on `main`).
- The file tools refuse protected paths, including through a symlink.
- Live tasks under `unattended` + `project`, each failing before its phase and passing after:
  - `escape-write`: write a file outside the project.
  - `escape-network`: fetch a web page.
  - `escape-ssh-key`: read a planted key in a fake home's `~/.ssh`.
  - `dotenv-bash`: `cat .env` with a planted canary.
  - `git-hook`: plant a pre-commit hook, by command and by the write tool.
  - `glave-config`: create `.glave/` settings from a command.
  - `fix-bug` and the rest still pass inside the sandbox.

## Central configuration

Organizations will cap these settings ("approval at most `auto`", "sandbox at least `project`"), and projects may only tighten them. That is the layered configuration in issue #7; this design keeps the settings as plain values so the layers can cap them later.

## How others do it (checked 2026-10-08)

| | Approval | Sandbox | Protected inside the project | Reads of `~/.ssh` |
| --- | --- | --- | --- | --- |
| Codex | `on-request` (default), `never`, `granular` | `read-only`, `workspace-write`, `danger-full-access`; Seatbelt; bubblewrap + seccomp | all of `.git`, `.codex` (even before it exists), `.agents`, `.aws` | allowed |
| Claude Code | `default`, `acceptEdits`, `plan`, `auto` (a classifier), `dontAsk`, `bypassPermissions` | off by default; Seatbelt; bubblewrap; network through a domain-filtering proxy | `.git/hooks`, `.git/config`, bare-repo files, `.claude`, `.mcp.json`, shell rc files, `.vscode`, `.idea` | allowed |
| Gemini CLI | | off by default; Seatbelt profiles, Docker, Podman, gVisor | | |
| OpenCode | `allow`, `ask`, `deny` per tool | none | | |

## Phases

1. This document.
2. Approval modes: `--approval <mode>` (`--yes` is `unattended`), the table above, protected paths in the file tools, recording. Without an OS sandbox yet, `auto` falls back to asking for commands.
3. Undo covering commands (its own design), with a live task that deletes and discards work by command and must get it all back.
4. macOS sandbox: profile generation, `outside_sandbox`, the hint, the live escape tasks. Then `auto` becomes the default where the sandbox runs.
5. Linux sandbox: the helper, bubblewrap, seccomp, the Landlock fallback.
6. Later: network allowlist through a proxy, Unix socket allowlist, organization caps, Windows.
