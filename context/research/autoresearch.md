# Andrej Karpathy AutoResearch

## Summary

AutoResearch is a minimal autonomous experimentation loop for machine-learning research. A coding agent edits one training file, runs a fixed-budget experiment, reads a validation metric, keeps the change if it improves, reverts it if it does not, and repeats.

The repository is deliberately narrow:

- `prepare.py` owns fixed constants, data preparation, tokenization, data loading, and evaluation. It is not edited by the agent.
- `train.py` contains the GPT model, optimizer, hyperparameters, and training loop. It is the only file the agent edits.
- `program.md` is the instruction layer that tells the agent how to set up a branch, run experiments, log results, and decide whether to keep or discard changes.

The important conceptual shift is that the human no longer hand-edits the training code during the loop. The human designs the research organization through instructions, constraints, and evaluation.

## Core Loop

The loop in `program.md` is:

1. Start a fresh `autoresearch/<tag>` branch.
2. Read the repository context: `README.md`, `prepare.py`, and `train.py`.
3. Establish a baseline run.
4. Modify only `train.py`.
5. Commit the candidate change.
6. Run `uv run train.py > run.log 2>&1`.
7. Extract `val_bpb` and memory metrics from the log.
8. If `val_bpb` improves, keep the commit and advance the branch.
9. If `val_bpb` is equal or worse, reset to the prior commit.
10. Continue indefinitely until interrupted.

The target metric is validation bits-per-byte (`val_bpb`), where lower is better. The training run is fixed at five minutes of wall-clock training time, excluding startup and compilation. Karpathy notes this should yield roughly 12 experiments per hour or about 100 overnight on the reference setup.

## Design Choices

AutoResearch works because it is tightly constrained:

- Single editable file: keeps diffs reviewable and limits evaluator tampering.
- Fixed time budget: makes experiments comparable on the same hardware, even when the agent changes model size, batch size, architecture, or optimizer behavior.
- Single scalar metric: enables automatic accept/reject decisions.
- Git-based ratchet: good changes move the branch forward; bad changes are discarded.
- Local logs: every trial leaves evidence about the idea, outcome, crash mode, and resource use.

The default repo targets a single NVIDIA GPU and was tested on H100. Karpathy explicitly notes that fixed-budget results are not directly comparable across different compute platforms.

## Reported Results

The repository README describes the system as a small but real LLM training setup that an agent can modify overnight. Karpathy's `program.md` makes the first run a baseline and then instructs the agent to continue without asking the human whether to proceed.

One public session report in the repository discussions shows a run improving best `val_bpb` from `0.997900` to `0.969686` across 126 experiments in about 10.5 hours on an H100. The run kept 23 experiments, discarded 102, and had one crash. Reported winning changes included batch-size reduction, depth changes, RoPE base frequency changes, embedding learning-rate changes, weight decay on embeddings/value embeddings, initialization scaling, and warmdown tuning.

The `nanochat` leaderboard records AutoResearch-linked improvements to "time to GPT-2": a March 9, 2026 entry at 1.80 hours and a March 14, 2026 round-2 entry at 1.65 hours, compared with a March 4 baseline of 2.02 hours.

## Relation To Self-Improvement

AutoResearch is not unconstrained recursive self-modification. The base agent is not rewriting its own weights, redefining its own objective, or expanding its own authority. The system-level loop improves a bounded artifact: training code under a fixed evaluation harness.

The flywheel is:

`idea -> code edit -> experiment -> metric -> keep/revert -> trace -> next idea`

This matters because it makes self-improvement concrete and inspectable. The system improves only when an empirical gate says the candidate is better. Failed ideas are not wasted; they become a search trace that can guide future proposals.

## Limits And Caveats

- Eval overfitting: repeated optimization against one validation metric can Goodhart the metric or spoil the validation set.
- Noise floor: very small `val_bpb` gains may be run variance unless retested.
- Hardware specificity: the five-minute budget normalizes within a machine, not across machines.
- Narrow scope: the setup works because the task is small, measurable, and cheap enough to run many times.
- Agent search quality: follow-up work on AutoResearch as an HPO testbed found classical HPO methods such as CMA-ES and TPE outperform pure LLM-based agents in fixed search spaces; a hybrid method performed best.
- Evaluator control: the loop is only trustworthy if the agent cannot alter the evaluator, data loader, hidden answers, or scoring logic.

## Takeaways

AutoResearch is best understood as the smallest viable self-improvement ratchet:

- Bound autonomy to a narrow edit surface.
- Keep the evaluator fixed and outside the agent's edit scope.
- Make experiments cheap enough to run repeatedly.
- Log every trial.
- Promote only empirically validated changes.
- Treat instructions, evals, and experiment policy as the next surfaces for improvement.

## Sources

- Karpathy AutoResearch repository: https://github.com/karpathy/autoresearch
- AutoResearch README: https://raw.githubusercontent.com/karpathy/autoresearch/master/README.md
- AutoResearch `program.md`: https://raw.githubusercontent.com/karpathy/autoresearch/master/program.md
- AutoResearch discussion #43 session report: https://github.com/karpathy/autoresearch/discussions/43
- nanochat repository and leaderboard: https://github.com/karpathy/nanochat
- HPO comparison paper: https://arxiv.org/abs/2603.24647
- VentureBeat coverage: https://venturebeat.com/technology/andrej-karpathys-new-open-source-autoresearch-lets-you-run-hundreds-of-ai
