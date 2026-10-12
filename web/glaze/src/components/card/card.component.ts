import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

/** Which surface a card sits on: flat on the page's card level, raised above it, or sunken. */
export type GlzSurface = 'card' | 'raised' | 'sunken';

/**
 * A surface that groups related content: an optional header (`[glzCardHeader]`), the body,
 * and an optional footer of actions (`[glzCardFooter]`). Its look comes from the surface
 * tokens (`--glz-surface-card`, `--glz-surface-raised`, `--glz-surface-sunken`).
 *
 * ```html
 * <glz-card surface="raised">
 *   <h3 glzCardHeader>Usage</h3>
 *   <p>1.2M tokens this week.</p>
 *   <div glzCardFooter><button glzButton>Details</button></div>
 * </glz-card>
 * ```
 */
@Component({
  selector: 'glz-card',
  templateUrl: './card.component.html',
  styleUrl: './card.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[class]': 'surfaceClass()' },
})
export class GlzCardComponent {
  readonly surface = input<GlzSurface>('card');

  protected readonly surfaceClass = computed(() => `glz-surface-${this.surface()}`);
}
