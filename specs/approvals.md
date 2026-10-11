# Approvals

How much glave asks the person before acting. Tracks issues #21, #24, #78 and #81; the walls around commands are in `sandbox.md`.

## Modes

| Mode | Reads (`read`, `search`, `glob`) | Changes (`write`, `edit`) | Runs (`bash`, most MCP tools) |
| --- | --- | --- | --- |
| `auto` (the default, in the sandbox) | go through | go through | go through, in the sandbox |
| `ask` (the default without one) | go through | ask | ask |

Set with `--approval <mode>`, or `approval` in `config.toml`; an organization's policy can limit the modes allowed (`policy.md`). A tool's effect decides which column it is in; MCP tools that say they only read are reads.

**Auto runs only behind a wall, unless the person says otherwise.** The sandbox is on by default wherever this computer can run it (`sandbox.md`), and `auto` is the default there. Where commands would run without a sandbox (it cannot run here, or the person turned it off), the default is `ask`, and an `approval = "auto"` chosen with the sandbox still on by default gives way to `ask` too, saying so once at startup. Commands run unconfined with nothing asked only when the person chose both `sandbox = "off"` and `approval = "auto"`, or an organization's policy allows only `auto`.

In every mode, reading a file that usually holds secrets (`.env` and its variants, keys, credential files) asks first: the model would see its contents. With the sandbox on, the file tools refuse protected paths outright (`sandbox.md`).

`auto` is the default because almost everyone works that way, and agency means glave acts on the person's behalf and asks only what is truly theirs. What keeps it safe without asking is prevention: the sandbox around every command (`sandbox.md`), commands read before they run (below), and a judge for what the rules cannot decide (issue #13). `ask` is the cautious mode, and the default wherever that prevention is missing. The `edits` mode of earlier versions (changes go through, commands ask) is retired.

## Commands read before they run

Every `bash` command is parsed with tree-sitter-bash before it runs, as Codex, OpenCode and Claude Code parse it, and every command in it is found: in pipelines and lists, subshells, substitutions, loops, and scripts given to `bash -c` or `eval`. Wrappers such as `sudo`, `env X=1`, `timeout`, `nice`, `nohup` and `xargs` are looked through, and `/bin/rm` is `rm`. Then:

- **Risky commands ask in every mode**, even when the person allowed `bash` for the thread, and with no one there they are not run. The model is told why, and not to get the same result another way. Risky means:
  - deleting what cannot be got back: the root, a top-level folder, the home folder, the project or what holds it, anything outside the project but the temporary folder, or a folder the command cannot name in advance;
  - throwing away uncommitted work or rewriting what others share: `git reset --hard`, `git clean -f`, `git checkout .` and `git checkout -- …`, `git restore` of the work tree, `git stash drop` and `clear`, `git branch -D`, `git push --force` and its kin;
  - running code fetched from the network: `curl … | sh`, `bash <(curl …)`, `sh -c "$(curl …)"`;
  - acting on the computer: `sudo`, formatting or erasing disks, writing to devices, shutting down, changing permissions beyond the project;
  - planting what runs later: shell start-up files, `~/.ssh`, git hooks, launch agents, `crontab`, `launchctl`, `systemctl enable`, global git settings;
  - reading a file that usually holds secrets, as the file tools already ask;
  - printing a credential: `git credential fill` and `git credential-<helper>`, `gh auth token` (and `glab`'s), `security find-generic-password` and `find-internet-password` with `-w` or `-g`, and `security dump-keychain -d`, `gcloud auth print-access-token` and `print-identity-token`, `az account get-access-token`, `op read` and `op item get`, `kubectl config view --raw`, `aws configure get` and `aws configure export-credentials`, `aws ecr get-login-password`, `vault read` and `vault kv get`. The sandbox cannot stop these where the credential lives in the keychain or a helper it allows, so the rule does; it is found before `sudo`'s own caution, so a judge clearing `sudo` never clears it;
  - acting beyond this computer: publishing a package or image, pushing to a remote, merging or releasing on GitHub, changing a cluster, cloud infrastructure, a database or a payments account, deploying.
- **In `ask` mode, commands that only read run without asking:** plain commands such as `ls`, `cat`, `rg`, `wc` and `find` without `-exec` or `-delete`, joined by `&&`, `||`, `;` or `|`, with no redirection or substitution. Git is not among them: a repository's settings can make even `git status` run programs.
- **Everything else** is left to the mode.

These checks are tripwires for mistakes and plain attacks, such as a planted instruction to `git reset --hard`, not a security boundary: a determined program can say the same thing another way. The boundary is the sandbox (`sandbox.md`).

## The judge

A rule cannot tell "push my branch" from a push nobody asked for. In auto mode, a judge can:

- **Firm cautions always ask.** Reading secrets (a secret file, or a command that prints a credential), and commands too tangled to read, never reach the judge. With no one there they are not run, and the model is told it would have shown it secrets and not to get them another way.
- **Soft cautions go to the judge:** every other risky kind above. It is one model call that sees only the person's own messages (never a subagent's brief, never a compaction summary), the project's instructions, the action and why a rule flagged it. It never sees tool output or the model's own words, so nothing the agent read can talk it into anything.
- **It answers run or ask.** Run means the person clearly asked for this action, or for something that plainly needs it; the call runs and is recorded as `approval.requested` and `approval.decided` (`allow_once`) with the judge's reason in the summary. Ask, an unclear answer, an error or no answer means the caution asks as before.
- **Ordinary commands never reach it,** and in `ask` mode it is never consulted.

