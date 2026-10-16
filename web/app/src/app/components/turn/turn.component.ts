import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import {
  GlzMarkdownComponent,
  GlzMessageComponent,
  GlzReasoningComponent,
  GlzStepsComponent,
} from '@glave/glaze';

import type { TurnResults } from '../../lib/results';
import { ThreadItemComponent } from '../thread-item/thread-item.component';
import { TurnChangesComponent } from '../turn-changes/turn-changes.component';

const COPY = {
  reply: 'Reply',
} as const;

/**
 * One turn, results first: the person's message; the reply (streaming in as markdown while it
 * is written); what changed; the receipt, a sunken line with how it went, how long it took and
 * what it cost, which opens to the steps; and last, what waits for the person. A turn that did
 * no work is its messages in order.
 */
@Component({
  selector: 'app-turn',
  imports: [
    GlzMarkdownComponent,
    GlzMessageComponent,
    GlzReasoningComponent,
    GlzStepsComponent,
    ThreadItemComponent,
    TurnChangesComponent,
  ],
  templateUrl: './turn.component.html',
  styleUrl: './turn.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TurnComponent {
  readonly turn = input.required<TurnResults>();

  protected readonly copy = COPY;
  protected readonly changed = computed(() => this.turn().files.length > 0);
}
