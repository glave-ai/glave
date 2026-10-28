import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { GlzStepsComponent } from '@glave/glaze';

import type { TurnPart } from '../../lib/runs';
import { ReplySourcesComponent } from '../reply-sources/reply-sources.component';
import { ThreadItemComponent } from '../thread-item/thread-item.component';

/**
 * One part of a turn, in its place: a run of calls as the server's line, which opens to its
 * calls, the line's room kept, unseen, until the server has told it; or anything else as its
 * item, a reply with the pages it drew on from the web under it.
 */
@Component({
  selector: 'app-turn-part',
  imports: [GlzStepsComponent, ReplySourcesComponent, ThreadItemComponent],
  templateUrl: './turn-part.component.html',
  styleUrl: './turn-part.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TurnPartComponent {
  readonly part = input.required<TurnPart>();

  protected readonly copy = {
    /** What names a run for screen readers until the server has told it. */
    steps: $localize`:@@turnPart.steps:Steps`,
  } as const;
  /** A run's line: the server's words, else a stand-in only screen readers read, and which. */
  protected readonly line = computed(() => {
    const part = this.part();
    const sentence = part.kind === 'run' ? part.sentence : null;
    return { words: sentence ?? this.copy.steps, pending: sentence === null };
  });
}
