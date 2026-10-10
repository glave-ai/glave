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

`[glave]` takes `tier` (`fast` or `hard`), `approval` (`ask`, `edits`, `auto`), `sandbox`
(`read-only`, `project`), `network`, `context_window`, `setup` (shell commands run first),
`env`, `turns` (messages after the instruction; each resumes the thread from its log),
`goal` (run the instruction as a goal: `{ budget = 600000 }`, the budget optional; how to
verify the goal is the instruction's to say, in words, and the model ends it; a goal's turns may end at the step limit and go on, so the check `goal_ended`
grades it instead of each turn's ending) and `checks`. In `env` and `setup`, `{dir}`, `{workspace}` and `{task}` name the attempt's
folder, the workspace and the task's folder. The checks are listed in `crates/eval/src/check.rs`.
