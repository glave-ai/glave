import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
  output,
} from '@angular/core';
import { ShieldQuestion } from 'lucide';

import { injectId } from '../../lib/ids';
import { GlzButtonComponent } from '../button/button.component';
import { GlzIconComponent } from '../icon/icon.component';

/** The person's answer to an approval. */
export type GlzDecision = 'allow_once' | 'allow_for_thread' | 'deny';

/** The words for each decision, as the card says what was decided. */
export interface GlzDecisionLabels {
  readonly allow_once: string;
  readonly allow_for_thread: string;
  readonly deny: string;
}

const DECIDED: GlzDecisionLabels = {
  allow_once: 'Allowed once',
  allow_for_thread: 'Allowed for this thread',
  deny: 'Denied',
};

/**
 * An action waiting for the person's yes: what wants to run, the detail to judge it by
 * (projected: the command, the file, the address), and the three answers. Once `decision` is
 * set, the card says what was decided in place of the buttons.
 *
 * ```html
 * <glz-approval-card tool="bash" summary="Run rm -rf build/" [decision]="decided()"
 *   (decide)="answer($event)"><pre>rm -rf build/</pre></glz-approval-card>
 * ```
 */
@Component({
  selector: 'glz-approval-card',
  imports: [GlzButtonComponent, GlzIconComponent],
  templateUrl: './approval-card.component.html',
  styleUrl: './approval-card.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    role: 'group',
    '[attr.aria-labelledby]': 'headingId',
    '[attr.aria-busy]': 'ariaBusy()',
  },
})
export class GlzApprovalCardComponent {
  /** What wants to run, in a line. */
  readonly summary = input.required<string>();
  readonly tool = input('');
  /** The answer given, once there is one. */
  readonly decision = input<GlzDecision | null>(null);
  /** Whether the answer is being sent, so the buttons wait. */
  readonly busy = input(false, { transform: booleanAttribute });
  readonly heading = input('Approval needed');
  readonly allowOnceLabel = input('Allow once');
  readonly allowForThreadLabel = input('Allow for this thread');
  readonly denyLabel = input('Deny');
  readonly decidedLabels = input<GlzDecisionLabels>(DECIDED);
  /** The person's answer. */
  readonly decide = output<GlzDecision>();

  protected readonly headingId = injectId('glz-approval');
  protected readonly icon = ShieldQuestion;
  protected readonly ariaBusy = computed(() => (this.busy() ? 'true' : null));
  protected readonly decidedText = computed(() => {
    const decision = this.decision();
    return decision === null ? null : this.decidedLabels()[decision];
  });

  protected allowOnce(): void {
    this.decide.emit('allow_once');
  }

  protected allowForThread(): void {
    this.decide.emit('allow_for_thread');
  }

  protected deny(): void {
    this.decide.emit('deny');
  }
}
