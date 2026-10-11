import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  inject,
  input,
} from '@angular/core';

import { GlzRadioGroupComponent } from '../radio-group/radio-group.component';

/**
 * One choice in a `<glz-radio-group>`: a native radio input with its label.
 *
 * ```html
 * <glz-radio value="auto">Auto</glz-radio>
 * ```
 */
@Component({
  selector: 'glz-radio',
  templateUrl: './radio.component.html',
  styleUrl: './radio.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GlzRadioComponent {
  readonly value = input.required<string>();
  readonly disabled = input(false, { transform: booleanAttribute });

  protected readonly group = inject(GlzRadioGroupComponent);
  protected readonly checked = computed(() => this.group.value() === this.value());
  protected readonly isDisabled = computed(() => this.disabled() || this.group.isDisabled());

  protected pick(): void {
    this.group.pick(this.value());
  }

  protected blurred(): void {
    this.group.markTouched();
  }
}
