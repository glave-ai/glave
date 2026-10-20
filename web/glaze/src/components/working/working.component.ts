import { LiveAnnouncer } from '@angular/cdk/a11y';
import {
  ChangeDetectionStrategy,
  Component,
  afterNextRender,
  computed,
  inject,
  input,
} from '@angular/core';

import { testIds } from '../../lib/test-ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { label: 'label' } as const;

/**
 * That glave is working, in the place its reply will take: a mark, then one quiet line
 * ("Thinking…") that shimmers, both still when the person asks for less motion. Screen readers
 * hear the line once, politely, when it appears. A mark of the app's own replaces the dot:
 *
 * ```html
 * <glz-working [label]="thinking">
 *   <svg glzWorkingMark>…</svg>
 * </glz-working>
 * ```
 */
@Component({
  selector: 'glz-working',
  templateUrl: './working.component.html',
  styleUrl: './working.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.data-testid]': 'testId()' },
})
export class GlzWorkingComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** What glave is doing, in a few words. */
  readonly label = input('Thinking…');

  constructor() {
    const announcer = inject(LiveAnnouncer);
    afterNextRender(() => {
      void announcer.announce(this.label(), 'polite');
    });
  }
}
