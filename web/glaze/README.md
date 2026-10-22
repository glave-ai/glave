# Glaze

`@glave/glaze`: an Angular component library for any app. Accessible (WCAG 2.2 AA, axe in every test), themed only through CSS custom properties, built on the web platform first and the Angular CDK second. Open source; it will be published as 0.x once its API settles.

See every component live in the showcase (`npm run showcase` in `web/`), a dev-only app that ships in no package. Coding agents: [AGENTS.md](AGENTS.md) says how to use it. A Glaze CLI (a schematic) will later add that guidance to a consuming project's own AGENTS.md.

## What is in it

Only what an app uses today: a piece no app uses leaves Glaze.

- **Styles** (`styles/glaze.css`): the tokens, the layout primitives (`glz-stack`, `glz-row`, `glz-grid`, `glz-gap-*`), the prose flow (`glz-flow`), and what moves: `glz-spin`, which turns a ring while work goes on, `glz-rise` and `glz-settle`, in the motion tokens alone.
- **Actions and display**: `glzButton`, `glzIconButton`, `glz-icon`, `glz-badge`, `glz-avatar`, `glz-card`, `glz-empty-state`.
- **Forms**: `glz-form-field`, `glz-error`, `glzInput`, `glz-checkbox`, `glz-radio-group`, `glz-radio`.
- **Layout**: `glz-sidenav-layout`, `glz-panel-layout`, `glz-top-bar`, `glz-nav-list`, `glzNavItem`, `glz-tabs`, `glz-tab`.
- **Overlays**: `glz-dialog`, `glz-confirm-dialog`, `glz-menu-button`, `glz-menu-select`, `glz-picker`, `glz-popover` with `glzPopoverTrigger`, `glzTooltip`.
- **Conversation**: `glz-composer`, `glz-message`, `glz-reasoning`, `glz-receipt`, `glz-steps`, `glz-approval-card`, `glz-question-card`, `glzStickToBottom`, `glzCopy`.

`src/public-api.ts` lists every export; each one's doc comment has a short usage example.

## Example

```ts
import {
  GlzButtonComponent,
  GlzFormFieldComponent,
  GlzInputDirective,
  GlzErrorComponent,
} from '@glave/glaze';

@Component({
  imports: [
    ReactiveFormsModule,
    GlzFormFieldComponent,
    GlzInputDirective,
    GlzErrorComponent,
    GlzButtonComponent,
  ],
  templateUrl: './rename.component.html',
})
export class RenameComponent {
  readonly title = new FormControl('', { nonNullable: true, validators: [Validators.required] });
  readonly missing = toSignal(
    this.title.statusChanges.pipe(map(() => this.title.hasError('required'))),
  );
}
```

```html
<glz-form-field label="Title" hint="Shown in the thread list">
  <input glzInput [formControl]="title" />
  @if (missing()) {
  <glz-error>Give the thread a title.</glz-error>
  }
</glz-form-field>
<button glzButton variant="primary">Save</button>
```

## Theming

Override tokens, nothing else. The accent is one colour; the text on it, its hover and its tints follow from it:

```css
:root {
  --glz-accent: #0b5cad;
  --glz-font-sans: 'Inter', system-ui, sans-serif;
}
```

Light and dark follow the system; `data-theme="light"` or `"dark"` on `<html>` forces one.

## Structure

`src/components/<name>/<name>.component.{ts,html,scss,spec.ts}`, `src/directives/`, `src/lib/` (pure functions), `styles/` (the global CSS), `showcase/` (the showcase app). `npm run build:glaze` builds the package with ng-packagr into `dist/glaze`.
