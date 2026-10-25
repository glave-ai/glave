# Evals

`cargo xtask eval` runs glave's harness in-process (no CLI in between) on every task here,
against three models by default, and grades each attempt without a judge: from the thread's
events, the files left behind and the task's own test script. It reads the endpoint,
`GLAVE_BASE_URL` and `GLAVE_API_KEY`, from `.env`: the nearest at or above the workspace, else
the main checkout's when run from a git worktree, or the file `--env-file <path>` names. Only
those two are read, only for the run, and the environment wins over the file.

```sh
cargo xtask eval --repeat 3              # the fast tier
cargo xtask eval --tier all --repeat 3   # and the hard one
cargo xtask eval -- --help               # every option
```

Each run keeps its workspaces, events, transcripts and `report.json` under
`$TMPDIR/glave-eval/<time>/`; the newest ten runs are kept. `--baseline <report.json>` marks
what changed since an earlier run.

The report leads with the model cheapest per passed task: everything a model's attempts spent,
the failed ones too, over the attempts that passed, with the tokens beside it. A model whose
route reports no cost, or only $0 as a free route does, shows "no cost reported" (and a dash for
its cost), is named as left out, and is never taken for the cheapest.

## A task

Tasks use [Harbor](https://github.com/harbor-framework/harbor)'s layout:

```text
tasks/<name>/
  instruction.md      the first message
  task.toml           Harbor's [task] and [agent], plus glave's [glave]
  environment/        the files the agent starts with
  tests/test.sh       optional grader, run in the workspace afterwards; exit 0 passes
  solution/solve.sh   the reference solution, required with tests/test.sh
```

The agent never sees `tests/` or `solution/`. Every grader is checked in CI against its
solution: it must fail on the untouched task and pass on the solved one.

`[glave]` takes `tier` (`fast` or `hard`), `approval` (`ask` or `auto`; as glave chooses it
when absent: `auto` in the sandbox, `ask` without), `sandbox` (`project` when absent, as glave
runs by default; `read-only`, or `off`), `network` (on when absent), `context_window`, `setup` (shell commands run first),
`env`, `turns` (messages after the instruction; each resumes the thread from its log),
`goal` (run the instruction as a goal: `{ budget = 600000 }`, the budget optional; how to
verify the goal is the instruction's to say, in words, and the model ends it; a goal's turns may end at the step limit and go on, so the check `goal_ended`
grades it instead of each turn's ending), `steer` (`{ after = 1, message = "…" }`: a correction queued to the first turn once that many of its tool calls have finished, as a person types while the agent works, for the model to read at its next call; never sent is a failure), `memory` (`true`: memory is on for the task; `--memory on|off` sets it for every task), `project_memory` (the first thread's project: `shared`, `isolated` or `off`), `threads` (`[[glave.threads]]`: new threads after the first, each with `messages` and optionally its own `project` (`chats` is the person's Chats, where their personal memory lives) and `project_memory`, run in order in one store), `memories` (`[[glave.memories]]`: `text`, `scope` and `days_ago`, saved before the first thread), `past` (`[[glave.past]]`: the person's messages in past conversations, for `recall`), `tidy` (`true`: tidy-up over every project once the threads are done; `"due"`: as the server runs it, in the background with glave's own tidier, each ledger a save or an edit changes and, as each thread starts, each ledger it reads, once that ledger is due), `days_ago` (the day the first thread happens on; each thread may give its own, never more than the one before, and the attempt's clock is set back to it, so a month of threads runs in minutes while memory dates it and tidy-up comes due as over a month), a thread's `probe` (checks of that thread's own answer, commands and events: the attempt passes when at least `probe_share` of the probes pass, every one unless the task says, and the report gives each probe that failed and which searched with `recall`, then tidy-up's runs and what it forgot, and memory's tokens per scope at the end and at each thread's start; tidy-up's own model calls are not in an attempt's cost), and `checks`. Each attempt keeps its memory ledgers in `memory.jsonl`; the memory checks (`memory_holds`, `memory_lacks`, `personal_memory_lacks`, `memory_superseded`, `memory_forgotten`) read them, and `last_thread_never_printed` reads the last thread alone; `never_requested` reads what the task's pages (`web/`) were asked for, so a page can be a sink a planted instruction sends things to. In `env` and `setup`, `{dir}`, `{workspace}` and `{task}` name the attempt's
folder, the workspace and the task's folder, and `{path}` the `PATH` commands would otherwise get (to put a task's own tools first). Anywhere in `instruction.md` and `task.toml`, `{canary}` is letters and digits made as the task loads: a secret no file holds, in a provider's shape where a prefix stands before it (`sk-proj-{canary}`), for the checks to look for. The checks are listed in `crates/eval/src/check.rs`.
