# Aider

**Identity:** A terminal-native "AI pair programmer" that edits your local git repo directly, built around structural repo maps, text-based edit formats, and automatic git commits.
**Source:** https://github.com/Aider-AI/aider · Apache-2.0 · Python (3.10–3.14; tree-sitter `.scm` query grammars) · snapshot `5dc9490`
**Family:** terminal

## Overview

Aider is a single-file-process Python CLI (`aider = "aider.main:main"`) that turns an LLM into a pair programmer working against an existing git repository. The user adds files "to the chat," states a request, and the model responds with edits expressed in a *text* edit format (search/replace blocks, unified diffs, whole-file, or OpenAI-style patch); Aider parses those edits, applies them to disk, lints them, optionally runs tests, and auto-commits each change with a generated commit message. It is explicitly a developer-in-the-loop tool, not an autonomous agent: the default posture is "propose edits, apply, commit, show diff," with the human steering each turn.

The design posture is *deterministic harness, model-driven content*. Almost everything except "what to write" is owned by Python code: which files are in context, how the repo map is built and token-budgeted, how edits are parsed and fuzzily reconciled against the real file, when to commit, when to lint, when to reflect on an error. The model is given a strict output grammar (the edit format) and a curated context window, then its output is mechanically validated and applied. There are no model-callable "tools" in the modern function-calling sense for the primary flow — the edit format *is* the tool surface, parsed out of free text.

Aider is unusually empirical and benchmark-driven. The repo ships a full `benchmark/` harness and the project tracks an edit-format leaderboard; model defaults (edit format, repo-map on/off, reminder placement, examples-as-system-message) are tuned per-model in `models.py` based on measured pass rates. The README brags about a "Singularity" stat (percent of each release written by Aider itself). This is a harness whose authors decide design by measurement, not vibes — directly aligned with Glave's doctrine.

Audience: individual developers in a terminal who already use git, want tight control and human-readable diffs, and want to use almost any model (cloud or local) via LiteLLM. It scales down to scripting (`--message`, Python API) but is not a hosted/server platform.

## Architecture

The control loop lives in `aider/coders/base_coder.py` (the `Coder` base class, ~86KB). A "Coder" is the harness instance for one edit format; subclasses in `aider/coders/` differ mainly in their prompts and their `get_edits`/`apply_edits` implementations. `Coder.create()` is a factory: given a model + `edit_format` string it picks the matching subclass, and it can *clone* state from a prior coder (`from_coder=`) so you can switch modes (e.g. ask → code, or architect → editor) mid-session while carrying files, history, costs, and commit hashes forward. When the edit format changes it summarizes the old history so the new model isn't confused by the prior format's syntax.

A turn flows like this (`run_one` → `send_message`):
1. `init_before_message` resets reflection state (`reflected_message=None`, `num_reflections=0`).
2. `preproc_user_input` handles slash commands, URL detection, and `/`-command dispatch.
3. `format_messages` / `format_chat_chunks` assembles the prompt as an ordered `ChatChunks` dataclass: system → examples → readonly_files → repo (the repo map) → done (summarized history) → chat_files (full editable file contents) → cur (current exchange) → reminder. Token budgeting and prompt-cache markers are applied here.
4. `send` calls the model via LiteLLM (`model.send_completion`), streaming by default, and accumulates `partial_response_content` (and optionally a function_call).
5. After the reply: `check_for_file_mentions` may detect filenames the model named and offer to add them (turning into a reflection); `reply_completed` is a hook (used by ask/architect/context modes).
6. `apply_updates` → `get_edits` (parse the edit format) → `apply_edits_dry_run` → `prepare_to_edit` → `apply_edits` (write to disk). Parse/apply failures set `self.reflected_message` to the error text.
7. If files were edited: `auto_commit` makes a git commit (LLM-authored attribution), and `move_back_cur_messages` flushes the exchange into history (kicking off background summarization if too big).
8. If `auto_lint`: `lint_edited` runs the linter on changed files; lint errors prompt "fix?" and become a reflection.
9. `run_shell_commands` handles any shell commands the model suggested (confirmation-gated).
10. If `auto_test`: runs the test command; failures prompt "fix?" and become a reflection.

**Reflection** is the autonomy primitive: any of the above can set `reflected_message`, and `run_one` loops by feeding that message back as the next user turn, up to `max_reflections = 3`. This is how Aider self-corrects malformed edits, missing files, lint errors, and test failures without a human — a bounded, deterministic agent loop rather than open-ended tool calling.

