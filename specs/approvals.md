# Approvals

How much glave asks the person before acting. Tracks issues #21, #24 and #78; the walls around commands are in `sandbox.md`.

## Modes

| Mode | Reads (`read`, `search`, `glob`) | Changes (`write`, `edit`) | Runs (`bash`, most MCP tools) |
| --- | --- | --- | --- |
| `auto` (the default) | go through | go through | go through: in the sandbox when it is on, as the person when not |
| `ask` | go through | ask | ask |

Set with `--approval <mode>`. A tool's effect decides which column it is in; MCP tools that say they only read are reads.

In every mode, reading a file that usually holds secrets (`.env` and its variants, keys, credential files) asks first: the model would see its contents. With the sandbox on, the file tools refuse protected paths outright (`sandbox.md`).

`auto` is the default because almost everyone works that way, and agency means glave acts on the person's behalf and asks only what is truly theirs. What keeps it safe without asking is being able to undo (issue #79), with a judge (issue #13) and the sandbox by default (issue #12) to come. `ask` is the cautious mode. The `edits` mode of earlier versions (changes go through, commands ask) is retired.

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
