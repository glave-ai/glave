# Glave

> _A glave is the instrument, not the wielder._

Glave is a project building humane, affordable intelligence — AI that
expands what people can do rather than replacing them. The principles
that govern this work are codified, not aspirational; they live in
[`sacred/`](./sacred) and bind everything built under this name.

- **[Manifesto](./sacred/MANIFESTO.md)** — what we believe and why we are
  building.
- **[Constitution](./sacred/CONSTITUTION.md)** — the binding articles
  that govern architectural, operational, and commercial decisions.

If you're trying to understand a design or product call, start there.
The code in this repo is the materialization of those documents, not a
substitute for them.

---

## Repo layout

```
.
└── sacred/             # Canonical documents (Manifesto, Constitution)
```

Today the repo holds the canonical text. Product surfaces will come later,
after the model, data, and evaluation flywheel have earned them.

---

## Conventions

- **Sacred is binding:** do not alter `sacred/` casually. Promote ideas
  there only after they have stabilized.
- **Plain text:** prefer Markdown for research, specs, rubrics, eval
  plans, and iteration logs so both people and agents can read them.
- **Local artifacts:** screenshots, scratch output, and other ephemera
  go in `tmp/` (gitignored). Never commit them at the repo root.

---

## Status

Early research reset. The current priority is testing the model
improvement flywheel: public data, teacher transforms, private evals,
judge audits, failure mining, and tightly logged self-iteration.
