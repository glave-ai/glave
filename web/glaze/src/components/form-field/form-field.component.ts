import { ChangeDetectionStrategy, Component, computed, input, signal } from '@angular/core';

import { GLZ_FORM_FIELD, type GlzFieldControl, type GlzFieldHost } from '../../lib/forms';
import { injectId } from '../../lib/ids';
import { GlzErrorComponent } from '../error/error.component';

/** The mark after a required field's label; screen readers hear "required" from the control. */
const REQUIRED_MARK = '*';

/**
 * A labelled form control with an optional hint and its errors. The label, hint and errors
 * are tied to the control for screen readers, and errors are announced as they appear. It
 * holds any Glaze control, or a native `input`, `textarea` or `select` marked `glzInput`,
 * under reactive forms, `ngModel` or signal forms.
 *
 * Errors show once the control is invalid and touched: signal forms' messages by themselves,
 * and with reactive forms the `<glz-error>` elements projected.
 *
 * ```html
 * <glz-form-field label="Name" hint="Everyone in the space sees it">
 *   <input glzInput [formControl]="name" />
 *   @if (nameMissing()) { <glz-error>Give it a name.</glz-error> }
 * </glz-form-field>
 * ```
 */
@Component({
  selector: 'glz-form-field',
  imports: [GlzErrorComponent],
  providers: [{ provide: GLZ_FORM_FIELD, useExisting: GlzFormFieldComponent }],
  templateUrl: './form-field.component.html',
  styleUrl: './form-field.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GlzFormFieldComponent implements GlzFieldHost {
  readonly label = input.required<string>();
  readonly hint = input<string>('');

  readonly id = injectId('glz-field');
  readonly labelId = `${this.id}-label`;
  protected readonly hintId = `${this.id}-hint`;
  protected readonly errorId = `${this.id}-error`;
  protected readonly requiredMark = REQUIRED_MARK;

  private readonly control = signal<GlzFieldControl | undefined>(undefined);
  protected readonly showError = computed(() => this.control()?.showError() ?? false);
  protected readonly required = computed(() => this.control()?.required() ?? false);
  protected readonly messages = computed(() => this.control()?.messages() ?? []);
  protected readonly labelable = computed(() => this.control()?.labelable ?? true);
  protected readonly showHint = computed(() => this.hint() !== '' && !this.showError());

  readonly describedBy = computed(() => {
    if (this.showError()) {
      return this.errorId;
    }
    return this.showHint() ? this.hintId : null;
  });

  /** Called by the control inside. */
  register(control: GlzFieldControl): void {
    this.control.set(control);
  }
}
