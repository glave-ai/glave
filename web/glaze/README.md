# Glaze

`@glave/glaze`: glave's Angular component library. Accessible (WCAG 2.2 AA, axe in every test), themed only through CSS custom properties, built on the web platform first and the Angular CDK second. Open source with glave; it will be published as 0.x once the app's design settles.

See every component live at `/glaze` in the app (`npm start` in `web/`). Coding agents: [AGENTS.md](AGENTS.md) says how to use it. A Glaze CLI (a schematic) will later add that guidance to a consuming project's own AGENTS.md.

## What is in it

**Actions and display**: `button[glzButton]`, `button[glzIconButton]`, `glz-icon`, `glz-badge`, `glz-avatar`, `glz-card`, `glz-empty-state`, `glz-progress`, `glz-spinner`, `glz-skeleton`.

**Forms**: `glz-form-field` with `glz-error`; `glzInput` on native inputs, textareas and selects (date and time included); `glzSlider` on range inputs; `glzFieldControl` for any other native control; `glz-checkbox`, `glz-switch`, `glz-radio-group` with `glz-radio`, `glz-combobox`. Reactive forms, `ngModel`, signal forms and `model()` all bind.

**Layout and navigation**: `glz-sidenav-layout`, `glz-top-bar`, `glz-rail` with `glzRailItem`, `glz-nav-list` with `glzNavItem`, `glz-tabs` with `glz-tab`, `glz-breadcrumbs` with `glz-breadcrumb`.

**Overlays**: `glz-dialog` (dialog or drawer), `glz-confirm-dialog`, `glz-menu` with `glzMenuItem` and `glzMenuTrigger`, `glzTooltip`, `glz-toast-stack` with `GlzToastService`.

**Conversation**: `glz-composer`, `glz-message`, `glz-reasoning`, `glz-receipt`, `glz-approval-card`, `glz-question-card`.

**Behaviour**: `glzStickToBottom`, `glzCopy`, `glzShortcut`, `glzClickOutside`, `glzAutofocus`, `glzScrollSpy`.

Each component's doc comment has a short usage example.

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

Override tokens, nothing else:

```css
:root {
  --glz-color-accent: light-dark(#0b5cad, #7ab8ff);
  --glz-radius-md: 4px;
  --glz-font-sans: 'Inter', system-ui, sans-serif;
}
```

Light and dark follow the system; `data-theme="light"` or `"dark"` on `<html>` forces one.

## Structure

`src/components/<name>/<name>.component.{ts,html,scss,spec.ts}`, `src/directives/`, `src/services/` (the toast queue only), `src/lib/` (pure functions), `styles/` (the global CSS). `npm run build:glaze` builds the package with ng-packagr into `dist/glaze`.
