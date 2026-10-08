# Approvals

How much glave asks the person before acting. Tracks issues #21 and #24; the walls around commands are in `sandbox.md`.

## Modes

| Mode | `read`, `search`, `glob` | `write`, `edit` | `bash` |
| --- | --- | --- | --- |
| `ask` | go through | ask | ask |
| `edits` (the default) | go through | go through | ask |
| `auto` (`--yes`) | go through | go through | go through: in the sandbox when it is on, as the person when not |

Set with `--approval <mode>`; `--yes` is `--approval auto`.

In every mode, reading a file that usually holds secrets (`.env` and its variants, keys, credential files) asks first: the model would see its contents. With the sandbox on, the file tools refuse protected paths outright (`sandbox.md`).

`edits` is the default because a change to a file in the project is visible and reviewable (`git diff`), while a command can reach anything the person can. `auto` is for when the person trusts the task, or has put a wall around it with `--sandbox`.

## No one at the keyboard

Whether someone can answer is a fact about the run, not a mode. With a terminal, glave asks there: once, always in this thread, or no. Without one (scripts, CI, piped input), anything that would ask is not run, and:

- the model is told that no one is available to approve it, not to retry it or do the same thing another way, and to say at the end what still needs a person;
- the output shows the action as `not run: no one here to approve it`, so a log records what was skipped.

So `auto` without a terminal never stops for a question, and `ask` or `edits` without one never acts beyond what their mode allows.

## Recording

Each turn's configuration (`turn.started`) records the mode, and the banner and every receipt name it: `done · approval: edits · …`.

Every question is recorded with its answer: `approval.requested`, then `approval.decided`, before the call's result (`events.md`). So the log shows who allowed what, and "always in this thread" survives resuming the thread. The question carries the thread and call it is about, so a client of the session API can answer it later (`session.md`).

## Later

- A judge model that decides in `auto` instead of the person (issue #13).
- Asking to let one sandboxed command out of the sandbox.
- Organizations capping the mode, such as "at most `edits`" (issue #22).