**State** lives in the `Coder` instance (`cur_messages`, `done_messages`, `abs_fnames`, `abs_read_only_fnames`, `aider_commit_hashes`, cost counters) plus on disk: the **git repo is the source of truth for code**, and chat/input/LLM history are append-only markdown/text files (`.aider.chat.history.md`, `.aider.input.history`, `--llm-history-file`). There is no DB; sessions are reconstructable from git + the markdown history.

**Deterministic vs model-driven:** deterministic = file selection, repo-map construction and ranking, edit parsing + fuzzy matching, git commit/attribution, lint/test invocation, reflection control, token budgeting, prompt caching. Model-driven = prose reasoning, which edits to make, commit-message text (a weak/secondary model), and shell-command *suggestions* (never auto-run without confirm).

## Feature Inventory

### Project / context discovery
- **Repo map** (`aider/repomap.py`) — the signature feature. Uses **tree-sitter** (`aider/queries/tree-sitter-language-pack/*.scm`, ~30+ languages) to extract definitions and references (`Tag = (rel_fname, fname, line, name, kind)`), builds a **`networkx` MultiDiGraph** of symbol references across files, runs **PageRank** to rank symbols/files, and emits a token-budgeted outline of the most relevant code (`get_ranked_tags_map`). Personalization biases rank toward files already in chat and toward identifiers/files **mentioned in the current message** (`mentioned_fnames`, `mentioned_idents`). Map size adapts: with no files in chat it expands the budget (`map_mul_no_files`) to give a broader view. Tags are cached on disk (`load_tags_cache`/`save_tags_cache`, keyed by mtime).
- **Conventions / instruction files** — no bespoke `CLAUDE.md` primitive; instead `--read FILE` adds read-only context files (a `CONVENTIONS.md` is the documented idiom), and `--load` runs a file of `/`-commands at startup. Read-only files get their own prompt prefix ("Do not edit these files!").
- **File mentions** — if the model names a repo file in prose, Aider offers to add it (`get_file_mentions`/`check_for_file_mentions`), and if it pastes a URL, offers to scrape it (`check_for_urls` → `cmd_web`).
- **`/context` mode** (`context_coder.py`) — a dedicated coder whose only job is to read the repo map and reply with the set of files that *need editing*; it reflects until the mentioned file set stabilizes, then those files are auto-added.

### Tool / action / command grammar
There is no JSON tool-calling loop for editing. The user-facing grammar is **slash commands** (`aider/commands.py`, ~40 commands): `/add`, `/drop`, `/read-only`, `/ls`, `/clear`, `/reset`, `/tokens`, `/diff`, `/undo`, `/commit`, `/git`, `/lint`, `/test`, `/run` (run a shell cmd, optionally add output on nonzero exit), `/map`, `/map-refresh`, `/model`, `/editor-model`, `/weak-model`, `/models`, `/chat-mode`, `/ask`, `/code`, `/architect`, `/context`, `/web`, `/voice`, `/paste` (clipboard image/text), `/copy`, `/copy-context`, `/load`, `/save`, `/settings`, `/report`, `/help`, `/editor`, `/think-tokens`, `/reasoning-effort`, `/multiline-mode`, `/exit`. The *model's* action surface is the edit format plus optional ```bash``` shell-command suggestions.

### Edit formats (core differentiator)
Each is a `Coder` subclass with its own prompt + parser:
- **`diff` (EditBlockCoder)** — the default for strong models. SEARCH/REPLACE blocks: filename line, fenced block, `<<<<<<< SEARCH` / `=======` / `>>>>>>> REPLACE`. Parser (`editblock_coder.py`) is forgiving: regex tolerates 5–9 angle brackets, and matching falls back through `perfect_replace` → whitespace-flexible → `replace_most_similar_chunk` (difflib `SequenceMatcher`) → `try_dotdotdots` (elision) → leading-whitespace reconciliation. Empty SEARCH = create new file. Failed blocks produce a detailed error that becomes a reflection.
- **`diff-fenced`** — same but filename goes inside the fence (tuned for Gemini).
- **`udiff` / `udiff-simple`** — unified-diff hunks applied via `search_replace.py`'s `flexible_search_and_replace` (multiple preproc passes); errors distinguish "no match" vs "not unique."
- **`whole` (WholeFileCoder)** — model returns entire file contents (default for weak models; simplest, most token-hungry).
- **`patch` (PatchCoder)** — OpenAI "V4A" `*** Begin Patch` / `*** End Patch` format (`patch_coder.py`, ~30KB).
- **`*-func` coders** — function-call/JSON variants (mostly disabled/legacy).
- **Editor sub-formats** (`editor-diff`, `editor-whole`, `editor-diff-fenced`) — stripped-down prompts used as the *executor* under architect mode.

