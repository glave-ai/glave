import { DOCUMENT, NgTemplateOutlet } from '@angular/common';
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  booleanAttribute,
  computed,
  effect,
  inject,
  input,
  model,
  signal,
  untracked,
} from '@angular/core';
import { ChevronRight } from 'lucide';

import { durationOf } from '../../lib/motion';
import { testIds } from '../../lib/test-ids';
import { GlzIconComponent } from '../icon/icon.component';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = {
  line: 'line',
  label: 'label',
  headline: 'headline',
  chevron: 'chevron',
  text: 'text',
} as const;

/** How long a headline shows, at least, before the next takes its place while the model thinks. */
export const GLZ_THOUGHT_EVERY_MS = 1500;

/**
 * The model's reasoning in one quiet, muted line: `label` ("Thinking" while it thinks, which
 * shimmers while `live`; "Thought for 4s" once it is done), then its `headline`, the model's own
 * words, cut with an ellipsis when long. While `live`, a new headline takes the last one's place
 * at most every 1.5 s, the last fading out over the standard duration; at once when the person
 * asks for less motion, and once the reasoning is done. Done, the line ends in a chevron and
 * folds the reasoning in full (`text`), as a run's steps do: a button (`aria-expanded`) that
 * shows it under the line, behind the same rule, muted and smaller, and hides it again; it
 * starts shut (`[(open)]`).
 *
 * ```html
 * <glz-thought [label]="label()" [headline]="headline()" [live]="thinking()" [text]="text()" />
 * ```
 */
@Component({
  selector: 'glz-thought',
  imports: [GlzIconComponent, NgTemplateOutlet],
  templateUrl: './thought.component.html',
  styleUrl: './thought.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.data-testid]': 'testId()' },
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
  /** The reasoning in full, as the model wrote it, its lines kept. */
  readonly text = input('');
  /** Whether the reasoning shows in full under the line, once it is done. */
  readonly open = model(false);

  protected readonly chevron = ChevronRight;

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

  protected toggle(): void {
    this.open.update((open) => !open);
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
