import { ChangeDetectionStrategy, Component, computed, input, model } from '@angular/core';

import type { GlzOption } from '../../lib/options';
import { testIds } from '../../lib/test-ids';
import { injectId } from '../../tokens/ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { label: 'label', option: 'option' } as const;

/**
 * A choice among two or three options, all in view as segments side by side, its `label` at
 * its start: the one chosen lifted onto the card's surface and drawn round in the selected
 * marker. Each segment is a native radio, so arrow keys move the choice and screen readers hear
 * a group named by the label. For a few words each, such as how a list is grouped; a choice
 * that needs more, or among more, is a menu select. `value` is bound two ways.
 *
 * ```html
 * <glz-segmented label="Group by" [options]="groupings" [(value)]="grouping" />
 * ```
 */
@Component({
  selector: 'glz-segmented',
  templateUrl: './segmented.component.html',
  styleUrl: './segmented.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.data-testid]': 'testId()' },
})
export class GlzSegmentedComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));
  /** What the choice is about, shown at its start and naming the group. */
  readonly label = input.required<string>();
  readonly options = input.required<readonly GlzOption[]>();
  /** The value of the option chosen, or `null`. */
  readonly value = model<string | null>(null);

  /** The name shared by the group's native radios. */
  protected readonly name = injectId('glz-segmented');
  protected readonly labelId = injectId('glz-segmented-label');
  protected readonly segments = computed(() =>
    this.options().map((option) => ({ ...option, checked: option.value === this.value() })),
  );

  protected pick(value: string): void {
    this.value.set(value);
  }
}
