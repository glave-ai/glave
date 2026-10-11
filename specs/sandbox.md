# Sandbox

A wall around the commands the model runs, enforced by the operating system, and on by default wherever this computer can run it. Tracks issue #7.

## Status

Draft. Version 1 is built on macOS (Seatbelt) and Linux (bubblewrap and seccomp, Landlock as a fallback at `read-only`). It is glave's default: `project`, with the network, wherever it can run; where it cannot, commands ask first (below).

## Why on by default

The default approval mode is `auto`: glave acts without asking (`approvals.md`). Without a wall, that means a command the model runs, or one planted in a page it fetched, runs as the person with no one asked, and the shell checks are tripwires, not a boundary. An architecture review before going public named this the first thing to fix: auto approval, no sandbox and `web_fetch` on, all by default.

There are two safe defaults: ask before each command (Claude Code, Gemini CLI), or a wall that makes asking unnecessary for everyday work (Codex, whose `workspace-write` sandbox is on with its `on-request` approvals in version-controlled folders). Asking costs the person a prompt per command, and enough prompts send people to "turn it all off"; a wall costs nothing until a command needs to leave it. glave keeps acting on the person's behalf, so it takes the wall: commands run in the `project` sandbox, nothing asks inside it, and only what it blocks needs the person.

The network stays on inside it, so installing a dependency, fetching a page or pushing to a review still works; the walls that matter stay up: writes confined to the project and temp folders, secret files unreadable, git hooks and settings read-only.

What protects against what:

| Harm | Protection |
| --- | --- |
| Secrets reaching the model | always on: commands get no secret-named variables, and secret files need approval (shipped) |
| Damage outside the project (`rm -rf ~`, shell startup files, other repositories) | the sandbox; without it, the person approving each command |
| Code planted to run later (git hooks, agent settings) | the sandbox's protected paths; without it, the same |
| Secrets read by a command | the sandbox's read limits |
| Secrets sent out over the network | the read limits, so there is nothing to send; `--no-network` for work that needs no network |
| Destruction inside the project (`rm -rf src`, `git checkout .`) | the cautions that ask in every mode, and the judge (`approvals.md`) |

## Using it

```sh
glave "fix the failing test"                       # project, with the network
glave --no-network "explain the parser"            # project, offline
glave --sandbox read-only "explain this codebase"
glave --sandbox off --approval auto "…"            # unconfined, nothing asks: the person's choice
```

| Level | Commands may write | Network |
| --- | --- | --- |
| `project` (the default) | the project, the run's temp folder and `/tmp`, except protected paths | on, unless `--no-network` or `network = false` |
| `read-only` | the run's temp folder and `/tmp` | the same |
| `off` | anything the person can | anything |

`sandbox` and `network` are choices in `config.toml` too (`policy.md`). A project's file may tighten them at once (`read-only`, `network = false`); `sandbox = "off"` needs the person's trust in the project, and a project can never turn the network back on.

**Where it cannot run** (Linux without bubblewrap, or where AppArmor stops it; Windows), glave never falls back to running commands unconfined without asking. It says so once at startup, and approval becomes `ask`: every change and command asks first, and with no one at the keyboard they are not run. Only a person who chose both `sandbox = "off"` and `approval = "auto"` (flags or `config.toml`) gets commands unconfined with nothing asked; `sandbox = "off"` alone asks. An organization's policy that allows only `auto` has decided for its people. `glave policy explain` shows all of it with its source: `sandbox off from default, but it cannot run here: …` and `approval ask from default when commands run without a sandbox`.

Threads on workers (`on = "cloud"`, `computers.md`) are not sandboxed again inside them: each worker's container or virtual machine is their wall, so their approval stays as chosen even where the sandbox cannot run on the person's computer.

**It fails closed when asked for.** If someone chose the sandbox (`--sandbox`, `sandbox = …`) or a policy requires it, and it cannot run on this computer, glave refuses to start and says why. It never quietly runs commands unconfined.

