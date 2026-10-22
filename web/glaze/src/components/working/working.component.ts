import { LiveAnnouncer } from '@angular/cdk/a11y';
import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  signal,
  untracked,
} from '@angular/core';

import { testIds } from '../../lib/test-ids';
import { glzText } from '../../lib/words';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { label: 'label', elapsed: 'elapsed' } as const;

/** How long each of the `words` shows before the next. */
const WORD_MS = 2000;

/**
 * That work is going on, in the place a reply will take: a mark, then one quiet line that
 * shimmers, and how long it has gone on. The line is the `step` being taken while there is one;
 * else the `words`, the next every couple of seconds; else the `label` ("Working…"). The mark and
 * the shimmer are still when the person asks for less motion; the words still change, since
 * they move nothing. Screen readers hear the label once, politely, when the line appears, then
 * each step as it begins, never the words. A mark of the app's own replaces the dot:
 *
 * ```html
 * <glz-working [label]="label" [words]="words" [step]="step" [elapsed]="elapsed">
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
  private readonly text = glzText('working', {
    label: 'Working…',
  });
  private readonly announcer = inject(LiveAnnouncer);

  /** What is being done, in a few words: what screen readers hear as the line appears. */
  readonly label = input(this.text.label);
  /** Words shown in the label's place, one at a time, the next every couple of seconds. */
  readonly words = input<readonly string[]>([]);
  /** The step being taken now, in a few words, shown in place of the words while it lasts. */
  readonly step = input<string | null>(null);
  /** How long the work has gone on, written for the reader ("12s"), shown after the line. */
  readonly elapsed = input<string | null>(null);

  /** How many times the words have moved on. */
  private readonly turns = signal(0);

  protected readonly shown = computed(() => {
    const words = this.words();
    const word = words.length === 0 ? undefined : words[this.turns() % words.length];
    return this.step() ?? word ?? this.label();
  });

  /** Moves the words on every couple of seconds while they show, and stops when they don't. */
  protected readonly turning = effect((onCleanup) => {
    if (this.step() !== null || this.words().length < 2) {
      return;
    }
    const timer = setInterval(() => {
      this.turns.update((turns) => turns + 1);
    }, WORD_MS);
    onCleanup(() => {
      clearInterval(timer);
    });
  });

  /** Whether screen readers have heard that the work goes on. */
  private told = false;

  /** Tells screen readers, politely: the label as the line appears, then each step it takes. */
  protected readonly telling = effect(() => {
    // A step's first line: a command can run to many.
    const step = this.step()?.split('\n')[0] ?? null;
    const said = step ?? (this.told ? null : this.label());
    this.told = true;
    if (said !== null) {
      untracked(() => void this.announcer.announce(said, 'polite'));
    }
  });
}
