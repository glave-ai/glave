import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

/** What a badge signals. */
export type GlzTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger';

/**
 * How a badge shows: a pill with its text, a small dot in its tone whose text only screen
 * readers read, or its text alone, muted, for a status that should stay quiet.
 */
export type GlzBadgeVariant = 'pill' | 'dot' | 'plain';

/**
 * A short label or count beside something: a status, a number waiting. Its text is read as
 * is, so say what it counts when the number alone would not (`2 approvals`), visually hidden
 * if need be. A `dot` badge always says what it means in its text, read but not shown.
 *
 * ```html
 * <glz-badge tone="warning">2 <span class="glz-visually-hidden">approvals waiting</span></glz-badge>
 * <glz-badge tone="accent" variant="dot">needs you</glz-badge>
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
  readonly variant = input<GlzBadgeVariant>('pill');

  protected readonly classes = computed(
    () => `glz-tone-${this.tone()} glz-badge--${this.variant()}`,
  );
  protected readonly hidden = computed(() => this.variant() === 'dot');
}
