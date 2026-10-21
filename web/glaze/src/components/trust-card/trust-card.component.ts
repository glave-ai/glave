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
  item: 'item',
  trust: 'trust',
  notNow: 'not-now',
  answered: 'answered',
} as const;

/** One thing the person is asked to trust: what it does, and the thing itself, if any. */
export interface GlzTrustItem {
  readonly text: string;
  /** What it runs or reaches, shown in monospace: a command, an address, a model. */
  readonly detail?: string | null;
}

/** The person's answer: trust it, or keep it off for now. */
export type GlzTrustAnswer = 'trust' | 'not_now';

/** The words for each answer, as the card says what was answered. */
export interface GlzTrustAnswerLabels {
  readonly trust: string;
  readonly not_now: string;
}

/**
 * Something that waits for the person's trust, as a card that needs them, as
 * `glz-approval-card` is: a small label with a dot ("Needs you"), one plain sentence
 * (`summary`), each thing it would turn on (`items`, the thing itself in monospace), then the
 * two answers, trust (the primary) first. On a narrow card the answers go full width, one
 * above another. Once `answered` is set, the card rests and says what was answered in place of
 * the buttons.
 *
 * ```html
 * <glz-trust-card summary="This project brings its own tools and settings" [items]="items()"
 *   [answered]="answered()" (answer)="trust($event)" />
 * ```
 */
@Component({
  selector: 'glz-trust-card',
  imports: [GlzButtonComponent],
  templateUrl: './trust-card.component.html',
  styleUrl: './trust-card.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[attr.data-testid]': 'testId()',
    '[class.glz-trust-card--answered]': 'isAnswered()',
    role: 'group',
    '[attr.aria-labelledby]': 'labelledBy()',
    '[attr.aria-busy]': 'ariaBusy()',
  },
})
export class GlzTrustCardComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));
  private readonly words = glzText('trustCard', {
    label: 'Needs you',
    trustLabel: 'Trust this project',
    notNowLabel: 'Not now',
    answeredLabels: { trust: 'Trusted', not_now: 'Kept off for now' },
  });

  /** What it asks, in one plain sentence. */
  readonly summary = input.required<string>();
  /** Each thing it would turn on. */
  readonly items = input.required<readonly GlzTrustItem[]>();
  /** The answer given, once there is one. */
  readonly answered = input<GlzTrustAnswer | null>(null);
  /** Whether the answer is being sent, so the buttons wait. */
  readonly busy = input(false, { transform: booleanAttribute });
  /** The label over a card that waits. */
  readonly label = input(this.words.label);
  readonly trustLabel = input(this.words.trustLabel);
  readonly notNowLabel = input(this.words.notNowLabel);
  readonly answeredLabels = input<GlzTrustAnswerLabels>(this.words.answeredLabels);
  /** The person's answer. */
  readonly answer = output<GlzTrustAnswer>();

  protected readonly labelId = injectId('glz-trust-label');
  protected readonly summaryId = injectId('glz-trust-summary');
  protected readonly ariaBusy = computed(() => (this.busy() ? 'true' : null));
  protected readonly answeredText = computed(() => {
    const answered = this.answered();
    return answered === null ? null : this.answeredLabels()[answered];
  });
  protected readonly isAnswered = computed(() => this.answered() !== null);
  /** Named by its label and sentence while it waits, by its sentence once answered. */
  protected readonly labelledBy = computed(() =>
    this.isAnswered() ? this.summaryId : `${this.labelId} ${this.summaryId}`,
  );

  protected trust(): void {
    this.answer.emit('trust');
  }

  protected notNow(): void {
    this.answer.emit('not_now');
  }
}
