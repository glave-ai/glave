import { ChangeDetectionStrategy, Component } from '@angular/core';

/**
 * An error message for the control in a `<glz-form-field>`, shown only once the control is
 * invalid and touched. Show the one that applies with `@if`.
 *
 * ```html
 * @if (nameMissing()) { <glz-error>Give it a name.</glz-error> }
 * ```
 */
@Component({
  selector: 'glz-error',
  templateUrl: './error.component.html',
  styleUrl: './error.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GlzErrorComponent {}
