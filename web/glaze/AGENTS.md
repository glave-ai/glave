# Using Glaze (for AI coding agents)

Glaze (`@glave/glaze`) is an Angular 22 component library: standalone, zoneless, signals. It builds on the **web platform first**, then the **Angular CDK**, then its own code. Follow this page when building an app with it.

## Set up

- Global styles, once: `@glave/glaze/styles/glaze.css` (tokens, page base, native-control styles, CDK overlay styles), plus the fonts (`@fontsource/atkinson-hyperlegible-next`, `-mono`).
- Place `<glz-toast-stack />` once, at the end of the root template.
- Import each component or directive where it is used: `imports: [GlzButtonComponent, GlzFormFieldComponent, GlzInputDirective]`. There are no modules.
- Icons are Lucide (ISC): `import { Send } from 'lucide'` and `<glz-icon [icon]="send" />`, or register names with `provideGlazeIcons({ send: Send })`. Only imported icons are bundled.

## Theming

- Only CSS custom properties. Override them on `:root`, or on any element for part of the page: `--glz-color-accent`, `--glz-surface-card`, `--glz-radius-md`, `--glz-space-4`, `--glz-font-sans`, `--glz-elevation-2`, and so on (`styles/tokens.css` lists them all). Never style Glaze's internals by class.
- Surfaces, page up: `--glz-surface-page`, `-sunken`, `-card`, `-raised`, `-overlay`. `<glz-card surface="raised">` picks one.
- Light and dark come from CSS alone (`color-scheme` and `light-dark()`), following the system. To force one, set `data-theme="light"` or `"dark"` on `<html>`; storing the person's choice is the app's job.
- Sun gold (`--glz-color-hope`) is only for Hope's presence (`<glz-avatar hope>`).

## Forms

- Native controls get `glzInput`: `<input glzInput>`, `<textarea glzInput>`, `<select glzInput>`, `type="date"` too. Sliders: `<input type="range" glzSlider>`. Angular's own value accessors bind them.
- Custom controls: `glz-checkbox`, `glz-switch` (`[(checked)]`), `glz-radio-group` with `glz-radio`, `glz-combobox` (`[options]`, `[(value)]`). Each works with reactive forms (`formControlName`), `ngModel`, signal forms (`[formField]`) and two-way `model()` binding.
- Wrap every control in `<glz-form-field label="…" hint="…">`: it labels it, describes it and shows errors once it is invalid and touched. Reactive forms: project `<glz-error>` per error under `@if`. Signal forms: give validators a `message`; the field shows it.
- Grow a textarea with its text by CSS: Glaze's composer does; elsewhere use `field-sizing: content` and a `max-height`.

## The conversation

- `glz-composer`: `[(value)]`, `[(files)]`, `[busy]`, `(send)` with `{ text, files }`, `(stop)`. Enter sends, Shift+Enter is a new line, Escape stops while busy. Slots: `glzComposerStart` (model picker), `glzComposerEnd` (toggles).
- `glz-message` (`from`, `author`, `hope`, `streaming`) holds `glz-reasoning`, `glz-receipt` (a tool call: `tool`, `summary`, `status`, `durationMs`, `tokens`, `cost`, evidence projected), `glz-approval-card` (`(decide)` emits `allow_once`, `allow_for_thread` or `deny`; set `[decision]` to show it in place) and `glz-question-card` (`(answer)` emits one string per question; set `[answered]`).
- Put the message list in a scrolling element with `glzStickToBottom` (`[(atBottom)]`, `scrollToBottom()`).

## Layout and overlays

- Page: `glz-sidenav-layout` (`[glzSidenav]` slot; drawer on narrow screens), `glz-top-bar` (`glzTopBarStart`, `glzTopBarEnd`), `glz-rail` with `a[glzRailItem]`, `glz-nav-list` with `a[glzNavItem]` (`current`, `glzNavItemEnd` for a badge), `glz-tabs` with `glz-tab`, `glz-breadcrumbs`.
- `glz-dialog` (`[(open)]`, `kind="drawer-end"` for a drawer) and `glz-confirm-dialog` (`(decided)`) are native `<dialog>`s. Open them by setting `open`.
- Menus: `[glzMenuTrigger]="tpl"` on a button, `<ng-template #tpl><glz-menu><button glzMenuItem (triggered)>`. Tooltips: `[glzTooltip]="text"`, hints only. Toasts: `inject(GlzToastService).show(message, { tone, action })`.
- Behaviour directives: `glzCopy`, `glzShortcut="mod+k"`, `glzClickOutside`, `glzAutofocus`, `glzScrollSpy`.

## Rules

- Do: label every control (form field or `label`); give icon buttons a `label`; put a `data-testid` on what tests touch; keep templates to control flow and signal reads; check with axe.
- Do not: add inline styles, `!important` overrides of Glaze classes, a second toast stack, or state to Glaze (app state lives in the app's services); do not use colour alone to say something.
- Prefer the platform: native elements keep their keyboard, autofill and screen-reader behaviour.

## Platform choices and their support

Checked against Baseline (web-features 3.42, October 2026). "Newly" means newly available; "widely" means widely available.

| Need                                | Glaze uses                                                                         | Baseline                                          |
| ----------------------------------- | ---------------------------------------------------------------------------------- | ------------------------------------------------- |
| Modal dialogs, drawers              | `<dialog>` with `showModal()`: top layer, inert page, focus, Escape                | Widely (2024)                                     |
| Backdrop click to close             | Own handler (`closedby` is not Baseline)                                           | `dialog-closedby`: no                             |
| Opening dialogs                     | The `open` model; invoker commands (`commandfor`) can come later                   | `invoker-commands`: newly (Dec 2025)              |
| Toasts, tooltips, combobox list     | The popover API (`popover="manual"`)                                               | Newly (Jan 2025)                                  |
| Placing popovers                    | CSS anchor positioning where present, else measured (`lib/popover.ts`)             | `anchor-positioning`: no                          |
| `popover="hint"`, interest invokers | Not used                                                                           | No                                                |
| Reasoning, receipts                 | `<details>` / `<summary>`                                                          | Widely (2020); `name` for accordions newly (2024) |
| Textarea growing                    | CSS `field-sizing: content`; without it the box keeps its rows                     | Newly (Jun 2026)                                  |
| Light and dark                      | `color-scheme`, `light-dark()`                                                     | Widely; `light-dark()` newly (2024)               |
| Content behind a drawer             | `inert`                                                                            | Widely (2025)                                     |
| Invalid styling                     | `aria-invalid` once touched (`:user-invalid` is widely available, for plain forms) | Widely (2023)                                     |
| Keyboard models                     | CDK `ActiveDescendantKeyManager` (combobox), `FocusKeyManager` (tabs), CDK Menu    | The platform has none                             |
| Announcements                       | CDK `LiveAnnouncer`                                                                | The platform has none                             |
| Clipboard                           | CDK `Clipboard`                                                                    | —                                                 |
| Breakpoints                         | CDK `BreakpointObserver` (the side nav's modal state needs script)                 | —                                                 |
