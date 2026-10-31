import { ChangeDetectionStrategy, Component, computed, input, model } from '@angular/core';

import type { GlzOption } from '../../lib/options';
import { testIds } from '../../lib/test-ids';
import { injectId } from '../../tokens/ids';
import { GlzSwatchColorDirective } from '../../directives/swatch-color.directive';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { option: 'option', any: 'any' } as const;

/** What the browser's color picker opens on when no exact color is chosen yet. */
const STARTING_COLOR = '#1558d6';

/** One choice among colors: a word, or with `color`, a swatch named by its `label`. */
export interface GlzColorOption extends GlzOption {
  /** The CSS color to show. */
  readonly color?: string;
}

/**
 * A choice among colors: words and swatches in one row, each a native radio, so arrow keys move
 * the choice and screen readers hear each swatch's name, and last a many-colored swatch, the
 * browser's own color picker, for any exact color. `value` is bound two ways: the value of the
 * option chosen, or the `#rrggbb` picked.
 *
 * ```html
 * <glz-color-choice label="Accent" anyLabel="Pick your own color" [options]="accents" [(value)]="accent" />
 * ```
 */
@Component({
  selector: 'glz-color-choice',
  imports: [GlzSwatchColorDirective],
  templateUrl: './color-choice.component.html',
  styleUrl: './color-choice.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.data-testid]': 'testId()' },
})
export class GlzColorChoiceComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));
  /** What the choice is about, naming the group. */
  readonly label = input.required<string>();
  /** What the color picker is called. */
  readonly anyLabel = input.required<string>();
  readonly options = input.required<readonly GlzColorOption[]>();
  /** The value of the option chosen, or the exact color picked, or `null`. */
  readonly value = model<string | null>(null);

  /** The name shared by the group's native radios. */
  protected readonly name = injectId('glz-color-choice');
  protected readonly choices = computed(() =>
    this.options().map((option) => ({
      ...option,
      color: option.color ?? null,
      checked: option.value === this.value(),
    })),
  );
  /** Whether the color chosen is one the picker gave, not one of the options. */
  protected readonly picked = computed(() => {
    const value = this.value();
    return value !== null && !this.options().some((option) => option.value === value);
  });
  protected readonly anyValue = computed(() => {
    const value = this.value();
    return this.picked() && value !== null ? value : STARTING_COLOR;
  });

  protected pick(value: string): void {
    this.value.set(value);
  }

  protected pickAny(event: Event): void {
    this.value.set((event.target as HTMLInputElement).value);
  }
}
