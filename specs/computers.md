# Computers

Where work runs: the machines, the recipes they are made from, and the working copies threads act on. Tracks issue #29; the sandbox around commands is in `sandbox.md`.

## Status

First cut built (#116). The computer port (`Computers` in `glave-ports`) and its conformance suite; `glave-computer` with this machine as a computer (`LocalComputers`), `glave computer serve` (stdio, and TCP on 127.0.0.1 behind a token) and the client that reaches it (`RemoteComputers`), both passing the same suite; environments read from devcontainer.json or environment.toml.

The harness's six tools run on a computer through the port. The harness's `Workspace` is a binding to one workspace on one computer; the CLI, the session server and evals bind to this machine (`LocalComputers::here`, the folder glave starts in, worked on in place), and a test runs a whole turn against a computer over TCP and gets exactly what the model gets locally. The cut, as Codex's exec server and Claude Code's remote sessions draw it: the brain stays in the harness (approvals, the judge, the shell assessment, secrets handling, read-before-write freshness, every word the model reads, truncation, receipts), and the computer is the hands (paths confined to the workspace, files read and written, searches walked next to the files, commands run in the sandbox each one carries). Rather than one `tool/run`, the protocol stays fine-grained and coarse: `command/run`, `files/*` and `workspace/save`, plus `files/stat` (where a path resolves, what is there and its digest, so freshness holds without fetching the file), `files/glob` and `files/search` (run on the computer, answering with results), and listings that leave out what git ignores. Path refusals are whole sentences the model reads as they are. Sleep, wake and resize come next.

Workers (cloud computers) on Docker or Kubernetes: `--on cloud` (or `on = "cloud"` in `config.toml`) puts each new thread on a worker of its own; on Kubernetes a runtime class (gVisor or Kata) gives each one a kernel or machine of its own. See "Workers" below.

Worktrees per thread (#29): a thread works in the folder in place by default, exactly as before. With `worktree = true` in `config.toml` (the person's or the project's, nearest wins), `--worktree`, or `thread/start`'s `worktree`, it gets a git worktree of its own instead (below). Every thread the session API starts records `workspace.bound` (`events.md`), and resuming a thread reads it to open the same worktree again.

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
8. **Secrets never enter a remote computer.** Git and cloud credentials reach it through a credential helper that asks the harness for each use; values are never written to its disk. (Built so far: a git credential travels in `workspace/open` and is kept in the worker's memory, below; asking for each use comes with `credentials/request`.)

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

Built so far, beside `computer/create`, `computer/status`, `computer/destroy`, `workspace/open` (which may carry a `credential`, below) and `workspace/close` (wire version `2026-10-10.1`):

| Method | Does |
| --- | --- |
| `server/discover` | the server's name, version and protocol versions |
| `connection/authenticate` | the first message on a network transport: the token, before it expires; anything else ends the connection |
| `command/run` | runs a bash command at the workspace root, its output (standard output and error together) streamed as `command/output` notifications before the answer: exit code, signal, timeout or cancel, and how long it took. The sandbox (level, network, the home and glave folders to protect) travels with the command; a computer that cannot enforce it refuses the command as unsupported, never runs it unconfined |
| `command/cancel` | stops a running command and everything it started |
| `files/read`, `files/write`, `files/list` | one file read whole (at most 20 MiB, as base64), files written all-or-nothing after every path is checked, a folder listed (at most 10,000 entries), optionally leaving out what git ignores and counting it; every path confined to the workspace, symlinks followed before the check |
| `files/stat` | what is at a path: where it resolves inside the workspace, nothing, a folder, or a file with its size and SHA-256 |
| `files/glob` | files matching a pattern (`*.rs` by name, `src/**/*.rs` by path), newest first, leaving out what git ignores, up to a limit, with the total |
| `files/search` | lines matching a regular expression (plain text when it is not one), in path order, up to a limit; ignored, binary and large files are not searched, and files that usually hold secrets are never opened, only named |
| `workspace/save` | commits everything to a branch and pushes it to `origin`; a plain folder keeps the commit in a hidden history (`.glave/history.git`, up to 100 MB of files, listed in `.glave/.gitignore` so a later `git init` never commits it), never a `.git` |

A repository opens as a clone under `~/.glave/workspaces/<computer>/<workspace>`, removed on close; a folder opens in place and is kept.

`workspace/open` also takes `{"kind": "worktree", "path", "branch"}`: a git worktree of the repository around `path`, on `branch`, made from the folder's current commit when the branch is new (`git worktree add -b`), or checked out again when the branch exists. It lives at `~/.glave/worktrees/<repository>-<digest>/<branch, / as ->`, never inside the repository; the digest is of the repository's shared git folder, so two repositories with one name never share a folder. Opening the same branch again reopens the same worktree, and closing one keeps it. A folder outside a repository, or a repository with no commit, is refused with a sentence saying so; a computer that keeps no worktrees answers unsupported. Glave names a thread's branch `glave/` and the last 12 hex digits of the thread's id (its random part, so threads started in the same moment never share one), and starts the thread's tools there; the model is sent the same prompt and tools as in place, with paths relative to the worktree.

**Cleanup is deterministic.** Each worktree has a record beside it (`<folder>.json`: its repository's shared git folder, branch, owning thread as `<org>/<space>/<thread>`, and any hold). A worktree is **safe** to remove when `git status --porcelain` is empty and `git rev-list HEAD --not --remotes` prints nothing: every commit is on some remote, so a squash-merged branch that was pushed counts. Removing one never deletes its branch, so resuming its thread checks it out again (and clears any hold). Every removal is recorded on the owning thread as `workspace.removed` (`events.md`) with its reason. The triggers:

- **Thread archived or deleted** (`archived`): safe → removed; otherwise kept and held. The session API has neither yet; the hook is built and tested, and is wired when they come.
- **Idle 30 days** (`idle`): the thread's last event is 30 days old or more. glave checks once a day as it starts (the time of the last check is in `~/.glave/worktrees-checked`): safe → removed; otherwise kept and held, and said once.
- **Repository gone** (`repository_gone`): its shared git folder no longer exists. The worktree folder is moved to the Trash (macOS `~/.Trash`; the freedesktop.org Trash on Linux, with its `.trashinfo`), never deleted; with no Trash it is kept and glave says so.
- **`glave worktrees prune`** (`pruned`): removes every safe one and trashes those whose repository is gone. For each other one it says why it stays ("1 commit not pushed", "uncommitted changes"). `--force <name>` (the folder's name) removes that one anyway. glave chose the flag over offering to push a `glave/wip/...` branch first because it is simpler.

`glave worktrees` lists them all: folder, branch, whether removing each would lose work, and any hold with its trigger. These work on this machine's worktrees directly, not through the port. A cleanup that records on a thread a running glave also holds makes that glave's next write to it conflict. Moving cleanup into the session server will close that.

How the leading tools compare, by default: Claude Code's `--worktree <name>` is opt-in, puts the worktree inside the repository at `.claude/worktrees/<name>` on a `worktree-<name>` branch from the default branch, and asks on exit whether to keep one with changes. Codex's app puts worktrees in `$CODEX_HOME/worktrees` on a detached HEAD from the current commit and never deletes them on its own. glave keeps Codex's place outside the repository and its starting commit, and names a branch as Claude Code does, with its own prefix, as rule 3 already did (`glave/<thread>`). A computer on the host refuses what it cannot honour (an image, setup commands, a limited network) rather than ignoring it.

devcontainer.json honours `image`, `containerEnv` and `remoteEnv` (written values only, no `${...}`), `onCreateCommand`, `updateContentCommand` and `postCreateCommand` (as setup, in that order) and `hostRequirements` (cpus, memory, storage). It refuses by name what glave cannot carry out (building images, Compose, non-empty `features`, `initializeCommand`, `postStartCommand`, `postAttachCommand`, mounts, container options, GPUs, unknown keys), and lists the editor and preview settings it leaves alone (`customizations`, `forwardPorts` and the like). environment.toml refuses unknown keys.

## Workers

A worker is a disposable computer, one per thread: a container started from the environment's image, with glave inside serving the computer protocol. The harness stays with the data store and drives the worker through the port (`WorkerComputers` in `glave-computer`), exactly as it drives this machine. Backends sit behind a small trait inside the crate: Docker, and Kubernetes (behind the crate's `kubernetes` feature, which only the CLI turns on). The trait stays inside the crate until a backend across a boundary needs a protocol for it.

**Lifecycle.**

1. `computer/create` with an environment checks it against policy's limits (below), then the backend starts a worker. The image must be pinned by digest (`name@sha256:<64 hex>`), so the same bytes run every time; anything else is refused. The harness side mints the worker a TLS identity (a self-signed P-256 certificate) and a token, and they go in as files with glave before the container starts.
2. The harness connects, checks the protocol version, and makes the worker's one computer: the container itself. A worker that cannot start is removed, and the error carries the last lines it printed.
3. `workspace/open` with `{"kind": "git", "url", "at"}` clones the repository in the worker. With `--on cloud` that is the project's `origin` (SSH forms rewritten to HTTPS, since SSH keys never leave the person's machine) at the current commit, which must already be pushed. The git credential, when the host's own git has one (`git credential fill`, never interactive), travels inside `workspace/open` over TLS, and the worker keeps it in memory only: git gets it through a credential helper given on the command line (`-c credential.helper=...`) that reads two variables set on that one git process. It is never written to the repository's config, a file, the image, or the environment commands see. The environment's setup commands then run once, in order, at the workspace root; one that fails fails the open and says what it printed.
4. The harness copies the thread's attachments in under `.glave/in/` (`files/write`). Attachments are not yet first-class in the session API, so the harness takes a list of files (`Workers::attachments`); the CLI passes none yet.
5. Work leaves a worker only as a pushed branch, a file handed back, or a store write. The model is told so (below). Releasing a worker, when its session ends, reads the files under `.glave/out/` and hands them back (the CLI keeps them in `GLAVE_HOME/out/<thread>/`), removes that folder, and when `git status --porcelain` or `git rev-list HEAD --not --remotes` prints anything, saves everything to `glave/wip/<12 hex digits of the thread id>` and pushes it (`workspace/save`). Then the worker is destroyed, and `workspace.removed` is recorded with the reason `released` and the branch, if any. A thread whose worker was released does not resume onto this machine's folder: resuming it says its work is on the branches it pushed.

**Transport.** A worker with a network is reached over TLS 1.3 on a port published on 127.0.0.1 only. The harness trusts exactly the certificate it made (its SHA-256 fingerprint is pinned; the handshake signature is still checked) and presents the token first. The token is 32 random bytes and opens connections for 10 minutes; a connection opened before then lasts. `glave computer serve --tls <host:port> <folder>` serves on any address, but only with TLS: plain TCP stays on loopback behind `GLAVE_COMPUTER_TOKEN`. A worker with no network has no port at all; the harness reaches glave's standard input and output through the backend (Docker's or Kubernetes' attach), so nothing listens.

**Docker.** One container per worker from the Engine API over its Unix socket (`DOCKER_HOST`, `/var/run/docker.sock`, or Docker Desktop's in the home folder), named `glave-worker-<id>` and labelled `glave.worker`. Limits: `NanoCpus` and `Memory` (swap the same, so none) from the size, 4096 processes, `no-new-privileges`, `NET_RAW`, `MKNOD` and `AUDIT_WRITE` dropped, an init process. Network `none`, or the default bridge with the port on 127.0.0.1. Variables are the environment's, never anything named like a secret. glave goes in at `/opt/glave/glave` as a Linux build for the engine's processor, from the path in `[workers] glave` in `config.toml` (checked: an ELF for arm64 or x86-64 to match the engine), with `tls/cert.pem`, `tls/key.pem` and `tls/token` beside it. Removing a worker removes its container and its volumes. The image needs bash and git, as devcontainer images have.

**Kubernetes.** One Pod per worker in one namespace (`[workers] namespace`, or the kubeconfig's), named `glave-worker-<id>`, through the API server with the `kube` client: signed in by `[workers] kubeconfig`, else `KUBECONFIG` or `~/.kube/config`, else the service account of the Pod glave itself runs in. Each worker is two objects, both labelled `glave.worker=<name>`: a NetworkPolicy, made first so the Pod never runs without it, and the Pod. Removing a worker deletes both at once (no grace period).

- *The Pod.* The environment's image; CPU and memory from the size as both requests and limits (so the worker gets what it is allowed); a node of the processor glave was built for (`kubernetes.io/arch`, read from glave's ELF header); `runtimeClassName` from `[workers] runtime_class`; `restartPolicy: Never`; no service account token (`automountServiceAccountToken: false`) and no service variables (`enableServiceLinks: false`); a shared process namespace, so the Pod's pause process is the first and reaps what commands leave (Docker's init). Its container may not gain privilege (`allowPrivilegeEscalation: false`, not privileged), drops every capability (`glave computer serve` needs none: its port is above 1024), and runs under the runtime's seccomp profile. It runs as the image's user, as on Docker: Kubernetes cannot see which users an image has, and devcontainer images differ. Its root filesystem stays writable: a worker is a disposable development machine where installing packages and building (toolchains write under `/usr/local`) is the work, and the layer goes with the Pod.
- *Getting glave in.* Kubernetes has no copy before start, so the container starts with a small shell loop that waits (up to ten minutes) for `/opt/glave/ready` and then becomes glave. Once the Pod runs, the harness execs `tar -x` in it through the API server and streams glave, the TLS identity and the token (with a network only), and `ready` last. glave lands on an `emptyDir`; the identity on a memory-backed `emptyDir` (`tls/`, 1 MiB), so the key never touches the node's disk. Nothing secret is ever stored on the API server: no Secret, no variable, nothing in the Pod's spec. The image needs `sh` and `tar` besides bash and git; a cluster older than 1.30 cannot say when standard input closed, and then the worker's own wait tells whether everything arrived.
- *Reaching it.* Always through the API server, so it works wherever glave runs and the worker accepts no connection from the cluster's network. With a network, the harness listens on 127.0.0.1 and port-forwards each connection to glave's TLS port in the Pod, so the TLS session (pinned certificate, token) runs end to end inside the forward. Without, it attaches to glave's standard input and output. A Pod that cannot start (an image that cannot be pulled, a container that cannot be made, a runtime class the cluster lacks) fails at once with the reason; one waiting for a node waits up to five minutes and says what it waits for.
- *The network.* The NetworkPolicy lets nothing in (port-forward and attach come through the kubelet, which no policy sees). Network off lets nothing out. Network on lets everything out except link-local addresses (`169.254.0.0/16`, `fe80::/10`, `fd00:ec2::254`), where clouds serve their machines' credentials; DNS on port 53 stays open there for node-local caches. A NetworkPolicy is only as good as the cluster's network plugin: on one that ignores policies (flannel alone, for one), network off is not enforced, so such clusters need Calico, Cilium or another that enforces them.
- *Isolation.* Policy `[workers] isolation = "vm"` (`policy.md`) requires a runtime class: on Docker, or on Kubernetes without `runtime_class`, every new worker is refused, saying so. Glave Cloud runs untrusted code, so every worker there has one (gVisor's `runsc` or Kata); an enterprise's own cluster sets what it has. Any configured runtime class counts as the boundary; that it is a real sandbox is the cluster's word.
- *Access.* glave's identity needs, in the namespace: `pods` create, get, delete; `pods/exec`, `pods/attach`, `pods/portforward` create; `pods/log` get; `networkpolicies` create, delete.

**What the model is told** (model-facing), only on a worker, with the opening instructions and again after a compaction: one paragraph saying it is on a disposable Linux computer started for this thread, where its workspace came from (the clone and commit), that only what it pushes is kept (unpushed work goes to a `glave/wip/` branch on release), that files to hand back go in `.glave/out/`, and that attachments are in `.glave/in/`. With default settings nothing the model sees changes.

**Limits** come from `[workers]` in `policy.toml` (`policy.md`): `cpu` and `memory_gb` cap the size and are the size when the environment names none; `max_minutes` stops new work (commands, writes) after that long, and a command never outlives the time left, while reading and saving still work so the worker can be released with its work; `per_person` caps the workers one glave holds at once; `network` is `on` or `off`. Refused rather than ignored, because they are not enforced yet: a network allow-list (policy or environment), `disk_gb`, `secrets`, `idle_minutes` in the environment, and an environment without an image.

Not built yet: an egress allow-list; asking the harness for each credential use; workers per organization; sleep and idle timeout; removing workers a crashed glave left (they carry the `glave.worker` label); running the model's commands as a user other than the one holding the credential in memory; first-class attachments from the blob store; `--attach` in the CLI; on Kubernetes, gVisor tested in CI, a non-root user chosen by configuration, and a disk limit (`ephemeral-storage`).

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

- **On a laptop:** the computer is the laptop. The thread the person is driving works in the project folder itself unless `worktree = true` gives it a worktree under `~/.glave/worktrees/`; later, background threads, goals and scheduled work get their own copy beside it the same way (or a copy-on-write clone where the filesystem supports it).
- **In the cloud:** one persistent computer per person by default, sleeping when idle; a space can be pinned to a computer of its own; short-lived burst computers start from a snapshot of the person's computer for parallel work, later.
- **Removal:** a worktree is removed when its thread is archived, idle for 30 days or pruned, and only when nothing would be lost (above); reopening the thread checks its branch out again.
