import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

/** What a badge signals. */
export type GlzTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger';

/**
 * A short label or count beside something, as a pill in its tone: a status, a number waiting.
 * Its text is read as is, so say what it counts when the number alone would not
 * (`2 approvals`), visually hidden if need be.
 *
 * ```html
 * <glz-badge tone="warning">2 <span class="glz-visually-hidden">approvals waiting</span></glz-badge>
 * ```
 */
@Component({
  selector: 'glz-badge',
  templateUrl: './badge.component.html',
  styleUrl: './badge.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[class]': 'classes()' },
})
export class GlzBadgeComponent {
  readonly tone = input<GlzTone>('neutral');

  protected readonly classes = computed(() => `glz-tone-${this.tone()}`);
}
