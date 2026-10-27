import { Directive, computed, inject } from '@angular/core';

import { INERT_UNTIL_BOUND } from '../lib/inert-until-bound';
import { GLZ_FORM_FIELD, injectFormState } from '../tokens/forms';

/**
 * Glaze's look and form-field wiring for a native `input`, `textarea` or `select`. The element
 * stays native, so the browser's keyboard, autofill and screen-reader behaviour all apply,
 * and Angular's own value accessors bind it to reactive forms, `ngModel` or signal forms. In a
 * `<glz-form-field>` the field's label names it, its hint or error describes it, and it says
 * whether it is required and, once touched, invalid.
 *
 * Text, email, number, password, search, date and time inputs all work the same way.
 *
 * ```html
 * <glz-form-field label="Due">
 *   <input glzInput type="date" [formControl]="due" />
 * </glz-form-field>
 * ```
 */
@Directive({
  selector: 'input[glzInput], textarea[glzInput], select[glzInput]',
  host: {
    ...INERT_UNTIL_BOUND,
    class: 'glz-input',
    '[attr.id]': 'id()',
    '[attr.aria-describedby]': 'describedBy()',
    '[attr.aria-invalid]': 'ariaInvalid()',
    '[attr.aria-required]': 'ariaRequired()',
  },
})
export class GlzInputDirective {
  private readonly field = inject(GLZ_FORM_FIELD, { optional: true });
  private readonly state = injectFormState();

  /** Whether it is shown as invalid: invalid and touched. */
  private readonly showError = computed(() => this.state.invalid() && this.state.touched());

  protected readonly id = computed(() => this.field?.id ?? null);
  protected readonly describedBy = computed(() => this.field?.describedBy() ?? null);
  protected readonly ariaInvalid = computed(() => (this.showError() ? 'true' : null));
  protected readonly ariaRequired = computed(() => (this.state.required() ? 'true' : null));

  constructor() {
    this.field?.register({
      showError: this.showError,
      messages: this.state.messages,
      required: this.state.required,
      labelable: true,
    });
  }
}