### Chat modes (multi-coder orchestration)
- **`code`** (default) — propose-and-edit.
- **`ask`** — answer questions, never edit.
- **`architect`** (`architect_coder.py`) — **two-model pattern**: a strong "architect" model plans the change in prose, then a second **editor model** (possibly a cheaper/different model and a different edit format) is spun up via `Coder.create(from_coder=self)` to actually produce and apply the edits. Confirmation-gated unless `--auto-accept-architect`. This separates reasoning from edit-syntax compliance and lets you pay top-model rates only for planning.
- **`help`** — RAG over Aider's own docs.

### Permissions & safety
- Confirmation gates via `io.confirm_ask` with `ConfirmGroup` (apply-to-all) on: adding mentioned files, scraping URLs, running suggested shell commands, fixing lint/test errors. `--yes-always` auto-confirms (scripting). `suggest_shell_commands` can be disabled; shell commands are *never* run without explicit confirm by default.
- `--dry-run` applies nothing. `--git-commit-verify` controls hook bypass. `is_file_safe`/`allowed_to_edit` checks and `--subtree-only`. Repo sanity check (`--skip-sanity-check-repo`).
- This is a **trust-the-local-user** model — there is no sandbox, no capability policy, no network isolation. Safety = git rollback + human confirm.

### Sessions & state / persistence
- Git is the persistence + rollback layer; `/undo` reverts the last Aider commit (`last_aider_commit_hash`, `aider_commit_hashes` tracked). Markdown chat history (`--restore-chat-history` reloads it). `/save` and `/load` serialize the file set + settings as a replayable command script. No multi-session DB or server.

### Context management & token budgeting
- `ChatChunks` orders context deterministically and computes per-chunk token counts; the system reminder is included only if there's room under `max_input_tokens`.
- **History summarization** (`ChatSummary`, run in a background thread via `summarize_start`/`summarize_worker`) compacts `done_messages` when they exceed `--max-chat-history-tokens`; a weak model does the summarizing.
- **Prompt caching** (`add_cache_control_headers`) places Anthropic `cache_control: ephemeral` markers at chunk boundaries (system/examples, repo+readonly, chat_files) and supports **cache warming pings** (`--cache-keepalive-pings`, background thread) to keep the prefix warm. Repo-map and read-only files are deliberately ordered *before* volatile chat files so the cacheable prefix is maximized.
- Repo-map token budget is its own knob (`--map-tokens`, `--map-multiplier-no-files`, `--map-refresh`).

### Verification / completion (lint/test loop, auto-commit)
- **Auto-commit** (`auto_commit`): every applied edit becomes a git commit with an LLM-generated message (weak/secondary model). Rich **attribution policy** (`repo.py`): `--attribute-author`/`--attribute-committer` rewrite the name to "You (aider)", `--attribute-co-authored-by` adds a `Co-authored-by: aider (<model>)` trailer, plus message prefixes. `--dirty-commits` commits pre-existing dirty state before editing so AI edits are isolated.
- **Auto-lint** (`auto_lint=True` default): `linter.py` uses tree-sitter for a `basic_lint` syntax check across languages, `py_compile` + `flake8` for Python, and per-language `--lint-cmd LANG:cmd`. Lint output is fed back as a reflection.
- **Auto-test** (`auto_test=False` by default): runs `--test-cmd`; failure output becomes a reflection. Both lint and test are bounded by `max_reflections`.
- Completion is implicit: a turn ends when no reflection is pending. There is no explicit `submit`/`done` action.

### Provider & model routing
- All model I/O goes through **LiteLLM** (`models.py`, `llm.py`, `sendchat.py`), giving ~any provider including local. Per-model `ModelSettings` dataclass encodes tuned defaults: `edit_format`, `weak_model_name`, `use_repo_map`, `reminder` (sys/user placement), `examples_as_sys_msg`, `extra_params`, `cache_control`, `use_temperature`, `streaming`, `editor_model_name`, `editor_edit_format`, `accepts_settings` (thinking_tokens / reasoning_effort). Aliases map short names to canonical IDs. `--model-settings-file` / `--model-metadata-file` let users override. OpenRouter has dedicated handling (`openrouter.py`).
- **Three model roles**: main model, **weak model** (commit messages, summarization), **editor model** (architect mode executor). Reasoning control: `--reasoning-effort`, `--thinking-tokens`, plus reasoning-tag stripping (`reasoning_tags.py`).

