# Evals

`cargo xtask eval` runs glave's harness in-process (no CLI in between) on every task here,
against three models by default, and grades each attempt without a judge: from the thread's
events, the files left behind and the task's own test script. Load the endpoint first:

```sh
set -a; source .env; set +a
cargo xtask eval --repeat 3              # the fast tier
cargo xtask eval --tier all --repeat 3   # and the hard one
cargo xtask eval -- --help               # every option
```

Each run keeps its workspaces, events, transcripts and `report.json` under
`$TMPDIR/glave-eval/<time>/`; the newest ten runs are kept. `--baseline <report.json>` marks
what changed since an earlier run.

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
grades it instead of each turn's ending), `memory` (`true`: memory is on for the task; `--memory on|off` sets it for every task), `space_memory` (the first thread's space: `shared`, `isolated` or `off`), `threads` (`[[glave.threads]]`: new threads after the first, each with `messages` and optionally its own `space` and `space_memory`, run in order in one store) and `memories` (`[[glave.memories]]`: `text` and `scope`, saved before the first thread), and `checks`. Each attempt keeps its memory ledgers in `memory.jsonl`; the memory checks (`memory_holds`, `memory_lacks`, `personal_memory_lacks`, `memory_superseded`, `memory_forgotten`) read them, and `last_thread_never_printed` reads the last thread alone; `never_requested` reads what the task's pages (`web/`) were asked for, so a page can be a sink a planted instruction sends things to. In `env` and `setup`, `{dir}`, `{workspace}` and `{task}` name the attempt's
folder, the workspace and the task's folder, and `{path}` the `PATH` commands would otherwise get (to put a task's own tools first). The checks are listed in `crates/eval/src/check.rs`.
