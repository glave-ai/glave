# Meta-Harness Notes

## Definition

A normal evaluation harness runs benchmarks. A meta-harness manages the whole improvement loop:

- public benchmark calibration
- private release gates
- model-as-judge scoring
- judge audits
- contamination checks
- failure mining
- data generation
- training candidate comparison
- regression reports
- release decisions

For Glave, the meta-harness is the behavioral operating system. It should answer not only "which model scored higher?" but "what changed, why did it change, what did it break, and should we keep it?"

## Source Notes

- [EleutherAI lm-evaluation-harness](https://github.com/EleutherAI/lm-evaluation-harness): strong static benchmark substrate with many tasks, custom task configs, public prompts, local models, vLLM, SGLang, and API backends.
- [OpenAI Evals](https://github.com/openai/evals): a framework and registry for evaluating LLMs and systems, including private workflow evals and model-graded eval templates.
- [Inspect AI](https://inspect.aisi.org.uk/): an evaluation framework organized around datasets, solvers, scorers, tools, agents, logs, and sandboxing.
- [Hugging Face LightEval](https://huggingface.co/docs/lighteval/index): multi-backend evaluation with custom tasks, metrics, storage, and sample-level results.
- [OpenCompass](https://github.com/open-compass/opencompass): broad model and dataset support, including LLM-as-judge and specialized evaluators.
- [HELM](https://crfm.stanford.edu/2022/11/17/helm.html): argues for broad scenario coverage, multi-metric measurement, standardization, and explicit recognition of what the benchmark misses.
- [MT-Bench and Chatbot Arena](https://arxiv.org/abs/2306.05685): validates strong LLM judges as scalable preference approximators while documenting position, verbosity, self-enhancement, and reasoning biases.
- [G-Eval](https://arxiv.org/abs/2303.16634): uses rubrics, chain-of-thought style evaluation, and form-filling to improve alignment with human judgments, while noting evaluator bias toward LLM-generated text.
- [RewardBench](https://github.com/allenai/reward-bench): evaluates reward models and LLM judges before they are trusted as training signal.
- [Data Contamination Quiz](https://arxiv.org/abs/2311.06233): probes whether benchmark gains may come from contamination rather than capability.
- [AILuminate](https://mlcommons.org/ailuminate/): separates public practice prompts, private official prompts, taxonomy, evaluator ensemble, and per-category reporting for safety evaluations.

## Harness Layers

### 1. Runners

Run public benchmark suites for comparability:

- `lm-eval` for standard language tasks
- LightEval for multi-backend development
- Inspect-style custom tasks for agents, tool use, multi-turn dialogue, and behavior
- direct scripts for domain-specific tasks where official scoring matters

Public scores calibrate the model. They do not define the model.

### 2. Scorers

Use the strictest scorer available:

- exact-match or programmatic oracle for math, code, extraction, and closed-form tasks
- execution-based scoring for code and tool workflows
- retrieval-grounded scoring for sourced answers
- LLM judges for subjective qualities such as warmth, humor, taste, clarity, and conversational fit
- human review for launch gates and judge calibration

Subjective judges should use pairwise comparisons, randomized answer order, length controls, and multiple judges.

### 3. Judge Audits

Before a judge can shape training data, it must pass its own evals:

- position-bias tests
- verbosity-bias tests
- self-preference tests
- factual correctness traps
- refusal-quality probes
- humor and warmth calibration against human labels
- adversarial examples where the polished answer is wrong

The harness should track judge drift just like model drift.

### 4. Failure Mining

Every failed sample should become structured data:

- prompt
- model output
- expected behavior
- failure category
- judge rationale
- corrected answer
- data-transform recipe
- whether it enters training, eval, or holdout

The useful artifact is not the red number on a dashboard. It is the failure cluster that becomes the next dataset patch.

### 5. Gates

Use three eval modes:

- Calibration: public benchmarks used to compare with the field.
- Development: visible internal evals used for daily iteration.
- Release Gate: private holdouts, rotated tasks, judge audits, human spot checks, and regression thresholds.

The model can improve public scores and still fail release if it becomes colder, more sycophantic, more verbose, less honest, or less funny.

## Glave Scorecard

Minimum top-level dimensions:

- capability
- factuality
- robustness
- calibration and uncertainty
- warmth
- emotional precision
- humor
- taste
- non-sycophancy
- refusal quality
- conversational continuity
- latency
- cost

Each dimension needs sample-level traces, not just aggregate scores.
