import { ChangeDetectionStrategy, Component, computed, input, linkedSignal } from '@angular/core';
import {
  GlzMarkdownComponent,
  GlzMessageComponent,
  GlzReasoningComponent,
  GlzStepsComponent,
  GlzWorkingComponent,
} from '@glave/glaze';

import type { TurnResults } from '../../lib/results';
import { ThreadItemComponent } from '../thread-item/thread-item.component';
import { TurnChangesComponent } from '../turn-changes/turn-changes.component';

const COPY = {
  reply: 'Reply',
  thinking: 'Thinking…',
  reasoning: 'Reasoning',
} as const;

/**
 * One turn, results first: the person's message; the reply (streaming in as markdown while it
 * is written); what changed; the receipt, a sunken line with how it went, how long it took and
 * what it cost, which opens to the steps; and last, what waits for the person. A turn that did
 * no work is its messages in order.
 *
 * While the turn waits on its model with nothing to show, "Thinking…" stands in the reply's
 * place. The model's reasoning, as it streams, shows open under it, and folds away once the
 * answer begins; the person can open or close it meanwhile.
 */
@Component({
  selector: 'app-turn',
  imports: [
    GlzMarkdownComponent,
    GlzMessageComponent,
    GlzReasoningComponent,
    GlzStepsComponent,
    GlzWorkingComponent,
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
  /** Whether the model is still thinking: its reasoning streams, and no answer yet. */
  protected readonly thinking = computed(() => this.turn().live?.text === '');
  /** Open while the model thinks, folded once it answers; the person may change it meanwhile. */
  protected readonly reasoningOpen = linkedSignal(() => this.thinking());
}
