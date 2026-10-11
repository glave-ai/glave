import {
  Directive,
  booleanAttribute,
  computed,
  inject,
  input,
  model,
  output,
  signal,
} from '@angular/core';
import type { ControlValueAccessor } from '@angular/forms';
import type { FormCheckboxControl } from '@angular/forms/signals';

import { GLZ_FORM_FIELD, bindValueAccessor, injectFormState } from '../lib/forms';

/**
 * What a checkbox and a switch share: a `checked` model bound to reactive forms, `ngModel`,
 * signal forms (`[formField]`) or `[(checked)]`; `disabled`; a `touch` output; and the form
 * state for a form field around it. Not used on its own.
 */
@Directive()
export abstract class GlzCheckableDirective implements ControlValueAccessor, FormCheckboxControl {
  /** Whether it is on. */
  readonly checked = model(false);
  readonly disabled = input(false, { transform: booleanAttribute });
  /** Emitted when focus leaves it: it has been touched. */
  readonly touch = output();

  private readonly field = inject(GLZ_FORM_FIELD, { optional: true });
  private readonly state = injectFormState();
  private readonly formDisabled = signal(false);
  private onChanged: (checked: boolean) => void = () => undefined;
  private onTouched: () => void = () => undefined;

  readonly isDisabled = computed(() => this.disabled() || this.formDisabled());
  readonly showError = computed(() => this.state.invalid() && this.state.touched());
  protected readonly describedBy = computed(() => this.field?.describedBy() ?? null);
  protected readonly ariaInvalid = computed(() => (this.showError() ? 'true' : null));
  protected readonly ariaRequired = computed(() => (this.state.required() ? 'true' : null));

  constructor() {
    bindValueAccessor(this);
    this.field?.register({
      showError: this.showError,
      messages: this.state.messages,
      required: this.state.required,
      labelable: false,
    });
  }

  /** Follows the native input after the person changes it. */
  protected changed(event: Event): void {
    const target = event.target;
    if (target instanceof HTMLInputElement) {
      this.checked.set(target.checked);
      this.onChanged(target.checked);
    }
  }

  protected blurred(): void {
    this.onTouched();
    this.touch.emit();
  }

  writeValue(value: unknown): void {
    this.checked.set(value === true);
  }

  registerOnChange(fn: (checked: boolean) => void): void {
    this.onChanged = fn;
  }

  registerOnTouched(fn: () => void): void {
    this.onTouched = fn;
  }

  setDisabledState(disabled: boolean): void {
    this.formDisabled.set(disabled);
  }
}