### Extensibility
- Modest: `.aider.conf.yml` config, `--config`, env files, `--load` command scripts, custom lint/test commands, model-settings files, custom `/`-commands aren't user-pluggable as such. **No MCP, no hooks, no plugin/subagent SDK.** The extension story is "configure flags + conventions files," not "add tools."

### UX surfaces
- **Terminal** (primary): prompt_toolkit input, Rich/markdown streaming (`mdstream.py`), spinners, autocomplete, vim keybindings (`--vim`), themes, multiline mode.
- **Scripting / API**: `--message`/`--message-file` for one-shot non-interactive runs; importable Python API (`main(..., return_coder=True)`, then `coder.run(with_message=...)`).
- **Web GUI**: a Streamlit app (`gui.py`, `--gui`/`--browser`).
- **Voice**: `/voice` speech-to-text (`voice.py`).
- **Images & URLs**: add screenshots/images and scrape web pages (`scrape.py`, Playwright optional) into context.
- **IDE / "watch" mode** (`watch.py`, `--watch-files`): watches files for **`AI!` / `AI?` / `AI` comments**; when you write `# do X AI!` in your editor and save, Aider picks it up, runs the request (`AI!` = make changes, `AI?` = answer), and strips the comments. This is the "use from any IDE" mechanism without an extension.

### Other / unique
- Built-in **benchmark harness** (`benchmark/`) and edit-format leaderboard methodology baked into the project's development.
- **Analytics** (`analytics.py`) via PostHog, opt-in, with rich `event(...)` instrumentation across the loop (message_send, exceptions, etc.).
- `--copy-paste` / `--apply-clipboard-edits`: a workflow for using a web chat UI as the model and pasting edits back.

## Standout / Unique
- **Repo map via tree-sitter + PageRank**, query-personalized and token-budgeted (`repomap.py`). This is a genuinely differentiated context-construction technique: structural, ranked, and cost-bounded rather than "read whole files" or naive embedding RAG.
- **Edit format as a first-class, measured design axis** with many implementations and tolerant parsers + automatic reconciliation/reflection on failure. Aider treats "how the model expresses an edit" as the central reliability/cost lever and tunes it per model empirically.
- **Architect/editor two-model split** — pay for strong reasoning, then delegate mechanical edit-syntax compliance to a cheaper/faster model in a possibly simpler format.
- **Git-as-runtime**: auto-commit per edit with detailed AI attribution, `/undo`, dirty-state isolation — the change log *is* the trace and the safety net.
- **`AI!`/`AI?` comment-driven editing** — a near-zero-integration way to drive the agent from any editor.
- **Bounded reflection loop** unifying malformed-edit, missing-file, lint, and test recovery into one mechanism with a hard cap.

## Pros
- Excellent cost-aware context: structural repo map beats blind file reads; deterministic prompt ordering maximizes prompt-cache hits.
- Reliable editing on weaker/cheaper models thanks to forgiving parsers + reflection; per-model tuning encoded in code.
- Strong artifact discipline: every change is a reviewable git commit; trivial rollback.
- Model-agnostic via LiteLLM; works with local models; three model roles let you put expensive tokens only where they matter.
- Empirical culture: benchmarks and leaderboards drive defaults — design-by-measurement.
- Cheap deterministic feedback (tree-sitter lint, py_compile, flake8) before spending model turns.

## Cons / Limitations
- **Not autonomous / not a server**: developer-in-the-loop, single process, no sandbox, no capability/permission policy, no isolation. Trust boundary is "the local user."
- **No tool-calling agent surface and no MCP/hooks/plugins** — extensibility is config, not code. Can't easily add new model-callable actions.
- Edit-format-in-free-text is brittle in the tail: relies on fuzzy matching and a 3-reflection budget; complex multi-file refactors can thrash.
- Repo map depends on tree-sitter grammar coverage and can hit `RecursionError` / be disabled on very large repos.
- Context is user-curated ("add files to the chat"); weaker at fully autonomous exploration than tool-loop agents.
- State is local files + git; no first-class multi-session/parallel/run-orchestration story.
- No formal completion/`submit` signal — completion is "stopped reflecting."

