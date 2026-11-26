# Glaze

`@glave/glaze`: an Angular component library for any app. Accessible (WCAG 2.2 AA, axe in every test), themed only through CSS custom properties, built on the web platform first and the Angular CDK second. Open source; it will be published as 0.x once its API settles.

See every component live in the showcase (`npm run showcase` in `web/`), a dev-only app that ships in no package. Coding agents: [AGENTS.md](AGENTS.md) says how to use it. A Glaze CLI (a schematic) will later add that guidance to a consuming project's own AGENTS.md.

## What is in it

Only what an app uses today: a piece no app uses leaves Glaze. [`src/public-api.ts`](src/public-api.ts) lists every export, and each one's doc comment has a short usage example; the global CSS (tokens, layout primitives, prose flow and motion) is `styles/glaze.css`.

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

Override tokens, nothing else: [AGENTS.md](AGENTS.md), "Theming", says which a theme sets and how light and dark are chosen.

## Structure

`src/components/<name>/<name>.component.{ts,html,scss,spec.ts}`, `src/directives/`, `src/lib/` (pure functions), `styles/` (the global CSS), `showcase/` (the showcase app). `npm run build:glaze` builds the package with ng-packagr into `dist/glaze`.
