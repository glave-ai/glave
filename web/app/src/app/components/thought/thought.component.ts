import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  booleanAttribute,
  computed,
  inject,
  input,
} from '@angular/core';
import { GlzThoughtComponent, formatDuration } from '@glave/glaze';

import type { Thought } from '../../lib/conversation';

/**
 * The model's reasoning in one line, from what the server says it came to: "Thinking" and its
 * headline while it streams (`live`), "Thought for 4s" and its headline once it is done, which
 * opens the reasoning in full (`text`) under the line.
 */
@Component({
  selector: 'app-thought',
  imports: [GlzThoughtComponent],
  templateUrl: './thought.component.html',
  styleUrl: './thought.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThoughtComponent {
  readonly thought = input.required<Thought>();
  /** The reasoning in full, as the model wrote it. */
  readonly text = input.required<string>();
  /** Whether the reasoning is being written now. */
  readonly live = input(false, { transform: booleanAttribute });

  private readonly locale = inject(LOCALE_ID);
  /** Whether the model thinks still: live, and the server has not said how long it took. */
  protected readonly thinking = computed(() => this.live() && this.thought().ms === null);
  /** What the model does or did, for the reader: how long it took when the server said. */
  protected readonly label = computed(() => {
    const ms = this.thought().ms;
    if (ms !== null) {
      const took = formatDuration(ms, this.locale);
      return $localize`:@@thought.took:Thought for ${took}:DURATION:`;
    }
    return this.thinking()
      ? $localize`:@@thought.thinking:Thinking`
      : $localize`:@@thought.thought:Thought`;
  });
}
