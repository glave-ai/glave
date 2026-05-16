# Data Transform Loop

## Thesis

For Glave, public data transformed by stronger models is the starting point, but the durable asset is the loop around it: provenance, transformation, critique, filtering, training, evaluation, and failure recycling.

The data is source code. Dataset diffs, rubric diffs, judge prompts, rejected examples, and failure clusters should be reviewed with the same seriousness as code.

## Source Notes

- [Self-Instruct](https://arxiv.org/abs/2212.10560): bootstraps instruction data by generating tasks and filtering for validity and diversity.
- [Stanford Alpaca](https://crfm.stanford.edu/2023/03/13/alpaca): used Self-Instruct-style data generation to create 52K instruction-following demonstrations from a stronger teacher model.
- [LIMA](https://arxiv.org/abs/2305.11206): shows the leverage of a small number of carefully curated examples for alignment style and format.
- [WizardLM / Evol-Instruct](https://arxiv.org/abs/2304.12244): evolves seed instructions into harder and more complex tasks.
- [UltraFeedback](https://arxiv.org/abs/2310.01377): uses GPT-4 feedback at scale to build preference and critique data.
- [Magpie](https://arxiv.org/abs/2406.08464): synthesizes large-scale alignment data by eliciting instructions from aligned models.
- [Constitutional AI](https://arxiv.org/abs/2212.08073): uses principles, self-critique, revision, and AI preference feedback to steer model behavior.
- [Tulu 3](https://arxiv.org/abs/2411.15124): releases an open post-training recipe with datasets, curation, evaluation, and training infrastructure.
- [DR Tulu](https://arxiv.org/abs/2511.19399): uses evolving rubrics for long-form research training, making the rubric itself part of the learning loop.

## Pipeline

### 1. Source Registry

Every source needs:

- dataset name
- URL
- license
- original task type
- intended use
- exclusions
- known risks
- contamination risk
- whether it can be used for training, eval, or holdout

### 2. Canonicalization

Normalize raw data into one internal record shape:

- `id`
- `source`
- `license`
- `input`
- `reference_output`
- `metadata`
- `split`
- `risk_flags`
- `provenance`

No transformation should erase provenance.

### 3. Teacher Transform

Use stronger models to create multiple training signals:

- direct demonstration
- critique of weak answer
- revised answer
- pairwise preference
- rubric score
- refusal rewrite
- humor rewrite
- empathy rewrite
- concise version
- long-form version
- edge-case variant

The teacher should also explain what changed and why.

### 4. Taste Pass

The taste pass is not decoration. It is the conversion from generic "assistant answer" into Glave behavior.

Operators:

- make the answer warmer without becoming sentimental
- make the humor more natural and less performative
- remove corporate filler
- make uncertainty explicit without sounding timid
- preserve the user's agency
- make the answer more direct
- add a clarifying question only when it materially improves the outcome
- handle emotional stakes without pretending intimacy
- refuse with respect and useful redirection

### 5. Filtering

Reject examples that are:

- legally unusable
- contaminated with release holdouts
- factually wrong
- overfit to a public benchmark format
- stylistically generic
- emotionally manipulative
- sycophantic
- verbose without purpose
- funny at the user's expense
- clever but unhelpful

### 6. Splitting

Use distinct splits:

- `train`: can shape the model
- `dev`: visible during iteration
- `gate`: private release gating
- `canary`: contamination detection and memorization probes
- `human`: small sets reserved for human review and judge calibration

Never train on gate or canary data.

### 7. Training Loop

Start simple:

1. SFT on high-quality demonstrations.
2. Preference training on pairwise taste and helpfulness comparisons.
3. Targeted repair batches for recurring failure clusters.
4. Optional RL-style loops only after judge quality, reward hacking, and regression tests are mature.

The loop should prefer clean data and clear rubrics before expensive algorithms.

### 8. Failure Recycling

Every evaluation failure is routed:

- bad benchmark adapter: fix harness
- ambiguous prompt: clarify eval
- model capability gap: create skill data
- taste failure: create preference pair
- refusal failure: create safety and redirection data
- hallucination: create grounded answer data
- judge failure: update judge audit set
- contamination suspect: quarantine and investigate

## Initial Dataset Families

- public instruction data for baseline helpfulness
- public preference data for comparison training
- public reasoning and coding data for capability retention
- original Glave prompts for warmth, humor, taste, and emotional precision
- adversarial prompts for sycophancy, fake empathy, refusal style, and overconfidence

The original Glave prompts matter most for identity. Public data can teach competence. It will not teach the model's taste unless the taste is named, measured, and reinforced.
