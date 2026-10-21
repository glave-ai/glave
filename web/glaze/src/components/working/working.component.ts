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
import { glzText } from '../../lib/words';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { label: 'label' } as const;

/**
 * That work is going on, in the place a reply will take: a mark, then one quiet line
 * ("Working…") that shimmers, both still when the person asks for less motion. Screen readers
 * hear the line once, politely, when it appears. A mark of the app's own replaces the dot:
 *
 * ```html
 * <glz-working [label]="label">
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
  private readonly words = glzText('working', {
    label: 'Working…',
  });

  /** What is being done, in a few words. */
  readonly label = input(this.words.label);

  constructor() {
    const announcer = inject(LiveAnnouncer);
    afterNextRender(() => {
      void announcer.announce(this.label(), 'polite');
    });
  }
}
