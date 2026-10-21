import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
  output,
} from '@angular/core';

import { injectId } from '../../tokens/ids';
import { glzText } from '../../lib/words';
import { GlzButtonComponent } from '../button/button.component';
import { testIds } from '../../lib/test-ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = {
  caution: 'caution',
  reason: 'reason',
  decided: 'decided',
  deny: 'deny',
  allowThread: 'allow-thread',
  allowOnce: 'allow-once',
} as const;

/** The person's answer to an approval. */
export type GlzDecision = 'allow_once' | 'allow_for_thread' | 'deny';

/** The words for each decision, as the card says what was decided. */
export interface GlzDecisionLabels {
  readonly allow_once: string;
  readonly allow_for_thread: string;
  readonly deny: string;
}

/**
 * An action waiting for the person's yes, as a card that needs them: a small label with a dot
 * ("Needs you"), one plain sentence (`summary`) with what would run after it in monospace
 * (`detail`: the command, the file, the address), why it asks when more than the mode does
 * (`caution`, its own sentences in the warning tone, never inside the detail), the agent's own
 * words for why it wants this when it gave them (`reason`, quoted after `reasonLabel`, so they
 * never read as the app's), and the answers, the primary first. `onlyOnce` leaves out "for this
 * thread", for an action each of which asks. On a narrow card the answers go full width, one
 * above another. Once `decision` is set, the card is quiet and says what was decided in place
 * of the buttons.
 *
 * ```html
 * <glz-approval-card summary="Run this command?" detail="rm -rf ~/notes"
 *   caution="It deletes /home/ana/notes, outside the project."
 *   [decision]="decided()" (decide)="answer($event)" />
 * ```
 */
@Component({
  selector: 'glz-approval-card',
  imports: [GlzButtonComponent],
  templateUrl: './approval-card.component.html',
  styleUrl: './approval-card.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[attr.data-testid]': 'testId()',
    '[class.glz-approval-card--decided]': 'decided()',
    role: 'group',
    '[attr.aria-labelledby]': 'labelledBy()',
    '[attr.aria-describedby]': 'describedBy()',
    '[attr.aria-busy]': 'ariaBusy()',
  },
})
export class GlzApprovalCardComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));
  private readonly words = glzText('approvalCard', {
    label: 'Needs you',
    reasonLabel: 'The model says',
    allowOnceLabel: 'Allow once',
    allowForThreadLabel: 'Allow for this thread',
    denyLabel: 'Deny',
    decidedLabels: {
      allow_once: 'Allowed once',
      allow_for_thread: 'Allowed for this thread',
      deny: 'Denied',
    },
  });

  /** What it asks, in one plain sentence. */
  readonly summary = input.required<string>();
  /** What would run, shown in monospace: the command, the file, the address. */
  readonly detail = input('');
  /** Why it asks whatever the mode, in plain sentences, shown apart from what would run. */
  readonly caution = input('');
  /** The agent's own words for why it wants this, quoted as its words. */
  readonly reason = input('');
  /** Who the reason's words are, before the quote. */
  readonly reasonLabel = input(this.words.reasonLabel);
  /** Whether it can only be allowed once: no "for this thread", since each one asks. */
  readonly onlyOnce = input(false, { transform: booleanAttribute });
  /** The answer given, once there is one. */
  readonly decision = input<GlzDecision | null>(null);
  /** Whether the answer is being sent, so the buttons wait. */
  readonly busy = input(false, { transform: booleanAttribute });
  /** The label over a card that waits. */
  readonly label = input(this.words.label);
  readonly allowOnceLabel = input(this.words.allowOnceLabel);
  readonly allowForThreadLabel = input(this.words.allowForThreadLabel);
  readonly denyLabel = input(this.words.denyLabel);
  readonly decidedLabels = input<GlzDecisionLabels>(this.words.decidedLabels);
  /** The person's answer. */
  readonly decide = output<GlzDecision>();

  protected readonly labelId = injectId('glz-approval-label');
  protected readonly summaryId = injectId('glz-approval-summary');
  protected readonly cautionId = injectId('glz-approval-caution');
  protected readonly reasonId = injectId('glz-approval-reason');
  protected readonly ariaBusy = computed(() => (this.busy() ? 'true' : null));
  protected readonly decidedText = computed(() => {
    const decision = this.decision();
    return decision === null ? null : this.decidedLabels()[decision];
  });
  protected readonly decided = computed(() => this.decision() !== null);
  protected readonly hasDetail = computed(() => this.detail() !== '');
  protected readonly hasCaution = computed(() => this.caution() !== '');
  protected readonly hasReason = computed(() => this.reason() !== '');
  protected readonly offersThread = computed(() => !this.onlyOnce());
  /** Named by its label and sentence while it waits, by its sentence once decided. */
  protected readonly labelledBy = computed(() =>
    this.decided() ? this.summaryId : `${this.labelId} ${this.summaryId}`,
  );
  /** Described by why it asks, when a caution says, and by the agent's reason, when it gave one. */
  protected readonly describedBy = computed(() => {
    const ids = [
      ...(this.hasCaution() ? [this.cautionId] : []),
      ...(this.hasReason() ? [this.reasonId] : []),
    ];
    return ids.length === 0 ? null : ids.join(' ');
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
