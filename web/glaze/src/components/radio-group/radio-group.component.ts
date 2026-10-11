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
import type { FormValueControl } from '@angular/forms/signals';

import { GLZ_FORM_FIELD, bindValueAccessor, injectFormState } from '../../lib/forms';
import { injectId } from '../../lib/ids';

/**
 * A group of radio buttons, one of which may be picked. Each `<glz-radio>` is a native radio
 * input, so arrow keys move between them and screen readers announce the group. Works with
 * reactive forms, `ngModel`, signal forms (`[formField]`) or `[(value)]`. Name it with a
 * `<glz-form-field>` around it, or with `label`.
 *
 * ```html
 * <glz-radio-group label="Approval" [(value)]="mode">
 *   <glz-radio value="ask">Ask every time</glz-radio>
 *   <glz-radio value="auto">Auto</glz-radio>
 * </glz-radio-group>
 * ```
 */
@Component({
  selector: 'glz-radio-group',
  templateUrl: './radio-group.component.html',
  styleUrl: './radio-group.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    role: 'radiogroup',
    '[attr.aria-labelledby]': 'labelledBy()',
    '[attr.aria-label]': 'ariaLabel()',
    '[attr.aria-describedby]': 'describedBy()',
    '[attr.aria-invalid]': 'ariaInvalid()',
    '[attr.aria-required]': 'ariaRequired()',
    '[attr.aria-disabled]': 'ariaDisabled()',
  },
})
export class GlzRadioGroupComponent
  implements ControlValueAccessor, FormValueControl<string | null>
{
  /** The value of the radio picked, or `null`. */
  readonly value = model<string | null>(null);
  readonly disabled = input(false, { transform: booleanAttribute });
  /** The group's name, when no form field names it. */
  readonly label = input<string>('');
  /** Emitted when focus leaves a radio: the group has been touched. */
  readonly touch = output();

  /** The name shared by the group's native radios. */
  readonly groupName = injectId('glz-radio-group');

  private readonly field = inject(GLZ_FORM_FIELD, { optional: true });
  private readonly state = injectFormState();
  private readonly formDisabled = signal(false);
  private onChanged: (value: string | null) => void = () => undefined;
  private onTouched: () => void = () => undefined;

  /** Whether every radio in the group is disabled. */
  readonly isDisabled = computed(() => this.disabled() || this.formDisabled());
  readonly showError = computed(() => this.state.invalid() && this.state.touched());

  protected readonly labelledBy = computed(() => this.field?.labelId ?? null);
  protected readonly ariaLabel = computed(() =>
    this.field === null && this.label() !== '' ? this.label() : null,
  );
  protected readonly describedBy = computed(() => this.field?.describedBy() ?? null);
  protected readonly ariaInvalid = computed(() => (this.showError() ? 'true' : null));
  protected readonly ariaRequired = computed(() => (this.state.required() ? 'true' : null));
  protected readonly ariaDisabled = computed(() => (this.isDisabled() ? 'true' : null));

  constructor() {
    bindValueAccessor(this);
    this.field?.register({
      showError: this.showError,
      messages: this.state.messages,
      required: this.state.required,
      labelable: false,
    });
  }

  /** Picks `value`, as a radio in the group does when chosen. */
  pick(value: string): void {
    this.value.set(value);
    this.onChanged(value);
  }

  /** Marks the group touched, as a radio does when focus leaves it. */
  markTouched(): void {
    this.onTouched();
    this.touch.emit();
  }

  writeValue(value: unknown): void {
    this.value.set(typeof value === 'string' ? value : null);
  }

  registerOnChange(fn: (value: string | null) => void): void {
    this.onChanged = fn;
  }

  registerOnTouched(fn: () => void): void {
    this.onTouched = fn;
  }

  setDisabledState(disabled: boolean): void {
    this.formDisabled.set(disabled);
  }
}
