import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  inject,
  input,
} from '@angular/core';

import { injectId } from '../../tokens/ids';
import { GlzRadioGroupComponent } from '../radio-group/radio-group.component';

/**
 * One choice in a `<glz-radio-group>`: a native radio input with its label, and a `hint` under
 * it that says more, which screen readers hear after the name. In a group of `cards`, the
 * choice is a card.
 *
 * ```html
 * <glz-radio value="auto" hint="Asks only before risky actions">Auto</glz-radio>
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
  /** A line under the label that says more about the choice. */
  readonly hint = input('');

  protected readonly group = inject(GlzRadioGroupComponent);
  protected readonly checked = computed(() => this.group.value() === this.value());
  protected readonly isDisabled = computed(() => this.disabled() || this.group.isDisabled());
  protected readonly card = this.group.cards;
  protected readonly labelId = injectId('glz-radio-label');
  protected readonly hintId = injectId('glz-radio-hint');
  protected readonly hasHint = computed(() => this.hint() !== '');
  /** With a hint, the label alone names the radio and the hint describes it. */
  protected readonly labelledBy = computed(() => (this.hasHint() ? this.labelId : null));
  protected readonly describedBy = computed(() => (this.hasHint() ? this.hintId : null));

  protected pick(): void {
    this.group.pick(this.value());
  }

  protected blurred(): void {
    this.group.markTouched();
  }
}
