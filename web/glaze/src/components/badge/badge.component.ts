import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

/** What a badge signals. */
export type GlzTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger';

/**
 * A short label or count beside something: a status, a number waiting. Its text is read as
 * is, so say what it counts when the number alone would not (`2 approvals`), visually hidden
 * if need be.
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
  host: { '[class]': 'toneClass()' },
})
export class GlzBadgeComponent {
  readonly tone = input<GlzTone>('neutral');

  protected readonly toneClass = computed(() => `glz-tone-${this.tone()}`);
}
