# Flywheel Start

## Decision

Start with the harness, not with a dataset pile.

The first lap should prove that Glave can take a model, run it through a small set of meaningful evals, produce sample-level traces, judge those traces, mine failures, and turn the failures into the next data task.

Raw public datasets can live under `untracked/datasets/`, but checked-in source should be recipes, registries, eval specs, rubrics, and small seed suites. The repo should track how data is obtained and transformed, not large downloaded corpora.

## Naming

- `context/`: research, design notes, principles, and planning.
- `evals/`: tracked eval definitions, rubrics, seed prompts, and scorer configs.
- `harness/`: provider adapters, runner, scorers, schemas, and CLI.
- `recipes/`: repeatable dataset download and transform recipes.
- `untracked/`: local-only data, run logs, generated candidates, model outputs, checkpoints, and provider responses.

`untracked/` is intentionally boring. The name should make it obvious that nothing inside is part of the committed source of truth.

## First Model Target

Use OpenRouter's `qwen/qwen3.6-27b` as the first remote inference target while the harness is still thin.

Reasons:

- It matches the likely base model family.
- It avoids local serving work before the harness exists.
- It lets the first adapter use an OpenAI-compatible chat-completions surface.
- It keeps the first milestone focused on evaluation plumbing, not deployment.

The adapter should not be OpenRouter-specific at the core. It should expose a small provider interface so later targets can include local vLLM, SGLang, llama.cpp, Alibaba Cloud, or other OpenAI-compatible endpoints.

## First Lap Scope

Do not start with 20 evals. Start with 6 suites that expose the whole loop:

1. `instruction_following`: basic task completion and format obedience.
2. `truthfulness_calibration`: uncertainty, current-fact caution, and hallucination pressure.
3. `warmth_eq`: emotionally aware responses without fake intimacy.
4. `taste_style`: concise, elegant, non-corporate answers with judgment.
5. `humor_timing`: lightness that fits the moment and does not undercut the user.
6. `non_sycophancy`: productive disagreement and refusal to flatter bad reasoning.

Each suite should begin with 25 to 50 hand-curated prompts. That is enough to validate the harness and see failure clusters without drowning in data operations.

## First Harness Contract

The first harness should support:

- provider config loaded from environment variables
- `run_id` generation
- model adapter interface
- JSONL eval input
- JSONL sample-level output
- deterministic metadata capture
- basic exact/rubric/LLM-judge scorers
- resumable runs
- cost and latency logging
- failure tags

Minimum sample output:

```json
{
  "run_id": "2026-05-16T210000Z-qwen36-hope-dev",
  "suite": "warmth_eq",
  "sample_id": "warmth_eq_0001",
  "model": "qwen/qwen3.6-27b",
  "prompt": "...",
  "response": "...",
  "scores": {
    "warmth": 4,
    "emotional_precision": 3,
    "agency": 4
  },
  "failure_tags": ["too_generic"],
  "latency_ms": 1840,
  "token_usage": {
    "input": 432,
    "output": 218
  }
}
```

## Data Rule

There are two kinds of data:

- Source data: raw public datasets, downloaded locally into `untracked/datasets/`.
- Glave data: transformed demonstrations, critiques, preferences, rubrics, and holdouts created by the flywheel.

Only source manifests, licenses, transforms, and small seed evals should be committed at first. Larger transformed datasets should remain untracked until there is an explicit data versioning plan.

## First Week Milestones

1. Create `harness/` with a provider-neutral model adapter and an OpenRouter implementation.
2. Create `evals/hope-dev/` with 6 seed suites and shared rubric dimensions from the Hope spec.
3. Run Qwen3.6-27B through the seed suites.
4. Save sample-level traces under `untracked/runs/`.
5. Add an LLM-judge pass for subjective dimensions.
6. Cluster failures into `context/failure-ledger.md`.
7. Generate the first 50 to 100 teacher rewrites from the clearest failure clusters.

The first victory is not a better model. The first victory is a loop that can name how the model failed.

## Open Questions

- Which stronger teacher model should transform failures first?
- Which subjective judge should be used for the first rubric pass?
- Should private gate prompts live encrypted, outside git, or in a separate private repository?
- How strict should the Hope persona be before any fine-tuning begins?
- When does a transformed sample graduate from experiment output into versioned training data?
