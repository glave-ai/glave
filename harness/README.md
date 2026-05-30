# harness

Glave's eval harness. Slice 1 is the **deterministic skeleton**: run one task in
an isolated Docker container with a stub (non-model) agent, score it with a
two-sided oracle, and write a flat-file JSONL trace. No model, no network for the
agent — that arrives in slice 2 (gateway → OpenRouter).

All implementation is Rust. The only non-Rust artifacts are eval task fixtures
(in their target language) and Dockerfiles (config).

## Layout

```
harness/
  src/
    main.rs      CLI + run pipeline
    task.rs      the task-spec contract (task.toml)
    runtime.rs   WorkspaceRuntime trait + Docker backend
    oracle.rs    two-sided FAIL_TO_PASS / PASS_TO_PASS verdict
    agent.rs     stub agents (noop, solve); real agent lands in slice 2
    trace.rs     JSONL trace, run-id correlated
  images/<lang>/Dockerfile   per-language base image
tasks/<lang>/<difficulty>/<id>/
  task.toml      spec
  workspace/     code the agent edits   (mounted rw)
  oracle/        tests that judge it     (mounted ro, tamper-proof)
  solution/      reference fix (stub + validation only; never shown to a real agent)
traces/          JSONL run traces (gitignored)
.runs/           ephemeral per-run workspaces (gitignored)
```

## Usage

Run from the repo root (paths are resolved relative to it):

```sh
cargo build --manifest-path harness/Cargo.toml

# build the python base image once
./harness/target/debug/harness build-base --lang python

# prove the oracle discriminates (noop must fail, solve must pass)
./harness/target/debug/harness validate tasks/python/easy/fix-failing-test-001

# run a single task with a chosen stub agent
./harness/target/debug/harness run tasks/python/easy/fix-failing-test-001 --agent solve
./harness/target/debug/harness run tasks/python/easy/fix-failing-test-001 --agent noop
```
