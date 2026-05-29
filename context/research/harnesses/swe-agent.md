# SWE-agent (and Mini-SWE-agent)

**Identity:** The academic reference harness for autonomous software engineering, whose core thesis is that a well-designed *Agent-Computer Interface* (ACI) — not just a bigger model — is what makes agents solve real GitHub issues, measured rigorously on SWE-bench.
**Source:** https://github.com/SWE-agent/SWE-agent · MIT · Python (>=3.11; tool bundles are bash + small Python scripts) · snapshot `0f4f3bb`
**Family:** research (with a thin product/CLI skin; not a control plane)

## Overview

SWE-agent is the harness from the Princeton/Stanford team that also produced SWE-bench (the canonical issue-resolution benchmark) and SWE-ReX (the execution runtime). Its NeurIPS 2024 paper's central claim is in the title: "Agent-Computer Interfaces Enable Automated Software Engineering." The argument (`docs/background/aci.md`) is that LMs are bad at using a raw shell — `cat`-ing whole files, `sed`-editing blind, drowning in `grep` output — and that the leverage comes from giving the model a small set of *task-shaped* commands with terse, consistent feedback. The repo is explicitly "made for research: simple & hackable by design," governed by a single YAML file, with the stated goal of making it easy to *invent new ACIs* and measure them. The same engine is reused for offensive-security CTFs (EnIGMA mode) and arbitrary coding challenges, so the ACI idea is positioned as general, not SWE-bench-specific.

The posture is benchmark-first throughout. The primary workflow is not "help a developer at a terminal" but "run an LM over N frozen problem instances in parallel containers, emit one patch per instance, and score them." Everything downstream — `run-batch`, per-instance Docker images, `preds.json`, the `sb-cli` evaluation hook, the `.traj` trajectory inspector — exists to support that measured loop. SWE-agent achieved open-source SOTA on SWE-bench (and SOTA overall with Claude 3.7 in early 2025), which is the credential that makes its ACI choices worth studying.

Critically for Glave: the project itself has now declared SWE-agent **superseded by Mini-SWE-agent** (`docs/index.md`, `README.md`, the warning banner in `docs/overrides/main.html`). Mini-SWE-agent is a separate repo (not in this clone) that reportedly hits 65% on SWE-bench Verified in ~100 lines of Python by stripping the ACI down to a single primitive — let the model run *bash* and nothing else, no special editor, no tool registry, linear history, subprocess execution. The maintainers' public position is that frontier models no longer need the elaborate ACI scaffolding; the simple loop matches it. This is the single most important signal in the whole reference: the team that *invented* the elaborate ACI is now telling you to delete most of it.

The two together bracket the design space — maximal ACI engineering (SWE-agent 1.0) vs. radical subtraction (Mini-SWE-agent) — and the empirical claim that they perform the same. That is exactly Glave's "subtraction, decided by evals" doctrine, validated by the originators.

## Architecture

**Control loop.** The agent core is `sweagent/agent/agents.py` (~1300 lines). `DefaultAgent.run()` is a simple `while not step_output.done: self.step(); self.save_trajectory()` loop. Each `step()` calls `forward_with_handling()`, which calls `forward()` (one model query → parse → execute in env → observe), wrapped in a large try/except that converts every failure mode into either a *requery* (format error, blocked action, bash-syntax error → re-ask the model up to `max_requeries`, default 3) or an *exit-with-autosubmission* (cost/context/timeout/forfeit/runtime error → try to extract a patch anyway and stop). `step()` then appends to both the message *history* (sent to the model) and the *trajectory* (saved to disk), and updates an `info` dict with submission, exit_status, edited-files context, and model cost stats.

