import {
  ChangeDetectionStrategy,
  Component,
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

import { testIds } from '../../lib/test-ids';
import { GLZ_FORM_FIELD, bindValueAccessor, injectFormState } from '../../tokens/forms';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { input: 'input' } as const;

/**
 * A checkbox with its label: a native checkbox (keyboard, screen readers and forms all as the
 * browser has them) drawn in Glaze's style. Binds with reactive forms, `ngModel`, signal
 * forms or `[(checked)]`.
 *
 * ```html
 * <glz-checkbox [(checked)]="remember">Remember this choice</glz-checkbox>
 * <glz-checkbox [formControl]="terms">I agree</glz-checkbox>
 * ```
 */
@Component({
  selector: 'glz-checkbox',
  templateUrl: './checkbox.component.html',
  styleUrl: './checkbox.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.data-testid]': 'testId()' },
})
export class GlzCheckboxComponent implements ControlValueAccessor, FormCheckboxControl {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** Whether it is checked. */
  readonly checked = model(false);
  readonly disabled = input(false, { transform: booleanAttribute });
  /** Shown as partly checked, as for a "select all" over a mixed selection. */
  readonly indeterminate = input(false, { transform: booleanAttribute });
  /** Emitted when focus leaves it: it has been touched. */
  readonly touch = output();

  private readonly field = inject(GLZ_FORM_FIELD, { optional: true });
  private readonly state = injectFormState();
  private readonly formDisabled = signal(false);
  private onChanged: (checked: boolean) => void = () => undefined;
  private onTouched: () => void = () => undefined;

  private readonly showError = computed(() => this.state.invalid() && this.state.touched());
  protected readonly isDisabled = computed(() => this.disabled() || this.formDisabled());
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

  /** Follows the native checkbox after the person changes it. */
  protected changed(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.checked.set(event.target.checked);
      this.onChanged(event.target.checked);
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
