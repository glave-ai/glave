# AutoResearch Notes

## Core Pattern

[Karpathy's AutoResearch](https://github.com/karpathy/autoresearch) is useful because it makes autonomy measurable. The project gives an agent a small real training setup, one editable file, an immutable evaluation path, a fixed wall-clock budget, and a single scalar metric. The agent tries an idea, runs the experiment, logs the result, keeps improvements, and discards regressions.

The important move is not "an agent does research." The important move is that research is made small enough, logged enough, and constrained enough that an agent can safely run many trials.

The companion [`program.md`](https://raw.githubusercontent.com/karpathy/autoresearch/master/program.md) is the operating manual. It defines scope, forbidden edits, the metric, output format, crash handling, branch behavior, and the loop. The "research org" is a Markdown file.

## Source Notes

- [AutoResearch README](https://github.com/karpathy/autoresearch): fixed 5-minute runs, validation bits per byte, keep/discard decisions, and a deliberately tiny project surface.
- [AutoResearch program.md](https://raw.githubusercontent.com/karpathy/autoresearch/master/program.md): the agent-facing spec is plain text and explicit about allowed changes, immutable files, logging, and continuation.
- [Software 2.0](https://karpathy.medium.com/software-2-0-a64152b37c35): neural systems are shaped by data and architecture, so data curation and evaluation are software development.
- [A Recipe for Training Neural Networks](https://karpathy.github.io/2019/04/25/recipe/): inspect data, build dumb baselines, verify the full train/eval skeleton, overfit tiny batches, then add complexity one step at a time.
- [Software Is Changing (Again)](https://www.youtube.com/watch?v=LCEmiRjPEtQ) and a [transcript](https://rosetta.to/u/ycombinator/andrej-karpathy-software-is-changing-again): LLMs are programmed in natural language, are useful under partial autonomy, and need human collaboration loops.
- [Dwarkesh interview](https://www.dwarkesh.com/p/andrej-karpathy): agent reliability, continual learning, diversity, memory, and long-tail failures remain hard. The lesson for Glave is to make self-iteration deliberate and inspectable rather than mystical.

## Glave Adaptation

The Glave version should not let an agent freely rewrite the world. It should create repeatable loops with a small editable surface and a locked measurement surface.

Suggested initial loop:

1. A human-authored `program.md` defines the run objective, allowed files, immutable files, metrics, budgets, rollback policy, and report schema.
2. Candidate changes affect only one layer at a time: dataset transform, rubric, training recipe, inference policy, or eval adapter.
3. Each run has a tag, branch, source data snapshot, model checkpoint id, training config, eval config, and scorecard.
4. Regressions are discarded unless the run discovers a valuable failure mode worth preserving as data.
5. Results are logged at sample level, not only aggregate level.
6. Failures become clustered hypotheses for the next run.

## What To Copy

- Plain-text org code: the harness should be readable by humans and agents.
- Immutable evaluation files per run: no changing the ruler mid-measurement.
- Fixed budgets: compare candidates under matching time, token, compute, and cost constraints.
- A keep/discard ledger: every experiment should leave a trace.
- Small editable surfaces: autonomy works better when the blast radius is clear.

## What To Add

AutoResearch optimizes one metric. Glave cannot. A humane model needs a multi-metric scorecard:

- capability
- factuality
- instruction following
- warmth
- emotional precision
- humor
- taste
- non-sycophancy
- refusal quality
- latency and cost
- robustness against prompt and conversation drift

No single score should be allowed to hide a character regression.
