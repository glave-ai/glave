import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
} from '@angular/core';

import { initials } from '../../lib/format';
import { testIds } from '../../lib/test-ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { hope: 'hope' } as const;

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
    '[attr.data-testid]': 'testId()',
    role: 'img',
    '[attr.aria-label]': 'name()',
    '[class]': 'sizeClass()',
  },
})
export class GlzAvatarComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  readonly name = input.required<string>();
  /** A picture; without one the initials show. */
  readonly src = input<string | null>(null);
  readonly size = input<GlzAvatarSize>('md');
  /** Shows Hope's presence dot. */
  readonly hope = input(false, { transform: booleanAttribute });

  protected readonly initials = computed(() => initials(this.name()));
  protected readonly sizeClass = computed(() => `glz-avatar-${this.size()}`);
}
