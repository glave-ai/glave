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
├── sacred/             # Canonical documents (Manifesto, Constitution)
└── apps/
    └── frontend/       # Angular 21 web app — design system + reference UI
```

Today the repo holds two things: the canonical text, and a frontend that
will eventually surface and operate on it. More will follow as the work
expands.

---

## Frontend

An Angular 21 app using Angular Material with a custom design system
(graphite neutrals, iron-blue primary, oxidized-copper accent). All
styling flows through Material's `mat.theme()` and a small set of
`--glave-*` tokens defined in `src/styles.scss`.

### Quick start

```sh
cd apps/frontend
npm install
npm start          # dev server on http://localhost:4200
```

### Useful scripts

| Script                 | What it does                                  |
| ---------------------- | --------------------------------------------- |
| `npm start`            | Dev server with HMR                           |
| `npm run build`        | Production build                              |
| `npm test`             | Run the test suite (vitest)                   |
| `npm run lint`         | ESLint (angular-eslint + typescript-eslint)   |
| `npm run format`       | Prettier write — ts/js/html/scss/json/md/yaml |
| `npm run format:check` | Prettier check (fails on unformatted files)   |
| `npm run check`        | `format:check` + `lint` — what CI should run  |

### Routes

| Path           | Description                                                          |
| -------------- | -------------------------------------------------------------------- |
| `/`            | Homepage — hero, product tiles (API/Chat/Docs/Blog), Manifesto       |
| `/api`         | API surface — placeholder until the reference lands                  |
| `/docs`        | Documentation — placeholder                                          |
| `/chat`        | Chat surface — placeholder                                           |
| `/blog`        | Blog — placeholder                                                   |
| `/legal`       | Legal documents (Manifesto, Constitution, audit) — placeholder       |
| `/foundations` | Internal design-system reference: tokens, typography, components     |

---

## Conventions

- **Selector prefix:** `glave-` for new components (`app-` is still
  accepted for legacy scaffolding).
- **Component style:** standalone, signals, `OnPush`, lazy-loaded
  feature routes. See `apps/frontend/.claude/CLAUDE.md` for the full
  Angular guidance.
- **Indentation:** tabs at width 4 for code; 2 spaces for Markdown and
  YAML (enforced by Prettier).
- **Design tokens:** prefer `--mat-sys-*` (Material system tokens) for
  surfaces and roles; reach for `--glave-*` only when you specifically
  need brand neutrals or accents.
- **Local artifacts:** screenshots, scratch output, and other ephemera
  go in `tmp/` (gitignored). Never commit them at the repo root.

---

## Status

Early. The frontend ships a homepage, lazy-loaded placeholder pages for
each product surface (API, Docs, Chat, Blog, Legal), and the
Foundations design-system reference. Expect rapid change.
