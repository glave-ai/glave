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
import { ThoughtService } from '../../services/thought.service';

/**
 * The model's reasoning in one line, from what the server says it came to: "Thinking" and its
 * headline while it streams (`live`), "Thought for 4s" and its headline once it is done. The
 * line shows the reasoning in full in the thread's panel, as `key`, and says when it does.
 */
@Component({
  selector: 'app-thought',
  imports: [GlzThoughtComponent],
  templateUrl: './thought.component.html',
  styleUrl: './thought.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThoughtComponent {
  /** What the panel knows the reasoning by: its message's key, or the live one's. */
  readonly key = input.required<string>();
  readonly thought = input.required<Thought>();
  /** Whether the reasoning is being written now. */
  readonly live = input(false, { transform: booleanAttribute });

  private readonly thoughts = inject(ThoughtService);
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
  protected readonly selected = computed(() => this.thoughts.selected() === this.key());

  protected choose(): void {
    this.thoughts.choose(this.key());
  }
}
