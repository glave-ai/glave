import { DOCUMENT } from '@angular/common';
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  booleanAttribute,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';

import { INERT_UNTIL_BOUND } from '../../lib/inert-until-bound';
import { durationOf } from '../../lib/motion';
import { testIds } from '../../lib/test-ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { line: 'line', label: 'label', headline: 'headline' } as const;

/** How long a headline shows, at least, before the next takes its place while the model thinks. */
export const GLZ_THOUGHT_EVERY_MS = 1500;

/**
 * The model's reasoning in one quiet, muted line: `label` ("Thinking" while it thinks, which
 * shimmers while `live`; "Thought for 4s" once it is done), then its `headline`, the model's own
 * words, cut with an ellipsis when long. While `live`, a new headline takes the last one's place
 * at most every 1.5 s, the last fading out over the standard duration; at once when the person
 * asks for less motion, and once the reasoning is done. The line is a button: `choose` asks to
 * show the reasoning in full elsewhere, and `selected` says it shows there (`aria-pressed`).
 *
 * ```html
 * <glz-thought [label]="label()" [headline]="headline()" [live]="thinking()"
 *   [selected]="shown()" (choose)="show()" />
 * ```
 */
@Component({
  selector: 'glz-thought',
  templateUrl: './thought.component.html',
  styleUrl: './thought.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { ...INERT_UNTIL_BOUND, '[attr.data-testid]': 'testId()' },
})
export class GlzThoughtComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));
  private readonly document = inject(DOCUMENT);

  /** What the model does or did, in a word or a few: "Thinking", "Thought for 4s". */
  readonly label = input.required<string>();
  /** The reasoning's headline, the model's own words; `null` while it has none. */
  readonly headline = input<string | null>(null);
  /** Whether the model is thinking still: the label shimmers, and headlines change in time. */
  readonly live = input(false, { transform: booleanAttribute });
  /** Whether the reasoning shows in full elsewhere, as this line asked. */
  readonly selected = input(false, { transform: booleanAttribute });
  /** The person asks to see the reasoning in full. */
  readonly choose = output();

  /** The headline the line shows now. */
  protected readonly shown = signal<string | null>(null);
  /** Whether that headline fades out, for the next to take its place. */
  protected readonly fading = signal(false);
  /** When the shown headline last changed, in milliseconds since 1970. */
  private changed = Number.NEGATIVE_INFINITY;
  /** The next change, while one waits its turn or its fade. */
  private timer: ReturnType<typeof setTimeout> | undefined;

  constructor() {
    effect(() => {
      const headline = this.headline();
      const live = this.live();
      untracked(() => {
        this.follow(headline, live);
      });
    });
    inject(DestroyRef).onDestroy(() => {
      clearTimeout(this.timer);
    });
  }

  /**
   * Follows `headline`: at once while nothing shows or the model is done, else in its turn,
   * when the change already waiting shows the latest one.
   */
  private follow(headline: string | null, live: boolean): void {
    if (!live || this.shown() === null) {
      clearTimeout(this.timer);
      this.timer = undefined;
      this.fading.set(false);
      this.show(headline);
      return;
    }
    if (headline === this.shown() || this.timer !== undefined) {
      return;
    }
    const wait = Math.max(0, this.changed + GLZ_THOUGHT_EVERY_MS - Date.now());
    this.timer = setTimeout(() => {
      this.fade();
    }, wait);
  }

  /** Fades the shown headline out, then shows the latest in its place. */
  private fade(): void {
    this.fading.set(true);
    this.timer = setTimeout(
      () => {
        this.timer = undefined;
        this.fading.set(false);
        this.show(this.headline());
      },
      durationOf(this.document, 'standard'),
    );
  }

  private show(headline: string | null): void {
    this.shown.set(headline);
    this.changed = Date.now();
  }
}