**The Agent-Computer Interface (the IP).** Tools are not hardcoded — they are *bundles* loaded from `tools/<name>/` directories, each with a `config.yaml` declaring commands (name, signature, docstring, typed arguments) and a `bin/` of executables (mostly bash, some Python) uploaded into the container and put on `$PATH`. `sweagent/tools/commands.py` defines `Command`/`Argument` Pydantic models; `bundle.py` loads them; `tools.py` (`ToolConfig`/`ToolHandler`) assembles the active command set, generates docs, and converts each command into an OpenAI/LiteLLM function-calling schema (`get_function_calling_tool()`). The classic ACI bundles in this clone:
- `windowed` — a stateful file viewer: `open`, `goto`, `scroll_up/down`, `create`, showing a 100-line window (`state_command: _state` persists the open file/line to `/root/state.json`).
- `windowed_edit_linting` / `windowed_edit_replace` / `windowed_edit_rewrite` — three *editor variants* (line-range edit terminated by `end_of_edit`; search/replace; full rewrite). The linting variant is the famous one: it runs a syntax check and **refuses an edit that would break the file**, forcing the model to fix indentation before the change lands.
- `search` — `find_file`, `search_dir`, `search_file`, deliberately listing only the *files* that matched (per `aci.md`, richer per-match context "proved too confusing for the model").
- `filemap` — print a Python file with long function bodies skipped (cheap structural overview).
- `submit` / `review_on_submit_m` — explicit completion (below).
- `forfeit` (`exit_forfeit`), `edit_anthropic` (str-replace editor à la Anthropic computer-use), `diff_state`, `image_tools`, `web_browser`, `multilingual_setup`, `registry` (a key/value store, `/root/.swe-agent-env` + `/root/state.json`, that bash tools read/write to share state across turns).

A `ToolFilterConfig` blocklist bans interactive commands (`vim`, `nano`, `less`, `python` standalone REPL, `tail -f`, etc.) so the agent can't hang on a pager or interactive prompt — empirical scar tissue encoded as policy.

