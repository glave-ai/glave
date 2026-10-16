# Using Glaze (for AI coding agents)

Glaze (`@glave/glaze`) is an Angular 22 component library: standalone, zoneless, signals. It builds on the **web platform first**, then the **Angular CDK**, then its own code. Follow this page when building an app with it.

## Set up

- Global styles, once: `@glave/glaze/styles/glaze.css` (tokens, page base, layout primitives and prose flow, native-control styles, CDK overlay styles), plus the fonts (`@fontsource/atkinson-hyperlegible-next`, `-mono`).
- Import each component or directive where it is used: `imports: [GlzButtonComponent, GlzFormFieldComponent, GlzInputDirective]`. There are no modules.
- Icons are Lucide (ISC): `import { Send } from 'lucide'` and `<glz-icon [icon]="send" />`. Only imported icons are bundled.

## Theming

- Only CSS custom properties (`styles/tokens.css` lists them all). Never style Glaze's internals by class.
- The look is monochrome: warm greys, and one accent, ink by default. A theme sets `--glz-accent` alone, on `:root`; the colour on it (`--glz-on-accent`), its hover, its soft tint and the accent as text follow in CSS. Focus has its own quiet ring (`--glz-color-focus`), never the accent.
- Surfaces, page up: `--glz-surface-page`, `-sidebar`, `-sunken`, `-card`, `-raised`, `-overlay`; `-bubble` under the person's messages. `<glz-card surface="raised">` picks one.
- Light and dark come from CSS alone (`color-scheme` and `light-dark()`), following the system. To force one, set `data-theme="light"` or `"dark"` on `<html>`; storing the person's choice is the app's job.

## Space and layout

- Parents own spacing: no component sets its own outer margin; the space between things is the parent's `gap`. Lay a page out with the primitives, plain classes: `glz-stack` (one above another), `glz-row` (side by side, wrapping), `glz-grid` (as many columns as fit, `--glz-grid-min` wide), each spaced by a `glz-gap-*` class (`half`, `0` to `8`) or a `gap` in your own styles.
- Gaps come from the space scale, `--glz-space-half` and `--glz-space-1` to `-8`; text sizes from `--glz-text-xs` to `-4xl`. Both are fluid: they grow with `clamp()` from a phone (360px) to a wide screen (1440px).
- The padding inside a box is its kind's inset: `--glz-inset-row` (nav items, options), `-control` (inputs), `-button`, `-strip`, `-bubble`, `-card`, `-composer`, `-dialog`, `-pane-block` and `-pane-inline` (side panes), `-page-block` and `-page-inline`. Its corners are its kind's radius: `--glz-radius-mark`, `-row`, `-control`, `-strip`, `-card`, `-composer`.
- Running text flows: give its element `glz-flow` and it spaces its own blocks (paragraphs, lists, headings, code) and sits flush in what holds it. Markdown does this already.
- A row of buttons projected into a slot (`glzCardFooter`, `glzDialogActions`) is a `glz-row glz-gap-2`.

## Forms

- Native controls get `glzInput`: `<input glzInput>`, `<textarea glzInput>`, `<select glzInput>`, `type="date"` too. Angular's own value accessors bind them.
- Custom controls: `glz-checkbox` (`[(checked)]`), `glz-radio-group` with `glz-radio`. Each works with reactive forms (`formControlName`), `ngModel`, signal forms (`[formField]`) and two-way `model()` binding.
- Wrap every control in `<glz-form-field label="…" hint="…">`: it labels it, describes it and shows errors once it is invalid and touched. Reactive forms: project `<glz-error>` per error under `@if`. Signal forms: give validators a `message`; the field shows it.
- Grow a textarea with its text by CSS: Glaze's composer does; elsewhere use `field-sizing: content` and a `max-height`.

## The conversation

