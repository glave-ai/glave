# Approvals

How much glave asks the person before acting. Tracks issues #21, #24, #78 and #81; the walls around commands are in `sandbox.md`.

## Modes

| Mode | Reads (`read`, `search`, `glob`) | Changes (`write`, `edit`) | Runs (`bash`, most MCP tools) |
| --- | --- | --- | --- |
| `auto` (the default) | go through | go through | go through: in the sandbox when it is on, as the person when not |
| `ask` | go through | ask | ask |

Set with `--approval <mode>`. A tool's effect decides which column it is in; MCP tools that say they only read are reads.

In every mode, reading a file that usually holds secrets (`.env` and its variants, keys, credential files) asks first: the model would see its contents. With the sandbox on, the file tools refuse protected paths outright (`sandbox.md`).

`auto` is the default because almost everyone works that way, and agency means glave acts on the person's behalf and asks only what is truly theirs. What keeps it safe without asking is prevention: commands are read before they run (below), a judge decides what the rules cannot (issue #13), and the sandbox may become the default (issue #12). `ask` is the cautious mode. The `edits` mode of earlier versions (changes go through, commands ask) is retired.

## Commands read before they run

Every `bash` command is parsed with tree-sitter-bash before it runs, as Codex, OpenCode and Claude Code parse it, and every command in it is found: in pipelines and lists, subshells, substitutions, loops, and scripts given to `bash -c` or `eval`. Wrappers such as `sudo`, `env X=1`, `timeout`, `nice`, `nohup` and `xargs` are looked through, and `/bin/rm` is `rm`. Then:

- **Risky commands ask in every mode**, even when the person allowed `bash` for the thread, and with no one there they are not run. The model is told why, and not to get the same result another way. Risky means:
  - deleting what cannot be got back: the root, a top-level folder, the home folder, the project or what holds it, anything outside the project but the temporary folder, or a folder the command cannot name in advance;
  - throwing away uncommitted work or rewriting what others share: `git reset --hard`, `git clean -f`, `git checkout .` and `git checkout -- …`, `git restore` of the work tree, `git stash drop` and `clear`, `git branch -D`, `git push --force` and its kin;
  - running code fetched from the network: `curl … | sh`, `bash <(curl …)`, `sh -c "$(curl …)"`;
  - acting on the computer: `sudo`, formatting or erasing disks, writing to devices, shutting down, changing permissions beyond the project;
  - planting what runs later: shell start-up files, `~/.ssh`, git hooks, launch agents, `crontab`, `launchctl`, `systemctl enable`, global git settings;
  - reading a file that usually holds secrets, as the file tools already ask.
- **In `ask` mode, commands that only read run without asking:** plain commands such as `ls`, `cat`, `rg`, `wc` and `find` without `-exec` or `-delete`, joined by `&&`, `||`, `;` or `|`, with no redirection or substitution. Git is not among them: a repository's settings can make even `git status` run programs.
- **Everything else** is left to the mode.

These checks are tripwires for mistakes and plain attacks, such as a planted instruction to `git reset --hard`, not a security boundary: a determined program can say the same thing another way. The boundary is the sandbox (`sandbox.md`); the judge (issue #13) will weigh what the rules cannot.

## No one at the keyboard

Whether someone can answer is a fact about the run, not a mode. With a terminal, glave asks there: once, always in this thread, or no. Without one (scripts, CI, piped input), anything that would ask is not run, and:

- the model is told that no one is available to approve it, not to retry it or do the same thing another way, and to say at the end what still needs a person;
- the output shows the action as `not run: no one here to approve it`, so a log records what was skipped.

So `auto` without a terminal never stops for a question, and `ask` without one never acts beyond what its mode allows.

## Recording

Each turn's configuration (`turn.started`) records the mode, and the banner and every receipt name it: `done · approval: auto · …`.

Every question is recorded with its answer: `approval.requested`, then `approval.decided`, before the call's result (`events.md`). So the log shows who allowed what, and "always in this thread" survives resuming the thread. The question carries the thread and call it is about, so a client of the session API can answer it later (`session.md`).

## Later

- A judge model that decides in `auto` instead of the person (issue #13).
- Asking to let one sandboxed command out of the sandbox.
- Organizations capping the mode, such as "always `ask`" (issue #22).