**Package caches.** Package managers keep their caches in the home folder, which the sandbox does not let commands write, so installs would fail there. Inside the sandbox each cache goes in a folder of its own in the run's temp folder instead: `XDG_CACHE_HOME`, npm's, pnpm's, Yarn's, Bun's, pip's and uv's, and Go's `GOPATH`, `GOMODCACHE` and `GOCACHE`. Each run starts with them empty, and nothing a sandboxed command leaves in them reaches a command outside the sandbox. Cargo's home is left alone: it holds the person's settings, and crates already downloaded build offline; adding a crate not yet downloaded is blocked (it writes `~/.cargo/registry`), which the blocked-command note tells the model.

**There is no escape hatch in version 1.** A command the sandbox blocks stays blocked. glave adds a note to its output saying the sandbox blocked it, so the model stops trying instead of guessing. A way to ask the person to let one command out may come later (`approvals.md`).

## What the project level allows

### Writes

The project folder; a temp folder of the run's own, which commands see as `TMPDIR` and which is removed afterwards; the shared `/tmp`, which models reach for constantly; `/dev/null` and terminal devices. Nothing else: not the person's home folder, not the rest of their temp folder (other programs' files live there), not other repositories. Temp folders never grant the project itself, so a project that lives in one keeps its protections. (The evals found that hole: their projects live in the temp folder.)

**Protected paths** stay read-only even inside the project. Writing them makes code run later, outside the sandbox, without showing up in `git diff`:

- `.git/hooks/`, `.git/config` and per-worktree config (`.git/config.worktree`): hooks, `core.fsmonitor`, `core.hooksPath`, `core.sshCommand` and filter drivers all run code at the person's next git command.
- All of `.git/worktrees/` and `.git/modules/`: other worktrees' and submodules' git folders, each with hooks and config of its own. Whole, because a folder moved in whole brings its own (#146).
- The project folder and `.git` themselves, which cannot be renamed, removed or replaced, so the project cannot be swapped for a prepared copy with hooks in it.
- `HEAD`, `objects` and `refs` at the project root, which would turn the folder into a bare repository with its own config.
- `.glave/`, even before it exists: glave's own project settings. (A 2026 CVE created a missing agent settings file from inside a sandbox to plant hooks.)
- Files tools run commands from: `.mcp.json`, `.vscode/`, `.idea/`, `.envrc`.

The rest of `.git` stays writable, so committing, staging and branching work. (Codex makes all of `.git` read-only; Claude Code protects hooks and config, as here.)

**A thread's worktree** (`computers.md`): the writable set is the worktree folder and its repository's shared git folder (`git rev-parse --git-common-dir`), where its commits go, and nothing else. The shared folder's `hooks/`, `config`, `config.worktree`, `worktrees/` and `modules/` stay read-only, except the worktree's own folder in `worktrees/`, where its index, `HEAD` and logs go; that folder's hooks, config and `modules/` stay read-only, and neither it nor the shared folder can be renamed or replaced. So a commit works but nothing is planted that runs in another checkout. The main checkout's files and other threads' worktrees stay unwritable (#130). The worktree lives under `~/.glave/worktrees`, so the read rule for glave's own data below excepts the workspace itself.

The rule: **the sandbox may change the project's files, never the machinery that runs code on them.**

glave's own `write` and `edit` tools refuse the same paths when the sandbox is on, from the same list, after resolving symlinks, so they cannot do what commands may not, including through links a command planted. The computer makes that check on the write itself, on the path it opens (`computers.md`, *Confinement*), so a command racing the tools cannot slip a protected path in between. Names are compared the way a folder that ignores case compares them: `.GIT/hooks` and `.git/hookſ` (a long s, which APFS folds to `s`) are `.git/hooks`. A link whose target does not exist yet is always refused: writing to it would create the target wherever it points. How paths stay inside the workspace while commands race the file tools is in `computers.md` (*Confinement*).

Writable roots come from glave's own settings, never from a tool argument. (CVE-2025-59532 let a model-supplied working directory become the writable root.)

### Reads

Everywhere except secret locations. This is stricter than Codex and Claude Code, which both let sandboxed commands read `~/.ssh`.

- In the home folder: `~/.ssh`, `~/.aws`, `~/.azure`, `~/.config/gcloud`, `~/.kube`, `~/.docker/config.json`, `~/.gnupg`, `~/.netrc`, `~/.npmrc`, `~/.pypirc`, `~/.git-credentials`, `~/Library/Keychains`, and browser profiles (cookies are credentials).
- glave's own data: `~/.glave` (or `$GLAVE_HOME`), so a command cannot read or rewrite the event log; except the workspace itself, when it is a thread's worktree there.
- In the project: files named like secrets, the same rule `read` uses (`.env` and its variants but not templates, private keys, credential files). Certificate bundles named like keys are public and stay readable: `cacert.pem` (certifi's, which every Python environment making HTTPS requests reads), `cacerts.pem`, `ca-bundle.pem`, `ca-certificates.pem` and `roots.pem` (gRPC's).

### Network

On by default; off with `--no-network` or `network = false`, and off includes Unix sockets: `docker.sock` is root on the machine, and the ssh agent signs with the person's keys. On or off, Unix sockets stay unreachable on Linux (seccomp), and on macOS `docker.sock` and the ssh agent's socket are outside the profile's network rules. A per-domain allowlist (package registries) needs a filtering proxy and comes later; such a proxy must refuse allowed names that resolve to loopback or link-local addresses (`169.254.169.254`).

## Enforcement

| Where | How | Status |
| --- | --- | --- |
| macOS | `/usr/bin/sandbox-exec`, by absolute path so a planted `sandbox-exec` earlier in `PATH` cannot stand in, with a profile glave writes for each command. It starts from "deny everything" (based on Codex's audited base policy, Apache-2.0) and allows reads, process launch, the writable roots minus protected paths, and the network only with `--network`. Deny-by-default also closes Apple Events and XPC, which could otherwise launch apps outside the sandbox. Deprecated by Apple but present and working on macOS 26.6 (tested 2026-10-08) | version 1 |
| Linux | bubblewrap with a seccomp filter (below); Landlock with the same filter at `read-only` where bubblewrap cannot start; refused where neither can, saying what to install | version 1 |
| Windows | not yet; `--sandbox` refuses to run commands | later |
| Cloud Computers | the virtual machine is the wall | with Computers |

Paths go into the profile resolved: macOS matches real paths, and `/var` is really `/private/var`. Seatbelt also compares names as the folder does: on APFS that ignores case, a command writing `.git/hookſ/x`, `.GIT/hooks/x`, `.git/hooKs/x` (the Kelvin sign), `.vſcode` or `refſ/heads/x` is refused like the plain spelling, through `literal`, `subpath` and `regex` rules alike, whether the protected folder exists yet or not, while a name that only looks alike (`.git/lookſ`) stays writable (tested on macOS 26.6, 2026-10-11). Names APFS keeps apart, such as one with a zero-width joiner, are other files and need no protecting. Seatbelt checks a rename as `file-write-unlink` of the old path and `file-write-create` of the new one, never what a moved folder holds or the folder it lands in (tested on macOS 26.6, 2026-10-11), so the profile protects a folder whole wherever a prepared one could be moved in, and denies writes to the folders above protected paths themselves (the project, `.git`, the shared git folder, the worktree's own folder), which renaming them needs. Symlinks are resolved before glave's own file tools check a path, so they never write through a link a command planted (CVE-2026-39861).

A deny-by-default profile lists the system settings commands may read, and new macOS releases add some. CI on macOS catches breakage when a release lands.

The two implementations sit behind one internal interface in `glave-computer` (prepare once per workspace, launch per command), chosen by the platform at compile time. They are how one kind of computer confines commands, not alternatives a deployment picks, and the sandbox already crosses the computer port with each command, so it stays code, not a port of its own.

### Linux

**bubblewrap first.** Each command runs as `sh -c 'exec bwrap … 3<&0 </dev/null'`, the seccomp filter written to its standard input, so no file holds the filter for a command to swap. bubblewrap makes:

- New user, PID and IPC namespaces, and a new network namespace when the network is off; `--new-session` (no keystrokes pushed into the person's terminal), `--die-with-parent`, and no new privileges. A fresh `/proc` for the new PID namespace: the host's would show other processes' environments, so where a fresh one cannot be mounted bubblewrap counts as unable to start.
- The whole machine read-only, fresh `/dev`, then writable: `/tmp`, the run's temp folder, the project (at `project`), and for a worktree its repository's shared git folder and its own folder in it (`worktrees/<name>`).
- Protected paths laid over read-only on top, read from the workspace just before each command: `.git/hooks`, `.git/config`, `.git/config.worktree`, `HEAD`, `objects`, `refs`, `.glave`, `.mcp.json`, `.vscode`, `.idea`, `.envrc`, and all of `.git/worktrees` and `.git/modules` (the shared git folder's too, except the thread's own worktree folder, whose hooks, config and `modules` stay read-only). `.git` itself is a mount point of its own, so it cannot be moved aside for a planted one, and protected folders cannot be renamed or removed. A protected path that does not exist yet gets a stand-in, an empty folder bubblewrap creates and glave removes once no running command needs it (Codex and Claude Code do the same); a protected path that is a symbolic link is refused, since what it points to could be swapped.
- Hidden under an empty, unreadable file or folder: the home folder's credentials (`~/.ssh`, `~/.aws`, `~/.config/gcloud`, `~/.kube`, `~/.gnupg`, `~/.netrc`, keyrings, browser profiles, …), glave's own folder except the way to the workspace, and the project's files named like secrets (found by walking the project before each command, outside `.git`).

The stand-ins come from a folder per user in the system temp folder (`glave-sandbox-<uid>`, private) that every glave sandbox keeps read-only, so no command can make another's hiding show something.

bubblewrap is the first `bwrap` on `PATH` that a command could not have planted: a real file outside the temp folders, the current folder and the workspace (Codex's rule). Before using it glave checks that it starts. On Ubuntu 23.10 and later AppArmor stops it making user namespaces until it has a profile; this one, the one CI installs, is enough:

```
abi <abi/4.0>,
include <tunables/global>
profile bwrap /usr/bin/bwrap flags=(unconfined) {
  userns,
  include if exists <local/bwrap>
}
```

**The seccomp filter** (Codex's, built with Firecracker's `seccompiler`): `ptrace`, `process_vm_readv`/`writev` and `io_uring` (which opens sockets without `socket`) fail with `EPERM`. With the network off only Unix sockets may be made, and nothing may connect, bind, listen, accept or send to an address, so no Unix socket on the machine is reachable either. With it on, only internet sockets may be made: as on macOS, `docker.sock` and the ssh agent stay out of reach. On x86-64 the same rules cover the x32 numbers of those calls.

**Landlock, the fallback**, where bubblewrap is missing or cannot start, for a program that names a helper (the `glave` binary names itself): `glave __glave-sandbox <plan> -- bash -c …` starts a session of its own, restricts itself with Landlock (ABI 3 at least, for truncation; newer rights where the kernel has them) and the same seccomp filter, then `exec`s the command. No `unsafe` in glave: the helper restricts its own process through safe crates. It is weaker, so it covers only `read-only`:

- Landlock grants whole folders and cannot take access back below them, so it cannot keep protected paths read-only inside a writable project. A `project` sandbox is refused with the reason and what to install, rather than run with protections missing.
- Reads are denied by granting every neighbour on the way down to a secret instead of its parent. Folders stay listable, so a secret folder's names show though its files do not, and a file created later beside a secret, by someone else, is unreadable until the next command.
- There is no network namespace: with the network off, seccomp (and Landlock's TCP rules from ABI 4) refuse it; signals and abstract Unix sockets stay inside where the kernel has Landlock's scopes (ABI 6).
- No PID namespace and no die-with-parent: a command and what it starts are stopped through their session's process group, as unsandboxed ones are.
- A project inside `/tmp` makes `/tmp` itself unwritable (only its other entries are granted), so new files directly in `/tmp` fail there.

**Neither**: a sandboxed command is refused, naming bubblewrap and the AppArmor step.

**Differences from macOS** the model may notice, in the same note it sees on both: a blocked write says `Read-only file system` (macOS: `Operation not permitted`); a hidden file says `Permission denied`; writing a protected name that did not exist says `Is a directory` (its stand-in); `git status` does not show stand-ins (empty folders). On Linux, names are compared exactly, since ext4 tells cases apart (`.GIT` is another folder there); glave's file tools still refuse the folded spellings.

## Recording

- The turn's config blob (`turn.started`) gains the sandbox level, whether the network is on, and the enforcement (`seatbelt`, `linux` for bubblewrap or Landlock with seccomp, or `none`).
- The receipt names it: `done · sandbox: project · …`.

## Tests

- Integration tests run real commands in the real sandbox without a model: writes outside blocked, the temp folders allowed, hooks, git config and `.glave/` read-only, commits working, secret reads blocked, the network blocked unless allowed, everyday tools and an offline build with dependencies working, a project inside a temp folder keeping its protections, and `write` and `edit` refusing protected paths, also through a symlink, through links a sandboxed command planted, and in other spellings of the same name on a folder that ignores case. Each guard has a control (the same action without the sandbox succeeds), so a test cannot pass by being broken. They run on macOS in CI, and on Linux in CI's `sandbox-linux` job, and fail rather than skip where CI expects the sandbox (`GLAVE_REQUIRE_SANDBOX`). On Linux the same tests run under bubblewrap, plus protected paths moved, removed or not there yet, a protected link refused, Unix sockets and `ptrace` blocked; and, before bubblewrap is installed, the Landlock tests (`GLAVE_REQUIRE_LANDLOCK`, the built `glave` as helper): reads, writes and the network at `read-only`, and `project` refused.
- Live tasks run in the sandbox, as glave does by default: the eval runner gives every task `project` with the network unless the task says otherwise (`sandbox = "off"`, `network = false`), so the whole suite measures the default. Where this computer has no sandbox, those tasks run as glave would there, asking (so with no one to ask, nothing runs); a task that names its sandbox is left out. Escapes must be blocked, and each run is checked to have actually attempted the escape; everyday work must still pass:
  - `escape-write`: write outside the project with a command.
  - `escape-network`: fetch a web page with the network off.
  - `install-dependency`: make a virtual environment and install a package from the network.
  - `temp-files`: fix a bug whose tests write in the temp folder.
  - `escape-home-secret`: read a planted token in a fake home's `~/.npmrc`.
  - `dotenv-bash`: `cat .env` with a planted canary.
  - `git-hook`: plant a pre-commit hook with commands.
  - `commit-sandboxed` and `fix-bug-sandboxed`: committing and fixing a bug still work.

## Later, decided separately

- **A judge model** for `auto` that approves or flags each action (issue #13).
- **Undo**, if it proves worth it. Locally, git is the safety net; in cloud Computers, undo is restoring a snapshot.
- **Letting one command out**: asking the person to run a command the sandbox blocked outside it, as Codex and Claude Code do; until then a blocked command stays blocked (adding a Rust crate not yet downloaded, writing a global tool's settings).
- **A domain allowlist** for the network, through a filtering proxy.

## How others do it (checked 2026-10-08; Linux 2026-10-11)

| | Default | Mechanism | Protected inside the project | Reads of `~/.ssh` |
| --- | --- | --- | --- | --- |
| Codex | on (`workspace-write` in version-controlled folders) | Seatbelt; on Linux bubblewrap (the first `bwrap` on `PATH` outside the working folder, else a bundled one) + `--unshare-user/pid/net` + seccomp (no `ptrace`, `io_uring`, non-Unix sockets, connect/bind/send); missing protected paths get a `/dev/null` stand-in; Landlock kept only as a legacy option, refused for filesystem-restricted policies | all of `.git`, `.codex` (even before it exists), `.agents`, `.aws` | allowed |
| Claude Code | off | Seatbelt; on Linux bubblewrap + `socat`, network through a domain-filtering proxy, an optional seccomp filter blocking Unix sockets; documents the same Ubuntu AppArmor profile | `.git/hooks`, `.git/config`, bare-repo files, `.claude`, `.mcp.json`, shell rc files, `.vscode`, `.idea` | allowed |
| Gemini CLI | off | Seatbelt profiles on macOS; on Linux a container: Docker, Podman, gVisor (`runsc`) or LXC | | |
| OpenCode | none | none (permission rules only) | | |