- `glz-composer`: `[(value)]`, `[busy]`, `(send)` with the trimmed text, `(stop)`. Enter sends, Shift+Enter is a new line, Escape stops while busy. Slots: `glzComposerStart` and `glzComposerEnd`, either side of the toolbar.
- `glz-message` (`from`, `author`, `streaming`; no avatar and no name over it: the person's own a bubble at the end of the line, `--glz-message-gap` before it and `--glz-message-person-surface` under it; a reply plain full-width text; `author` names the article for screen readers; actions such as copy in the `glzMessageActions` slot, hung just under it in the gap before what follows, shown on hover and focus, always on touch) holds `glz-reasoning`, `glz-receipt` (a tool call, one quiet line: `icon`, `tool`, `summary`, `status`, evidence projected), `glz-steps` (consecutive receipts folded under one disclosure strip: `summary` such as "Ran 4 steps", `[(open)]`), `glz-approval-card` (a card that needs the person, as `glz-question-card` is; `(decide)` emits `allow_once`, `allow_for_thread` or `deny`; set `[decision]` to show it in place) and `glz-question-card` (`(answer)` emits one string per question; set `[answered]`).
- A reply's text: `<div [glzMarkdown]="text" [streaming]="writing">` renders GitHub-flavoured markdown (tables, task lists) into that element, flush at its top and bottom. It is safe whatever the text says: raw HTML shows as text, an image shows as a link to it and is never fetched, links open in a new tab and say so, and Angular sanitizes the HTML again on its Trusted Types policy. Each block renders once; while `streaming`, only the growing last one is redone, and half-written markdown shows as it will end. Code blocks get a copy button (`<testId>-copy`) and highlighting, loaded the first time a language appears. The person's own text stays plain.
- `<glz-copy-button [text]="text" testId="…">` copies text and says so (a check, "Copied", announced); in a message's `glzMessageActions`, and on markdown's code blocks.
- Put the message list in a scrolling element with `glzStickToBottom` (`[(atBottom)]`, `scrollToBottom()`).

## Layout and overlays

- Page: `glz-sidenav-layout` (`[glzSidenav]` slot; a drawer on narrow screens; `--glz-sidenav-width`, `--glz-sidenav-surface`) and `glz-panel-layout` (`[glzPanel]` slot at the end; a sheet on narrow screens; `--glz-panel-width`, `--glz-panel-surface`). Both bind `[(open)]` and `[(width)]` (pixels) and, on wide screens, make their edge a window splitter (`glzSplitter`, a focusable `separator`): drag it or use the arrow keys, Home and End to resize; drag it past half the minimum or press Enter to collapse; the collapsed edge stays as a slim grip to drag, click or Enter back. Give the edge a `resizeLabel`, a `resizeHint` (its tooltip) and the page's toggle `shortcut` (`aria-keyshortcuts`). `glz-top-bar` (`glzTopBarStart`, `glzTopBarEnd`; `--glz-top-bar-surface` and `--glz-top-bar-border`, so it can float over a page), `glz-nav-list` with `a[glzNavItem]` (`current`, `glzNavItemEnd` for a badge; `collapsible` with `[(expanded)]` and `glzNavListActions` beside its heading), `glz-tabs` with `glz-tab`. Remembering a panel's width or a group's state is the app's job.
- `glz-dialog` (`[(open)]`) and `glz-confirm-dialog` (`(decided)`) are native `<dialog>`s. Open them by setting `open`.
- A compact choice among a few options: `<glz-menu-select label="Model" [options] [(value)]>`, options with `hint`. A choice among many (a model catalogue): `<glz-picker label="Model" searchLabel="Search models" [options] [(value)]>`, a search box over a scrolling listbox (the combobox pattern, the CDK's `ActiveDescendantKeyManager`), in the options' own order (putting the likeliest first is the app's job); the search box is free of autocorrect, capitals and spellcheck. Tooltips: `[glzTooltip]="text"`, hints only.
- `[glzCopy]="text"` copies on click; `(copied)` says whether it worked.

## Rules

- Do: label every control (form field or `label`); give icon buttons a `label`; give components tests reach a `testId` (their parts get `<testId>-<part>`, such as `composer-send`), and plain elements a `data-testid`; keep templates to control flow and signal reads; check with axe.
- Do not: add inline styles, `!important` overrides of Glaze classes, or state to Glaze (app state lives in the app's services); do not use colour alone to say something.
- Prefer the platform: native elements keep their keyboard, autofill and screen-reader behaviour.

## Platform choices and their support

Checked against Baseline (web-features 3.42, October 2026). "Newly" means newly available; "widely" means widely available.

| Need                                | Glaze uses                                                                         | Baseline                                          |
| ----------------------------------- | ---------------------------------------------------------------------------------- | ------------------------------------------------- |
| Modal dialogs                       | `<dialog>` with `showModal()`: top layer, inert page, focus, Escape                | Widely (2024)                                     |
| Backdrop click to close             | Own handler (`closedby` is not Baseline)                                           | `dialog-closedby`: no                             |
| Opening dialogs                     | The `open` model; invoker commands (`commandfor`) can come later                   | `invoker-commands`: newly (Dec 2025)              |
| Tooltips, the picker's list         | The popover API (`popover="manual"`)                                               | Newly (Jan 2025)                                  |
| Placing popovers                    | CSS anchor positioning where present, else measured (`lib/popover.ts`)             | `anchor-positioning`: no                          |
| `popover="hint"`, interest invokers | Not used                                                                           | No                                                |
| Reasoning, receipts, steps          | `<details>` / `<summary>`                                                          | Widely (2020); `name` for accordions newly (2024) |
| Textarea growing                    | CSS `field-sizing: content`; without it the box keeps its rows                     | Newly (Jun 2026)                                  |
| Light and dark                      | `color-scheme`, `light-dark()`                                                     | Widely; `light-dark()` newly (2024)               |
| Content behind a drawer             | `inert`                                                                            | Widely (2025)                                     |
| Invalid styling                     | `aria-invalid` once touched (`:user-invalid` is widely available, for plain forms) | Widely (2023)                                     |
| Keyboard models                     | CDK `ActiveDescendantKeyManager` (picker), `FocusKeyManager` (tabs), CDK Menu      | The platform has none                             |
| Announcements                       | CDK `LiveAnnouncer`                                                                | The platform has none                             |
| Clipboard                           | CDK `Clipboard`                                                                    | —                                                 |
| Breakpoints                         | CDK `BreakpointObserver` (the side nav's modal state needs script)                 | —                                                 |
