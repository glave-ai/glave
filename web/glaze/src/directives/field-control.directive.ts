import { Directive, computed, inject } from '@angular/core';

import { GLZ_FORM_FIELD, injectFormState } from '../tokens/forms';

/**
 * Ties a native form element to the `<glz-form-field>` around it: the field's id (so its
 * label names the element), `aria-describedby` for the hint or the error shown,
 * `aria-invalid` once invalid and touched, and `aria-required`. `glzInput` and `glzSlider`
 * apply it; use it directly on any other native control.
 *
 * ```html
 * <glz-form-field label="Colour"><input type="color" glzFieldControl [formControl]="colour" /></glz-form-field>
 * ```
 */
@Directive({
  selector: '[glzFieldControl]',
  host: {
    '[attr.id]': 'id()',
    '[attr.aria-describedby]': 'describedBy()',
    '[attr.aria-invalid]': 'ariaInvalid()',
    '[attr.aria-required]': 'ariaRequired()',
  },
})
export class GlzFieldControlDirective {
  private readonly field = inject(GLZ_FORM_FIELD, { optional: true });
  private readonly state = injectFormState();

  /** Whether the control is shown as invalid: invalid and touched. */
  readonly showError = computed(() => this.state.invalid() && this.state.touched());

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
