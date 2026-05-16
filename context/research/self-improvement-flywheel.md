# Self-Improvement Flywheel

## Core Thesis

AutoResearch and Meta-Harness point to the same practical self-improvement pattern: do not let a model freely rewrite itself. Instead, let a bounded agent propose changes to an editable system surface, evaluate those changes under a trusted harness, keep only validated improvements, and preserve traces that make the next round smarter.

The flywheel is:

`candidate change -> evaluation -> trace corpus -> diagnosis -> new candidate -> accept/reject -> stronger baseline`

AutoResearch applies this to ML training code. Meta-Harness applies it to the executable harness around a frozen model.

## Shared Pattern

Both systems convert progress into an empirical ratchet:

- Bounded edit surface: AutoResearch restricts edits to `train.py`; Meta-Harness searches over harness code while keeping the base model fixed.
- Trusted evaluator: AutoResearch gates changes on `val_bpb`; Meta-Harness gates candidates on benchmark scores, held-out tests, and frontier selection.
- Versioned experiments: winning candidates become the new baseline; losing candidates are reverted or retained only as diagnostic evidence.
- Trace accumulation: runs produce logs, scores, failures, diffs, prompts, outputs, tool calls, and execution traces.
- Higher human leverage: humans design the loop, objective, constraints, and instruction layer instead of hand-running every experiment.
- System-level improvement: the compounding asset is the harness, eval suite, trace corpus, and selection policy, not only the base model.

## Key Difference

AutoResearch is the smallest viable flywheel. It has one editable file, one scalar metric, a fixed five-minute experiment budget, and git as the ratchet.

Meta-Harness is the richer version. It searches over retrieval, memory, prompting, orchestration, tool definitions, validation, and state updates. Its strongest contribution is the claim that full execution traces are necessary for long-horizon harness debugging. Scores identify better candidates; traces explain why candidates failed or succeeded.

## Prerequisites For Real Compounding

A self-improvement flywheel compounds only when the loop has real selection pressure and low leakage.

- Evaluator isolation: the proposer must not be able to edit the evaluator, see hidden answers, or infer answer-bearing hints from feedback.
- Cheap repeatable trials: the loop needs enough experiments for signal to emerge before costs dominate.
- Signal above noise: tiny metric gains need retesting, confidence checks, or promotion thresholds.
- Trace-rich logging: scalar scores identify winners; traces explain failures.
- Strict acceptance gates: promote only after validation, regression checks, and preferably held-out or transfer tests.
- Replayability: every accepted change should be tied to code, config, seed, environment, score, and logs.
- Permission boundaries: candidate code can improve orchestration, but it must not expand tool authority, mutate hidden state, or launder private eval feedback into memory.
- Human-owned objectives: the system can optimize aggressively, so the metric and guardrails must represent desired progress.

## Failure Modes

The main risk is false progress:

- Goodharting: the loop optimizes the metric while degrading real-world usefulness.
- Eval leakage: hidden answers or answer-shaped hints leak into prompts, memory, traces, or summaries.
- Validation overfitting: repeated search against one benchmark produces brittle gains that do not transfer.
- Noise chasing: small improvements are accepted even though they are run variance.
- Evaluator tampering: the agent improves scores by modifying scoring, data loading, logging, or task setup.
- Trace poisoning: bad or leaked traces become reusable lessons and contaminate future search.
- Over-broad autonomy: the edit surface grows faster than the control plane can audit it.
- Thin feedback: scores-only feedback makes the proposer guess; summaries can erase the causal details needed for real debugging.

## Practical Architecture

A robust implementation should separate the proposer, candidate code, evaluator, and promotion gate.

1. Control plane: owns tasks, permissions, hidden evals, scoring, trace storage, and promotion rules.
2. Candidate workspace: isolated branch or sandbox where the proposer can edit only approved files.
3. Validator: checks interface contracts, forbidden files, tool permissions, resource limits, and leakage-sensitive paths before execution.
4. Eval runner: executes candidates on public/search evals, regression tests, cost metrics, and reliability checks.
5. Trace store: append-only record of code, diffs, prompts, tool calls, outputs, errors, scores, resource use, and environment metadata.
6. Selector: compares against champion and frontier candidates; requires retests before promotion.
7. Hidden eval layer: used sparingly for promotion, returns only coarse aggregate signals, never answer-bearing diagnostics.
8. Curation layer: turns safe traces into reusable lessons, tests, or training data, with provenance tags and quarantine for sensitive material.

The operating rule: candidate patches may improve the harness, but only the control plane can judge, promote, expose memory, or touch hidden evaluation state.

## High-Confidence Findings

1. The strongest near-term self-improvement loop is harness improvement, not unconstrained model self-modification.

2. Evals are both judge and teacher, but only if feedback is high-fidelity and non-leaky.

3. AutoResearch proves the minimal ratchet: bounded edits, fixed metric, reversible experiments, accepted wins.

4. Meta-Harness extends the ratchet from prompts to executable harness code, where retrieval, memory, tools, and orchestration become optimizable surfaces.

5. Full traces are a compounding asset; scores alone are too thin for long-horizon debugging.

6. Real compounding requires hidden-eval separation, retesting, provenance, and strict control-plane ownership.

7. The flywheel gets stronger as proposer agents improve, because the same outer loop can search better without changing the target model.

8. The durable advantage is the system: evals, traces, harness code, acceptance policy, curated memory, and workflow integration.

## Sources

- AutoResearch repository: https://github.com/karpathy/autoresearch
- AutoResearch README: https://raw.githubusercontent.com/karpathy/autoresearch/master/README.md
- AutoResearch `program.md`: https://raw.githubusercontent.com/karpathy/autoresearch/master/program.md
- AutoResearch discussion #43: https://github.com/karpathy/autoresearch/discussions/43
- nanochat leaderboard: https://github.com/karpathy/nanochat
- HPO comparison paper: https://arxiv.org/abs/2603.24647
- Meta-Harness paper: https://arxiv.org/abs/2603.28052
- Meta-Harness project page: https://yoonholee.com/meta-harness/
- Meta-Harness artifact repository: https://github.com/stanford-iris-lab/meta-harness-tbench2-artifact
