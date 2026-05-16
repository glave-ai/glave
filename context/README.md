# Context

This folder is the working research wiki for Glave.

The `sacred/` folder holds the canonical principles. This folder translates those principles into model behavior, data construction, evaluation design, and iteration loops.

## Map

- [AutoResearch Notes](./research/autoresearch.md): what Karpathy's AutoResearch pattern implies for an autonomous model-improvement loop.
- [Meta-Harness Notes](./research/meta-harnesses.md): how to build an evaluation system that grades models, judges, data transforms, and regressions.
- [Data Transform Loop](./research/data-transform-loop.md): how public data becomes curated training signal through stronger teachers, audits, and self-iteration.
- [Flywheel Start](./flywheel-start.md): the first practical lap for harness, adapters, evals, and local data.
- [Hope Seed Spec](./hope-seed-spec.md): the first behavioral specification for the model line.

## Working Thesis

Glave should not train toward a generic chatbot personality. The target is humane, useful intelligence with taste: warm, emotionally precise, funny when the moment permits, honest under uncertainty, capable under pressure, and respectful of the user's agency.

The first serious model line can use an open base such as [Qwen/Qwen3.6-27B](https://huggingface.co/Qwen/Qwen3.6-27B), then improve through a loop:

1. Collect public tasks, public conversations, public evals, and original prompts.
2. Transform them with stronger teacher models into cleaner demonstrations, critiques, comparisons, and rubrics.
3. Train candidate models.
4. Run public benchmark calibration, private behavioral gates, judge audits, and contamination checks.
5. Mine failures into the next data batch.
6. Keep only changes that improve the model without degrading its character.

The harness is part of the model. The constitution, rubrics, datasets, evals, and failure ledger are the source tree for behavior.