## Lessons for Glave (econo-frontier, feature-parity lens)
- **Copy:**
  - The **repo map** concept: tree-sitter symbol extraction + reference-graph ranking + per-query personalization + hard token budget. This is direct cost-per-correct-task leverage — it cuts blind reads and repeated exploration. Reimplement in Rust (tree-sitter has Rust bindings; use a Rust PageRank).
  - **Deterministic prompt-chunk ordering for prompt-cache maximization** (stable prefix → volatile suffix) and explicit cache-control placement. Cheap, large $ win.
  - **Bounded reflection loop** as the autonomy primitive: parse-fail / lint-fail / test-fail all feed back as the next turn under a hard cap. Clean, debuggable, evaluable.
  - **Git-as-artifact-boundary**: per-change commits with structured attribution as the trace + rollback handle.
  - **Cheap deterministic verifiers first** (syntax/compile/lint) before burning model turns.
- **Adapt:**
  - **Edit format** should become a *typed action* at the harness boundary (a validated `apply_patch` action with structured args), not free-text parsed from prose — but keep Aider's empirical "measure pass-rate per format per model" discipline. Aider proves the format/model pairing is a measurable cost lever.
  - **Per-model settings table** (`ModelSettings`) → make it a typed config + router in the Rust kernel (edit/action format, reasoning budget, reminder placement, temperature) selected by measured evals.
  - **Architect/editor split** → in Glave terms, a planner node + a cheaper executor child node with explicit budgets; matches the "child task nodes with budgets" doctrine and is a real $/correct optimization.
  - **History summarization** → first-class, budgeted context manager (Glave already wants this), not a best-effort background thread.
  - **`AI!` comment trigger** → a nice low-friction *client actuator* idea; the server still owns the run.
- **Skip:**
  - Terminal/Streamlit/voice UX, clipboard-paste workflow — not Glave's moat.
  - The trust-the-local-user, no-sandbox safety posture — directly conflicts with Glave's server-owned policy + isolation constraints. Replace with deterministic server policy.
  - Free-text edit parsing with fuzzy reconciliation as the *primary* mechanism — keep the tolerance ideas as a fallback, but lead with typed/validated edits.
  - Mixing config-as-extensibility for everything; Glave needs typed run specs, not flag soup.
- **Cost angle:** Aider is the strongest reference in the set for *cost per correct task* fundamentals. Its three biggest levers are all cost levers: (1) the repo map shrinks input tokens while raising relevance, (2) chunk ordering + cache-control maximizes prompt-cache reuse, (3) the weak/editor-model split routes expensive tokens only to reasoning. The empirical edit-format leaderboard is essentially a $/correct optimization over output grammar. For Glave: a measured repo-map, aggressive cache-aware prompt assembly, and model-role routing should be early wins, validated against Glave's own traces rather than copied blind.

## Source
- Control loop / turn flow: `refs/harnesses/aider/aider/coders/base_coder.py` (`run_one` ~924, `send_message` ~1419, `apply_updates` ~2296, `auto_commit` ~2375, `lint_edited` ~1681, `format_chat_chunks` ~1226, `Coder.create` ~125).
- Edit formats: `aider/coders/editblock_coder.py` (+ `editblock_prompts.py`), `udiff_coder.py`, `patch_coder.py`, `wholefile_coder.py`, `search_replace.py`, `aider/coders/__init__.py` (coder registry).
- Repo map: `aider/repomap.py`; tree-sitter grammars `aider/queries/tree-sitter-language-pack/*.scm`.
- Git / attribution: `aider/repo.py`. Linter: `aider/linter.py`.
- Modes: `architect_coder.py`, `ask_coder.py`, `context_coder.py`, `help_coder.py`.
- Commands: `aider/commands.py`. Config surface: `aider/args.py`. Models/routing: `aider/models.py`, `aider/llm.py`, `aider/openrouter.py`.
- Context/cache: `aider/coders/chat_chunks.py`, history summarization in `base_coder.py`. Watch/IDE: `aider/watch.py` (+ `watch_prompts.py`). GUI: `aider/gui.py`. Voice: `aider/voice.py`. Scrape: `aider/scrape.py`. Analytics: `aider/analytics.py`. Entry: `aider/main.py`.
- Upstream: https://github.com/Aider-AI/aider · docs https://aider.chat (repomap: /docs/repomap.html, edit formats/leaderboard: /docs/leaderboards).
