import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  effect,
  input,
  output,
  signal,
} from '@angular/core';

import { injectId } from '../../tokens/ids';
import { type GlzQuestion, composeAnswer, replaceAt, togglePick } from '../../lib/questions';
import { GlzButtonComponent } from '../button/button.component';
import { GlzCheckboxComponent } from '../checkbox/checkbox.component';
import { GlzRadioComponent } from '../radio/radio.component';
import { GlzRadioGroupComponent } from '../radio-group/radio-group.component';
import { testIds } from '../../lib/test-ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { choice: 'choice', own: 'own', submit: 'submit', answer: 'answer' } as const;

/** A choice as the card shows it. */
interface ShownChoice {
  readonly label: string;
  readonly description: string;
  readonly checked: boolean;
}

/** A question as the card shows it. */
interface ShownQuestion {
  readonly index: number;
  readonly id: string;
  readonly text: string;
  readonly choices: readonly ShownChoice[];
  readonly hasChoices: boolean;
  readonly multiple: boolean;
  readonly picked: string | null;
  readonly ownWords: string;
  readonly answer: string;
}

/**
 * Questions the model asks the person, as a card that needs them: a small label with a dot
 * ("Needs you"), then each question in a plain sentence with its choices (one, or several when
 * `multiple`) and room for their own words, then the answer button. `answer` carries one
 * string per question, in order. Once `answered` is set, the card is quiet and shows the
 * answers instead.
 *
 * ```html
 * <glz-question-card [questions]="asked()" [answered]="answers()" (answer)="reply($event)" />
 * ```
 */
@Component({
  selector: 'glz-question-card',
  imports: [GlzButtonComponent, GlzCheckboxComponent, GlzRadioGroupComponent, GlzRadioComponent],
  templateUrl: './question-card.component.html',
  styleUrl: './question-card.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[attr.data-testid]': 'testId()',
    '[class.glz-question-card--answered]': 'isAnswered()',
    role: 'group',
    '[attr.aria-labelledby]': 'labelledBy()',
  },
})
export class GlzQuestionCardComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  readonly questions = input.required<readonly GlzQuestion[]>();
  /** The answers given, once there are some. */
  readonly answered = input<readonly string[] | null>(null);
  /** Whether the answers are being sent, so the button waits. */
  readonly busy = input(false, { transform: booleanAttribute });
  /** The label over a card that waits. */
  readonly label = input('Needs you');
  readonly ownWordsLabel = input('In your own words');
  readonly submitLabel = input('Answer');
  /** One answer for each question, in order. */
  readonly answer = output<readonly string[]>();

  protected readonly labelId = injectId('glz-question');
  private readonly idBase = injectId('glz-question-item');
  private readonly picks = signal<readonly (readonly string[])[]>([]);
  private readonly ownWords = signal<readonly string[]>([]);

  protected readonly shown = computed<readonly ShownQuestion[]>(() =>
    this.questions().map((question, index) => {
      const picked = this.picks()[index] ?? [];
      const own = this.ownWords()[index] ?? '';
      const choices = (question.options ?? []).map((choice) => ({
        label: choice.label,
        description: choice.description ?? '',
        checked: picked.includes(choice.label),
      }));
      return {
        index,
        id: `${this.idBase}-${String(index)}`,
        text: question.question,
        choices,
        hasChoices: choices.length > 0,
        multiple: question.multiple === true,
        picked: picked[0] ?? null,
        ownWords: own,
        answer: this.answered()?.[index] ?? '',
      };
    }),
  );
  protected readonly answers = computed(() =>
    this.questions().map((_, index) =>
      composeAnswer(this.picks()[index] ?? [], this.ownWords()[index] ?? ''),
    ),
  );
  protected readonly isAnswered = computed(() => this.answered() !== null);
  /** Named by its label while it waits, by its first question once answered. */
  protected readonly labelledBy = computed(() =>
    this.isAnswered() ? (this.shown()[0]?.id ?? null) : this.labelId,
  );
  protected readonly cannotSubmit = computed(
    () => this.busy() || this.answers().some((answer) => answer === ''),
  );

  constructor() {
    // New questions start with nothing picked.
    effect(() => {
      const count = this.questions().length;
      this.picks.set(Array.from({ length: count }, () => []));
      this.ownWords.set(Array.from({ length: count }, () => ''));
    });
  }

  protected pickOne(index: number, label: string | null): void {
    this.picks.update((picks) => replaceAt(picks, index, label === null ? [] : [label]));
  }

  protected pickMany(index: number, label: string, on: boolean): void {
    this.picks.update((picks) =>
      replaceAt(picks, index, togglePick(picks[index] ?? [], label, on)),
    );
  }

  protected typed(index: number, event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      const text = event.target.value;
      this.ownWords.update((words) => replaceAt(words, index, text));
    }
  }

  protected submit(): void {
    if (!this.cannotSubmit()) {
      this.answer.emit(this.answers());
    }
  }
}