The judge is the main model unless `GLAVE_JUDGE_MODEL` names another; `off` turns it off. On `evals/judge` (`cargo xtask eval judge --model …`), any of the three default models as judge lowers false alarms on actions the person asked for from 56% to 1–6%, and lets through no case that should ask.

## Untrusted content

Some tool output comes from outside and no one vouches for it: a page `web_fetch` read, the result of an MCP tool whose annotations say `openWorldHint: true` (MCP's flag for a tool that reaches beyond its server, such as a web search), and anything from an MCP server marked `"untrusted": true` in `mcp.json` or named in a policy's `[mcp] untrusted` (`policy.md`). Such output may hold instructions planted to make the agent act.

- **Labelled.** The model is shown it between `<untrusted>` tags, after a line saying it is data, not instructions, and not to follow requests in it or run commands it suggests unless the person asked. A closing tag inside the content is broken up, so the content cannot end the label early. Errors are glave's own words and are not labelled.
- **Remembered.** The call's `tool.call.completed` records `untrusted: true` (`events.md`), so the thread knows it has read such content, and resuming it rebuilds that. A subagent starts knowing what its parent read, and the report of a subagent that read such content is labelled and marks the parent too.
- **Risky actions after it are judged knowing it.** The cautions above still ask in every mode. In auto mode, a soft caution goes to the judge as before, which is also told that the agent has read untrusted content and that asking the agent to do what some content says is not asking for the action; it never sees the content itself. With the judge off, or in `ask` mode, the caution asks as always. Actions that are not cautions are left to the mode: the sandbox, not the label, is the boundary.

`web_fetch` is the one web tool built in (on by default, `[web]` in `config.toml`, `policy.md`): an HTTP GET of an http or https URL, read as Markdown, with the URL it ended at. It reaches beyond the workspace, so in `ask` mode it asks. It refuses private addresses (this computer, private networks, link-local and cloud metadata, carrier NAT, multicast, reserved ranges, and IPv6 forms that carry such an IPv4 address): addresses written in the URL, however spelled (decimal, octal, hex), are checked before anything is sent; names are checked as they resolve, and only the allowed addresses are connected to, so a name cannot resolve one way when checked and another when connected. Redirects (at most 5) are followed by hand and each hop is checked again; no proxy is used. A fetch reads at most 5 MB within 30 seconds, and the model is given at most 50,000 characters; pages are kept for 15 minutes (at most 32). Search is not built in: it comes from remote tools, such as an MCP server for Brave Search or SearXNG.

## Hooks

Hooks are commands the person sets up to run at four points of a turn, under the names and protocol Claude Code and Codex share:

| Point | Runs | A block |
| --- | --- | --- |
| `PreToolUse` | before a tool call, before anyone is asked about it | stops the call; the model's result is "Not run: a hook the person set up stopped this call: <reason>" |
| `PostToolUse` | after a tool call ran, before the model sees its result | adds "[A hook that runs after <tool> says: <reason>]" to the result |
| `UserPromptSubmit` | when the person sends a prompt (not a goal's continuation, not a subagent's brief) | the prompt is refused with its reason (`-32008` in the session API) and never reaches the model |
| `Stop` | once a turn has ended, unless it was cancelled; not in subagents' threads | cannot block: keeping work going until something holds is a goal's job, with a check |

They are written in `config.toml`, the person's (`~/.glave/config.toml`) and a project's (`.glave/config.toml`), and they add up rather than the nearest winning: the person's run first, then the project's, in the order written.

```toml
[[hooks.PreToolUse]]
matcher = "bash"              # a regular expression over the whole tool name; every tool when absent
command = "./scripts/guard.sh"
timeout = 30                  # seconds; 600 when absent

[[hooks.PostToolUse]]
matcher = "edit|write"
command = "cargo fmt --all"
```

- **Running.** Each hook is a bash command run at the workspace root on the workspace's computer, through the computer port, in the sandbox when there is one, with its timeout. It gets one line of JSON on its standard input: `hook_event_name`, `session_id` (the thread), `cwd`, `permission_mode` (`auto` or `ask`), and for the tool points `tool_name`, `tool_input` and `tool_use_id`, plus `tool_response` (`output`, `is_error`) after the tool; `prompt` for `UserPromptSubmit`, `outcome` for `Stop`. Hooks at one point run one after another, and the first that blocks ends the rest.
- **Answering.** Exit 0 lets things go on. Exit 2 blocks, and the hook's output is the reason. On exit 0, output that is a JSON object blocks when it says `"decision": "block"` (with `reason`), or for `PreToolUse` `hookSpecificOutput.permissionDecision` `"deny"` (with `permissionDecisionReason`). The computer port gives one stream, standard output and error together, so a hook answering in JSON writes nothing else. Any other exit code, a timeout, or a hook that cannot start is recorded as failed and things go on.
- **A hook can only narrow.** A blocking hook stops a call or a prompt; nothing a hook says loosens anything. `"permissionDecision": "allow"` or `"ask"` is ignored, a call a hook passed is still asked about exactly as the mode and the cautions say, and no hook changes a call's arguments.
- **Trust and policy.** The person's hooks are theirs. A project's hooks run only once the person trusts the project, each trusted by its whole definition, so a changed command, matcher or timeout asks again; until then they stay off and glave says so to the person (not to the model, which never calls a hook). `[hooks] own = false` in an organization's policy forbids hooks: a project's stay off whatever the person says, and the person's own make glave refuse to start, naming the rule (`policy.md`).
- **Recording.** Every hook that runs is recorded as `hook.ran` (`events.md`). With no hooks, or none that apply, nothing runs and the model is sent exactly what it was sent before.

Later: `additionalContext` (a hook adding text for the model), `updatedInput`, hooks at session start and end and around compaction, and hooks an organization provides.

## No one at the keyboard

Whether someone can answer is a fact about the run, not a mode. With a terminal, glave asks there: once, always in this thread, or no. Without one (scripts, CI, piped input), anything that would ask is not run, and:

- the model is told that no one is available to approve it, not to retry it or do the same thing another way, and to say at the end what still needs a person;
- the output shows the action as `not run: no one here to approve it`, so a log records what was skipped.

So `auto` without a terminal never stops for a question, and `ask` without one never acts beyond what its mode allows.

## Recording

Each turn's configuration (`turn.started`) records the mode, and the banner and every receipt name it: `done · approval: auto · …`.

Every question is recorded with its answer: `approval.requested`, then `approval.decided`, before the call's result (`events.md`). So the log shows who allowed what, and "always in this thread" survives resuming the thread. The question carries the thread and call it is about, so a client of the session API can answer it later (`session.md`).

## Later

- Asking to let one sandboxed command out of the sandbox.
