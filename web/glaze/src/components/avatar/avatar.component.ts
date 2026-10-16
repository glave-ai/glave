import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { initials } from '../../lib/format';

/** An avatar's size. */
export type GlzAvatarSize = 'sm' | 'md' | 'lg';

/**
 * Someone, or something, shown as a picture or initials. Named for screen readers by `name`.
 *
 * ```html
 * <glz-avatar name="Ada Lovelace" />
 * ```
 */
@Component({
  selector: 'glz-avatar',
  templateUrl: './avatar.component.html',
  styleUrl: './avatar.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[attr.data-testid]': 'testId()',
    role: 'img',
    '[attr.aria-label]': 'name()',
    '[class]': 'sizeClass()',
  },
})
export class GlzAvatarComponent {
  /** The test hook for this component. */
  readonly testId = input<string | null>(null);

  readonly name = input.required<string>();
  /** A picture; without one the initials show. */
  readonly src = input<string | null>(null);
  readonly size = input<GlzAvatarSize>('md');

  protected readonly initials = computed(() => initials(this.name()));
  protected readonly sizeClass = computed(() => `glz-avatar-${this.size()}`);
}
