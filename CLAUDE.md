# Glave — engineering & design guide

Scope: the Rust backend lives in `crates/`; the web app (Angular) lives in `apps/web`.
The **Rust** section governs `crates/`; **TypeScript & Angular** and **Design** govern `apps/`.

## General engineering (all code)

- Types enforce invariants; make illegal states unrepresentable.
- Boring-correct over clever. Subtraction is the skill — delete before you add.
- Decide by measured evals, not vibes.
- Build the seams + the loop, then stop. Don't scope-creep around the architecture.
- Never mark work done without green: lint (`-D warnings`) + tests, then a checkpoint commit.
- Small, reviewed changes over big drops. Match surrounding code — its naming, idioms, comment density.

## Rust (`crates/`)

Mirrors the workspace `Cargo.toml` lint posture:

- `unsafe` is forbidden.
- Denied: `unwrap`, `expect`, `panic`, indexing/slicing, `unwrap_in_result`. Pedantic is on — treat warnings as errors.
- Error handling: `anyhow::Result` + `?`. Never `unwrap`/`expect` — use `try_from(...).unwrap_or(...)`, `.get(n)`, or `match`.
- Ports/traits seam the brain from adapters; the oracle stays sealed by type (the agent never receives a `Grader`).
- Offline tests (`InMemory`); live model runs are a separate, human-triggered step.
- The gate: `cargo clippy --all-targets --all-features -- -D warnings` && `cargo test`.

## TypeScript & Angular (`apps/`)

Write functional, maintainable, performant, accessible code following Angular and TypeScript best practices.

### TypeScript

- Strict type checking. Prefer inference when the type is obvious. Avoid `any`; use `unknown` when uncertain.
- `const` everywhere. `let` only for function-local mutable state that's built up and returned (or similar) — used sparingly. `var` is outlawed.
- Immutability everywhere. The hazard: a signal holding a *mutable* object that gets mutated in place → change detection misses it. Default to immutable: `ReadonlyArray<T>`, `ReadonlyMap`, `ReadonlySet`, `readonly` fields, `as const`. Never mutate in place — produce a new value.
- Every signal is a `readonly` field. Access modifier is `private`, `protected`, or omitted (public is the default — never write `public`). e.g. `private readonly count = signal(0);`, `protected readonly total = computed(() => …);`

### Angular

- Standalone components always. Do NOT set `standalone: true` (default in v20+). Do NOT set `changeDetection: OnPush` explicitly (default in v22+).
- Signals for state; `computed()` for derived; `input()`/`output()` functions, not decorators.
- Never `mutate` a signal — use `set`/`update` (and prefer immutable values, per above).
- Signal Forms (`@angular/forms/signals`, the `form()` API) for new forms (stable v22+). Otherwise Reactive forms — never template-driven.
- Native control flow (`@if`/`@for`/`@switch`), not `*ngIf`/`*ngFor`/`*ngSwitch`. Async pipe for observables.
- `class`/`style` bindings, not `ngClass`/`ngStyle`. Host bindings in the `host` object, not `@HostBinding`/`@HostListener`.
- `inject()`, not constructor injection. `providedIn: 'root'` for singletons (`@Service` in v22+). Lazy-load feature routes.
- `NgOptimizedImage` for static images. Don't assume globals (e.g. `new Date()`).
- Keep components small, single-responsibility; prefer inline templates for small ones.

### Angular Material (UI)

- Angular Material is the component library. Use Material components wherever it makes sense — don't hand-roll what Material provides.
- Adhere to the custom theme: style via Material system tokens (`--mat-sys-*`) and our `--hope-*` brand tokens. Never hardcode colors/fonts/spacing — so light/dark and theme tweaks flow automatically.
- Form fields: `appearance="outline"` on every `mat-form-field` — never the default `fill` (the blocky one).

### ESLint

- ESLint ships with Angular 20+ (`eslint.config.js`). Run `npm run lint` / `npm run lint:fix`.
- Enforce: `prefer-const` (error), `no-var` (error), no `any`, `arrow-body-style: as-needed`, `no-console` (warn), explicit return types where useful, selector prefixes.

### Accessibility

- Must pass AXE; meet WCAG AA (focus management, contrast, ARIA). Access for everyone is the mission, not a checkbox.

## Design — Glave & Hope

The brand is for **everyone**, not developers. Glave is "what a lab looks like when it trusts people" — democratization, warmth, anti-paternalism. Hope is the human spirit made warm: a Samantha-like companion **without** the attachment-trap — warm, witty, intelligent; she empowers and gets out of the way.

Glave and Hope share **one** identity (a branded house). Hope is the living, luminous expression; Glave the more restrained corporate face. Same palette.

### Principles

- For everyone → radically intuitive; conversation-first; no jargon; real accessibility.
- Warm & human → warm light, soft rounded forms, generous calm space, organic motion; copy carries the warmth.
- Trust, not capture → no dark patterns, no engagement bait; transparent (show cost); respects attention; glad when you leave.
- Trends toward the light → reject the cold sci-fi AI cliché (black / neon / matrix). Hope looks like **sunrise, not the machine.** Warm *light*, not warm *paper* (that distinction is what keeps us off Anthropic's lane).
- Breathtaking through restraint → craft + light + space do the work; one or two moments of presence, not constant animation.
- Hope's presence is **ambient** (light / glow), never a literal face or avatar.
- Light-first; dark mode is **warm dusk**, never cold black.

### Palette — dawn gold (locked)

Chosen from Hope (the color of light / dawn), not against anyone. Primary is the jewel — used **sparingly**; the warmth lives in the **neutrals**.

- Primary (dawn gold): tint `#FCEFCF` · soft `#F7D384` · signature `#EFB13C` · strong `#B07512` · deep `#6E4D12` · on-gold text `#3D2A06`
- Neutrals (warm, light): page `#FBF8F1` · surface `#FFFDF8` · border `#E9E1D2` · muted text `#8C7E68` · ink `#2E2519`
- Accent: violet `#7E63C4` (light) / `#B49BE8` (on dusk) — gold + violet = the colors of dawn; used sparingly.
- Status: success `#4E9A4E` · warning `#DD6B2A` (shifted off-amber so it doesn't collide with gold) · error `#C8443B` · info `#3E7BC2`
- Warm dusk (dark): bg `#211A12` · surface `#2C241A` · border `#443A2C` · text `#F2E9D8` · muted `#B0A18A`

These are wired into `apps/web/src/styles.scss` as `--hope-*` brand tokens and as overrides on Material's `--mat-sys-*` system variables (light + warm-dusk dark).
