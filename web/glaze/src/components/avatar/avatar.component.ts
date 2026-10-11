import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
} from '@angular/core';

import { initials } from '../../lib/format';

/** An avatar's size. */
export type GlzAvatarSize = 'sm' | 'md' | 'lg';

/**
 * Someone, or something, shown as a picture or initials. `hope` adds Hope's presence dot, in
 * sun gold: the one place that colour appears. Named for screen readers by `name`.
 *
 * ```html
 * <glz-avatar name="Ada Lovelace" />
 * <glz-avatar name="Hope" hope />
 * ```
 */
@Component({
  selector: 'glz-avatar',
  templateUrl: './avatar.component.html',
  styleUrl: './avatar.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    role: 'img',
    '[attr.aria-label]': 'name()',
    '[class]': 'sizeClass()',
  },
})
export class GlzAvatarComponent {
  readonly name = input.required<string>();
  /** A picture; without one the initials show. */
  readonly src = input<string | null>(null);
  readonly size = input<GlzAvatarSize>('md');
  /** Shows Hope's presence dot. */
  readonly hope = input(false, { transform: booleanAttribute });

  protected readonly initials = computed(() => initials(this.name()));
  protected readonly sizeClass = computed(() => `glz-avatar-${this.size()}`);
}