**Action parsing.** `sweagent/tools/parsing.py` offers pluggable parsers selected by `parse_function.type`: `function_calling` (default, native tool calls — exactly one per turn enforced), `thought_action` (discussion + a fenced ```` ``` ```` command block), `xml_thought_action`, `json`, `action`/`action_only`, `edit_format`, and bash-codeblock parsers. So the *same* ACI works whether the model speaks native tool-calls or plain text, which is what lets the harness benchmark across providers.

**Environment / sandbox.** `sweagent/environment/swe_env.py` (`SWEEnv`) is a thin façade over **SWE-ReX** (`swe-rex>=1.4.0`), a separate runtime providing `AbstractDeployment` backends (Docker default, plus Local, Modal, Daytona, Dummy). `SWEEnv` exposes `communicate()` (run in the persistent bash session), `read_file`/`write_file`, `execute_command` (out-of-session subprocess), and lifecycle (`start`/`reset`/`hard_reset`/`close`). `reset()` copies the repo, checks out the base commit, and runs reset commands. The agent never touches Docker directly — all execution is mediated, which is how the same loop runs locally or on Modal/cloud.

**YAML config.** A single Pydantic-validated YAML tree (`config/default.yaml` etc.) defines everything: `agent.templates` (system/instance/next-step Jinja templates, observation-truncation template at 100k chars, error templates), `agent.tools` (bundles, blocklist, parser, timeouts, `submit_command`, `env_variables`), `agent.model` (LiteLLM model id, temperature, `per_instance_cost_limit`, `total_cost_limit`, `per_instance_call_limit`), and `agent.history_processors`. CLI flags map onto the same tree (`--agent.tools.parse_function.type=thought_action`). `config/benchmarks/` holds dated, frozen experiment configs (e.g. `250522_anthropic_filemap_simple_review.yaml`) — experiments are version-pinned files, not code edits.

**Where state lives.** Two parallel structures: *history* (the message list actually sent to the model, mutated by history processors) and *trajectory* (`List[TrajectoryStep]` of action/observation/response/thought/state/query/extra_info, written to `<instance_id>.traj` as indented JSON every step). In-container state lives in `/root/state.json` and `/root/.swe-agent-env` (the registry). The patch is materialized at `/root/model.patch`.

**Context management.** `sweagent/agent/history_processors.py`: `LastNObservations` (elide all but the last N observations), `ClosedWindowHistoryProcessor` (collapse stale file-window observations), `TagToolCallObservations` (tag-based elision), `CacheControlHistoryProcessor` (mark the last N messages with Anthropic `ephemeral` cache_control for prompt caching — directly a cost lever), `RemoveRegex`, `ImageParsingHistoryProcessor`. These are composable and configured in YAML.

**Best-of-N / review (`sweagent/agent/reviewer.py`).** A `RetryAgent` can run multiple attempts and select among them via two retry-loop strategies: `ScoreRetryLoop` (an LM *reviewer* scores each submission against an `accept_score`; keeps sampling until accepted or `max_attempts`/`cost_limit`) and `ChooserRetryLoop` (generate N solutions, a `Preselector` + `Chooser` LM picks the best). Both are explicitly budget-aware (`cost_limit`, `min_budget_for_new_attempt`).

**SWE-agent vs. Mini-SWE-agent.** SWE-agent (this clone) = the full machine: bundle/registry tool system, stateful windowed editor with linting, pluggable parsers, history processors, reviewer/best-of-N, SWE-ReX multi-backend deployment, hooks at agent/env/run levels, inspector UI. Mini-SWE-agent = the antithesis: one tool (bash), subprocess execution (no Docker dependency required, no SWE-ReX), linear appended history, ~100 lines, and per the maintainers the *same* SWE-bench-Verified score (~65%). Mini exists because frontier models stopped needing the scaffolding.

## Feature Inventory

**Agent-Computer Interface (ACI).** Constrained command grammar over the raw shell. Each command has a name, a `signature` (`open "<path>" [<line_number>]`), a docstring (often with worked examples and "tips" baked in, e.g. the edit tool's whitespace warnings), and typed `arguments` with `required`/optional ordering, enums, and `argument_format` (Jinja) controlling how values are rendered into the bash invocation. Multi-line commands use an `end_name` heredoc terminator (`end_of_edit`). Empty output is replaced with the canonical "Your command ran successfully and did not produce any output." Observations are hard-truncated at 100k chars with a note telling the model to use `head`/`grep`/redirects.

**Tool / command bundles.** Filesystem-defined, hot-swappable: `tools/<name>/{config.yaml, bin/*, install.sh, lib/}`. `install.sh` runs per-bundle at setup; `ToolHandler._install_commands` uploads bundles, chmods bins, prepends to `$PATH`, then verifies every command resolves via `which`. Duplicate tool names across bundles raise an error. `hidden_tools` lets a bundle ship a command without exposing it to the model.

**`submit` as explicit completion.** `submit` is the harness-visible endpoint, not "the model stops talking." `tools/submit/bin/submit` does `git add -A && git diff --cached > /root/model.patch` then prints the patch wrapped in `<<SWE_AGENT_SUBMISSION>>` sentinels; the agent's `check_for_submission_cmd()` watches observations for that sentinel and reads back `/root/model.patch`. The `review_on_submit_m` variant makes submit a *multi-stage reflection gate*: the first `submit` doesn't submit — it injects a `SUBMIT_REVIEW_MESSAGES` checklist (re-run repro script, revert test files, remove repro script) and only `submit -f` (or completing the stages) truly submits. On most errors the harness performs *auto-submission*: it tries to extract a patch from the dead/timed-out environment so a partial solution is still scored.

**Environment / sandbox abstraction.** SWE-ReX deployments: Docker (default, per-instance images), Local, Modal, Daytona, Dummy. Persistent bash session per instance; file IO and out-of-session command execution; configurable execution/install/total timeouts (`execution_timeout=30`, `install_timeout=300`, `total_execution_timeout=1800`, `max_consecutive_execution_timeouts=3`). Repo reset to base commit between attempts.

**YAML configuration (experiment knobs).** Single typed tree (Pydantic, `extra="forbid"` in places) covering templates, tools, model, history processors, deployment, problem statement. Dated benchmark configs as frozen experiments. CLI overrides map to the same schema.

**Trajectory / trace logging.** Every step is recorded to `<instance_id>.traj` (pretty JSON) with action, observation, model response, thought, env state, the exact `query` sent, execution time, and `extra_info`. Plus per-instance logs, `preds.json` (instance_id → patch), and the LM cost/token stats in `info["model_stats"]`. The **inspector** (`sweagent/inspector/server.py`, `run inspector`) is a web viewer that renders `.traj` files alongside the problem statement, gold/test patches, and the SWE-bench results report, with a pass/fail status emoji per trajectory. `run replay` re-runs a saved trajectory; `run-traj-to-demo` converts a trajectory into a few-shot demonstration.

**Evaluation harness & SWE-bench integration.** `run-batch` (`sweagent/run/run_batch.py`) runs many instances with `num_workers` parallelism, `redo_existing`, instance `filter`/`slice`/`shuffle`, and a `random_delay_multiplier` to avoid thundering-herd on container/API startup. `SWEBenchInstances` (`batch_instances.py`) loads the HuggingFace datasets (lite/verified/multimodal subset map), pins each instance to its per-instance Docker `image_name`, and constructs the problem statement (including issue images for multimodal). `SweBenchEvaluate` run-hook submits `preds.json` to the official scorer via `sb-cli` and writes `results.json`; `continuous_submission_every` enables streaming evaluation mid-run. `merge-preds`, `extract-pred`, `compare-runs`, `quick-stats` round out the eval tooling. Run-level hooks also include `SaveApplyPatchHook` and `OpenPRHook` (turn a solved issue into an actual PR via `gh`).

**Provider & model routing.** All model access goes through **LiteLLM** (`sweagent/agent/models.py`), so any LiteLLM-supported provider works. `GenericAPIModelConfig` exposes temperature, top_p, `per_instance_cost_limit`, `total_cost_limit`, `per_instance_call_limit`, `completion_kwargs`, retry config, LiteLLM fallback lists, and a custom model-cost/tokenizer registry for local/unlisted models. `InstanceStats` tracks `instance_cost`, `tokens_sent`, `tokens_received`, `api_calls` per instance; `GlobalStats` aggregates total cost. Special models: `HumanModel`/`HumanThoughtModel` (human-in-the-loop play), `ReplayModel`, `InstantEmptySubmitModel` (testing). Cost limits raise `CostLimitExceededError`/`TotalCostLimitExceededError` to stop the loop.

**Mini-SWE-agent (simplified variant).** Separate repo. One tool = bash; subprocess execution (no Docker/SWE-ReX dependency required); linear, fully-appended history; ~100 lines; reportedly matches SWE-agent on SWE-bench Verified (~65%). The official recommendation for new work. Demonstrates that the elaborate ACI was a function of weaker models.

**Anything unique.** Linting-gated edits (reject syntactically-broken edits before they land); the registry as a cross-turn KV state store for bash tools; multi-stage `review_on_submit` self-check; auto-submission on error so partial work is still scored; EnIGMA CTF mode reusing the same engine; hooks at three layers (agent, environment, run).

## Standout / Unique

- **The ACI thesis itself, empirically earned.** Window-100 file viewer, search-lists-files-only, linting-gated edits, blocked interactive commands, canonical empty-output message — each is a documented finding, not a guess (`docs/background/aci.md`). This is a catalogue of "things that measurably helped a coding agent."
- **`submit` as a harness-owned completion endpoint** with sentinel-delimited patch extraction, plus a *staged review gate* and *auto-submission on failure*. Completion is a declared, verifiable event, not "the model went quiet."
- **Eval rigor as a first-class subsystem:** per-instance pinned images, parallel batch runner, official `sb-cli` scoring hook, `.traj` traces, a web inspector, replay, and run-comparison tools. The harness is built to be *measured*, autonomously.
- **Best-of-N with LM reviewer/chooser under explicit budget caps** — turns "spend more to be more correct" into a tunable, cost-bounded knob.
- **The self-deprecating Mini-SWE-agent pivot:** the inventors proving, via the benchmark, that radical subtraction matches their elaborate design on current models.

## Pros

- Cleanly separates ACI design (filesystem bundles + YAML) from the control loop — new tools/experiments require no core code changes.
- Provider-agnostic via LiteLLM; same loop benchmarks across models and parsers (native tool-calls vs. text).
- Mediated execution (SWE-ReX) gives portable local/cloud/Modal deployment without touching the agent.
- Genuinely strong, reproducible eval methodology with full traces, cost accounting, and an inspector — the gold standard reference for autonomous scoring.
- Pydantic-typed config with frozen, dated experiment files; robust error handling that degrades to a scored partial patch.
- Explicit per-instance and total cost/call limits baked into the model layer.

## Cons / Limitations

- **Research-first, not a product control plane.** No billing, no auth/RBAC, no multi-tenant policy, no durable session service, no permission UX. It's a batch experiment runner with a thin CLI.
- **Self-declared obsolete.** Maintainers say use Mini-SWE-agent; the elaborate ACI in this clone is largely a monument to a weaker-model era.
- **Python + bash bundles**, loosely typed at the tool boundary (commands are strings parsed by regex/heredoc; multi-line quoting is fiddly). Far from Glave's strict-static-typing preference.
- **State scattered** across history vs. trajectory vs. in-container `state.json`/registry; the patch is reconstructed from `git diff`, not a structured edit log.
- **Container-per-instance is heavyweight**; the autonomy is real but the security model is "trust the image and the repo," not the contained-untrusted-code posture Glave requires for the Pro tier.
- Sandbox security is delegated to SWE-ReX/Docker; no built-in network isolation/cap-dropping/scanning story in-repo.

## Lessons for Glave (econo-frontier, feature-parity lens)

- **Copy:**
  - *Explicit, harness-owned `submit`/completion endpoint with verification.* Hope's task nodes should declare completion through a server-visible endpoint that materializes a structured artifact (diff/patch), exactly as `submit` writes `/root/model.patch` behind a sentinel — never "the model stopped."
  - *Trajectory traces as first-class artifacts* (action/observation/thought/exact-query/state/cost-per-step in one record). This is the substrate for the flywheel. Add the inspector + replay + run-compare pattern.
  - *Eval rigor*: pinned per-instance environments, parallel batch runner with filter/slice/shuffle, autonomous scoring hook, continuous mid-run scoring. This is precisely Glave's zero-HIL measured loop.
  - *Cost limits and per-step cost accounting at the model layer* (`instance_cost`, tokens, api_calls) — directly feeds cost-per-correct-task.
  - *Auto-submission on error* so partial work is still scored, so the eval gradient isn't all-or-nothing on crashes.
- **Adapt:**
  - The bundle/registry idea is good (tools as data, not code) but Glave should make tools *typed actions at the harness boundary* with schema-validated args (Pi-style TypeBox / Rust types), not bash scripts parsed by regex. Keep the "tool = declarative config + verifier" shape; drop the stringly-typed bash plumbing.
  - The ACI findings (windowed reads, search-lists-files, lint-gated edits, blocked interactive commands, canonical empty-output) should be *re-validated against the real Hope model via fast evals*, not copied blindly — Mini-SWE-agent is the warning that these may be net-negative cost/complexity on a strong model.
  - History processors (cache_control, last-N elision, closed-window collapse) map onto Glave's first-class context budget manager — but as measured policy, not last-minute hacks.
  - Best-of-N reviewer/chooser under a budget cap is a clean way to expose the cost↔correctness tradeoff as a tunable; adapt as a node-level "spend more to raise P(correct)" policy.
- **Skip:**
  - The full elaborate ACI as a starting point. Start from the Mini-SWE-agent end (minimal action surface, decide additions by eval), which is also Glave's stated subtraction doctrine.
  - The CLI/inspector UX as product — it's a research viewer; Glave's surface is server-owned traces/dashboards.
  - SWE-ReX as-is and Docker-per-instance trust model — Glave needs the stronger isolation posture (no-net, drop-caps, ephemeral/disposable VM, digest-pinned mirrored images) for untrusted Pro images and untrusted repo/model code.
  - Python implementation; the design ideas port to Rust, the code doesn't.
- **Cost angle:** SWE-agent already treats $ as a top-level control variable: per-instance/total cost limits, prompt-cache history processor, best-of-N gated by `min_budget_for_new_attempt`/`cost_limit`, and per-step token/cost stats in every `.traj`. Glave's cost-per-correct-task metric is computable directly from this shape: `total_cost` (from `GlobalStats`) over the count of `submitted` instances that the autonomous scorer marks `resolved`. Mini-SWE-agent's whole pitch is *cost* (and complexity) reduction at equal correctness — i.e., the same model, far fewer scaffolding tokens. That is the econo-frontier move in miniature.
- **Flywheel/eval notes (KEY):** SWE-agent *is* the reference implementation of Glave's flywheel mechanics, minus the autonomy-at-scale and the strict typing. The loop is: (1) hypothesis = a YAML config (prompt/tools/parser/processors) frozen as a dated file in `config/benchmarks/`; (2) run it over a fixed instance set with `run-batch`; (3) emit `.traj` traces + `preds.json` + per-step cost; (4) score *fully autonomously* via `sb-cli` (no human judge — exactly Glave's zero-HIL hard constraint); (5) compare runs with `compare-runs`/`quick-stats`. The trajectory trace is the unit of learning — it captures the exact model query, the action, the observation, the state, the cost, and the eventual resolved/unresolved verdict, which is everything a self-improvement loop needs to attribute success to a config change. The decisive lesson is that **measurement is the product**: the same team, using their own eval harness, discovered that subtracting the ACI down to bare bash held SWE-bench score constant while cutting complexity and tokens. Glave should build the trace+autoscore+compare substrate first, treat every prompt/tool as a config-level hypothesis, and let the measured cost-per-correct gradient — not intuition — decide what stays in Hope's harness. SWE-agent confirms Glave's existing memory note that the bottleneck is signal/measurement, not ideas.

## Source

Key paths in `refs/harnesses/swe-agent/`:
- Agent loop & retry/autosubmit/handling: `sweagent/agent/agents.py`
- ACI tool config/handler & blocklist: `sweagent/tools/tools.py`; command/argument model: `sweagent/tools/commands.py`; bundle loader: `sweagent/tools/bundle.py`; parsers: `sweagent/tools/parsing.py`
- Tool bundles (the actual ACI): `tools/windowed/`, `tools/windowed_edit_linting/` (+ `_replace`/`_rewrite`), `tools/search/`, `tools/filemap/`, `tools/submit/`, `tools/review_on_submit_m/`, `tools/forfeit/`, `tools/edit_anthropic/`, `tools/registry/`
- Environment/sandbox (over SWE-ReX): `sweagent/environment/swe_env.py`
- Model routing & cost stats (LiteLLM): `sweagent/agent/models.py`
- History/context processors: `sweagent/agent/history_processors.py`
- Best-of-N reviewer/chooser: `sweagent/agent/reviewer.py`
- Batch runner / SWE-bench loading: `sweagent/run/run_batch.py`, `sweagent/run/batch_instances.py`; eval hook: `sweagent/run/hooks/swe_bench_evaluate.py`; PR/apply hooks: `sweagent/run/hooks/{open_pr,apply_patch}.py`
- Trajectory types: `sweagent/types.py`; inspector UI: `sweagent/inspector/server.py`; CLI dispatch: `sweagent/run/run.py`
- Config tree: `config/default.yaml`, `config/benchmarks/*.yaml`
- ACI thesis & architecture docs: `docs/background/aci.md`, `docs/background/architecture.md`; Mini-SWE-agent supersession notice: `docs/index.md`, `README.md`
- `pyproject.toml` (deps: `litellm`, `swe-rex>=1.4.0`, `datasets`)

Upstream:
- Repo: https://github.com/SWE-agent/SWE-agent
- Paper (ACI): https://arxiv.org/abs/2405.15793
- Mini-SWE-agent: https://github.com/SWE-agent/mini-swe-agent
- SWE-ReX runtime: https://github.com/SWE-agent/SWE-ReX
- SWE-bench: https://github.com/SWE-bench/SWE-bench
