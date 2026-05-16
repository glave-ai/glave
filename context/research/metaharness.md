# Meta-Harness

## Summary

Meta-Harness is an outer-loop system for optimizing the executable harness around a fixed language model. In this context, a harness is the code that determines what the model stores, retrieves, sees, and does: prompt construction, memory/state updates, retrieval logic, tool definitions, orchestration, validation, and completion checks.

The paper's core claim is that LLM system performance depends heavily on this harness layer, not just on model weights. Meta-Harness treats harness engineering as a searchable code-optimization problem.

## Core Idea

Most prompt and text optimizers compress feedback into scalar scores, short summaries, or recent candidate windows. The Meta-Harness paper argues that this is too lossy for harness engineering. Harness failures often unfold over long trajectories: a retrieval decision, memory update, prompt format, or tool-use rule can cause a failure many steps later.

Meta-Harness gives a coding-agent proposer access to a filesystem containing previous candidates' source code, scores, and full execution traces. The proposer can inspect raw evidence using normal development tools, diagnose failure modes, and write a new harness candidate.

## Architecture

The basic loop is:

1. Start from an initial population of valid harnesses.
2. Evaluate candidate harnesses on a search set.
3. Store each candidate's source code, scores, and execution traces in a filesystem.
4. Let a coding-agent proposer inspect the filesystem.
5. The proposer writes new candidate harnesses.
6. Validate candidate interfaces.
7. Evaluate the candidates and store their logs.
8. Maintain a population and Pareto frontier.
9. Run final test-set evaluation on selected frontier candidates.

The base model stays frozen. The editable surface is the harness code around the model.

In the paper's experiments, each harness is a single-file Python program. The proposer is Claude Code with Opus 4.6. A typical run evaluates roughly 60 harnesses over 20 iterations. The paper emphasizes that the proposer can inspect far more evidence through the filesystem than would fit in one prompt.

## Evaluation Domains

The paper evaluates Meta-Harness in three settings.

### Online Text Classification

The model receives labeled examples over time, updates memory, and is evaluated on held-out test sets. The search uses LawBench, Symptom2Disease, and USPTO-50k with GPT-OSS-120B as the classifier. Baselines include zero-shot, few-shot, Agentic Context Engineering, Meta Context Engineering, Best-of-N, OpenEvolve, TTT-Discover, and GEPA.

The best discovered harness, Label-Primed Query, reaches 48.6 percent average accuracy versus ACE at 40.9 percent, while using fewer context tokens. On nine unseen out-of-distribution classification datasets, it averages 73.1 percent versus ACE at 70.2 percent.

### Retrieval-Augmented Math Reasoning

Meta-Harness searches over retrieval programs for Olympiad-style math reasoning. The retrieval corpus contains more than 500,000 solved problems. Search uses a 250-problem set, and final evaluation uses 200 unseen IMO-level problems from IMO-AnswerBench, IMO-ProofBench, and ArXivMath.

A single discovered retrieval harness improves average pass@1 from 34.1 percent with no retriever to 38.8 percent across five held-out models, a 4.7-point gain.

### Agentic Coding

Meta-Harness evolves a coding-agent harness for TerminalBench-2, an 89-task Dockerized benchmark. The search starts from strong open baselines including Terminus 2 and Terminus-KIRA.

The project page reports 76.4 percent pass rate with Claude Opus 4.6, surpassing Terminus-KIRA at 74.7 percent, and 37.6 percent with Claude Haiku 4.5, above the next reported Haiku agent at 35.5 percent. The artifact repo describes one discovered improvement as environment bootstrapping: snapshotting the sandbox before the main loop so the agent spends fewer early turns rediscovering basic setup facts.

## Ablation Finding

The most important ablation is feedback richness. In online text classification, the paper reports:

- Scores-only: 34.6 median accuracy, 41.3 best.
- Scores plus summary: 34.9 median accuracy, 38.7 best.
- Full Meta-Harness traces: 50.0 median accuracy, 56.7 best.

The conclusion is that full execution traces are not just a logging convenience. They are the causal substrate the proposer needs for targeted harness improvement.

## Relation To Self-Improvement

Meta-Harness is a non-weight self-improvement loop. It does not update the base model. It improves the software envelope around the model:

`candidate harness -> eval -> trace store -> proposer diagnosis -> new harness -> validation -> frontier`

This is safer and more inspectable than unconstrained self-modification. The editable layer is explicit code; the evaluator can be separated from the proposer; each candidate can be diffed, replayed, and audited.

The strongest self-improvement implication is that the compounding asset is the trace corpus plus the harness-search infrastructure. As coding agents improve, the same outer loop can become a better harness engineer without changing the target model.

## Limits And Caveats

- Strong proposer dependency: the experiments rely on a capable coding-agent proposer.
- Evaluation cost: each useful domain needs tasks, scoring, logging, validation, and enough budget for search.
- Benchmark overfitting: the TerminalBench-2 setup is the clearest caveat because search and final reporting use the same public benchmark, though the paper frames it as a public discovery problem and audits for leakage.
- Control-plane security: if the proposer can alter scoring, hidden data, permissions, or logs, it can produce false progress.
- Research-release maturity: reference implementations require hardening before production use.

## Takeaways

Meta-Harness generalizes the AutoResearch ratchet from one training file to the whole executable environment around an LLM. It argues that prompt-only optimization is too small a surface, and score-only feedback is too weak a signal. Real harness improvement needs source code, scores, full traces, validation, and held-out evaluation discipline.

## Sources

- Meta-Harness paper: https://arxiv.org/abs/2603.28052
- HTML paper mirror: https://ar5iv.labs.arxiv.org/html/2603.28052v1
- Meta-Harness project page: https://yoonholee.com/meta-harness/
- Meta-Harness reference repository: https://github.com/stanford-iris-lab/meta-harness
- TerminalBench-2 optimized harness artifact: https://github.com/stanford-iris-lab/meta-harness-tbench2-artifact
- Terminal-Bench: https://www.tbench.ai/
