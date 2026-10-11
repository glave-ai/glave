import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

/**
 * A progress bar: how far along something is, from 0 to 100, or, with no `value`, that it is
 * under way. `label` names it for screen readers.
 *
 * ```html
 * <glz-progress label="Budget used" [value]="62" />
 * <glz-progress label="Uploading" />
 * ```
 */
@Component({
  selector: 'glz-progress',
  templateUrl: './progress.component.html',
  styleUrl: './progress.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    role: 'progressbar',
    'aria-valuemin': '0',
    'aria-valuemax': '100',
    '[attr.aria-label]': 'label()',
    '[attr.aria-valuenow]': 'clamped()',
    '[class.indeterminate]': 'indeterminate()',
    '[style.--glz-progress-value]': 'width()',
  },
})
export class GlzProgressComponent {
  readonly label = input.required<string>();
  /** Percent done; `null` while that is unknown. */
  readonly value = input<number | null>(null);

  protected readonly clamped = computed(() => {
    const value = this.value();
    return value === null ? null : Math.min(100, Math.max(0, value));
  });
  protected readonly indeterminate = computed(() => this.clamped() === null);
  protected readonly width = computed(() => `${String(this.clamped() ?? 30)}%`);
}
