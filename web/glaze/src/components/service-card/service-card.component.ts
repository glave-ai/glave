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

import { testIds } from '../../lib/test-ids';
import {
  type GlzServiceField,
  type GlzServiceFieldKind,
  type GlzServiceValue,
  type GlzServiceValues,
  answerOf,
  sameFields,
  startingValues,
} from '../../lib/service-fields';
import { togglePick } from '../../lib/questions';
import { glzText } from '../../lib/words';
import { injectId } from '../../tokens/ids';
import { GlzButtonComponent } from '../button/button.component';
import { GlzCheckboxComponent } from '../checkbox/checkbox.component';
import { GlzRadioComponent } from '../radio/radio.component';
import { GlzRadioGroupComponent } from '../radio-group/radio-group.component';
import {
  type GlzServiceDetail,
  GlzServiceDetailComponent,
} from '../service-detail/service-detail.component';

/** The values of a yes or no's two radios. */
const YES = 'yes';
const NO = 'no';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = {
  who: 'who',
  message: 'message',
  field: 'field',
  choice: 'choice',
  detail: 'detail',
  note: 'note',
  missing: 'missing',
  required: 'required',
  send: 'send',
  decline: 'decline',
  told: 'told',
} as const;

/** What a field of `kind` is drawn as. */
function controlOf(kind: GlzServiceFieldKind): ShownField['control'] {
  switch (kind.type) {
    case 'choice':
      return kind.multiple === true ? 'many' : 'one';
    case 'yes_no':
      return 'yes_no';
    default:
      return 'typed';
  }
}

/** A field as the card shows it. */
interface ShownField {
  readonly name: string;
  readonly id: string;
  /** What its input takes, where it has one. */
  readonly inputType: 'text' | 'number';
  /** Whether an answer needs it. */
  readonly required: boolean;
  readonly label: string;
  readonly description: string;
  readonly control: 'typed' | 'yes_no' | 'one' | 'many';
  readonly step: string;
  /** What is typed in it. */
  readonly text: string;
  /** What is picked of one choice or a yes or no; `null` for none. */
  readonly picked: string | null;
  readonly options: readonly {
    readonly value: string;
    readonly label: string;
    readonly checked: boolean;
  }[];
}

/**
 * What a connected service asks the person in the middle of a call, as a card that needs them:
 * the "Needs you" label with a dot, who asks in the server's sentence (`who`), so a tool can't
 * pass as the model, what it asks in the service's own words (`message`), only the fields it
 * asks for, what it adds when it says (`detail`, folded shut), where the answer goes (`note`),
 * then send and decline. `answer` carries each field's
 * value by its name; `decline` that they said no. Once `told` is set, the card rests as that
 * one line.
 *
 * ```html
 * <glz-service-card
 *   who="GitHub asks you, not the model"
 *   message="Which repository?"
 *   sendLabel="Send to GitHub"
 *   note="Your answer goes only to GitHub."
 *   [fields]="fields()"
 *   [told]="told()"
 *   (answer)="reply($event)"
 *   (decline)="refuse()"
 * />
 * ```
 */
@Component({
  selector: 'glz-service-card',
  imports: [
    GlzButtonComponent,
    GlzCheckboxComponent,
    GlzRadioGroupComponent,
    GlzRadioComponent,
    GlzServiceDetailComponent,
  ],
  templateUrl: './service-card.component.html',
  styleUrl: './service-card.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[attr.data-testid]': 'testId()',
    '[class.glz-service-card--answered]': 'isTold()',
    role: 'group',
    '[attr.aria-labelledby]': 'labelledBy()',
  },
})
export class GlzServiceCardComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));
  private readonly words = glzText('serviceCard', {
    label: 'Needs you',
    declineLabel: 'Decline',
    yesLabel: 'Yes',
    noLabel: 'No',
    requiredLabel: 'Required',
    missingLabel: 'Answer what’s required, and numbers as numbers, to send.',
  });

  /** Who asks, in the server's sentence. */
  readonly who = input.required<string>();
  /** What it asks, in the service's own words. */
  readonly message = input.required<string>();
  readonly fields = input.required<readonly GlzServiceField[]>();
  /** What it adds, for the person to read first, folded shut under the fields; `null` for none. */
  readonly detail = input<GlzServiceDetail | null>(null);
  /** What the send button says, naming who it goes to. */
  readonly sendLabel = input.required<string>();
  /** Where the answer goes. */
  readonly note = input.required<string>();
  /** What came of it, in the server's line, once it was answered. */
  readonly told = input<string | null>(null);
  /** Whether the answer is being sent, so the buttons wait. */
  readonly busy = input(false, { transform: booleanAttribute });
  /** The label over a card that waits. */
  readonly label = input(this.words.label);
  readonly declineLabel = input(this.words.declineLabel);
  readonly yesLabel = input(this.words.yesLabel);
  readonly noLabel = input(this.words.noLabel);
  /** What marks a field an answer needs. */
  readonly requiredLabel = input(this.words.requiredLabel);
  /** Why it cannot be sent yet. */
  readonly missingLabel = input(this.words.missingLabel);
  /** Each field's value, by its name. */
  readonly answer = output<Readonly<Record<string, unknown>>>();
  readonly decline = output();

  protected readonly yes = YES;
  protected readonly no = NO;
  protected readonly labelId = injectId('glz-service');
  private readonly idBase = injectId('glz-service-field');
  private readonly values = signal<GlzServiceValues>({});

  protected readonly isTold = computed(() => this.told() !== null);
  protected readonly labelledBy = computed(() => (this.isTold() ? null : this.labelId));
  protected readonly shown = computed<readonly ShownField[]>(() =>
    this.fields().map((field, index) => this.shownField(field, index)),
  );
  private readonly answering = computed(() => answerOf(this.fields(), this.values()));
  protected readonly incomplete = computed(() => this.answering() === null);
  protected readonly cannotSend = computed(() => this.busy() || this.incomplete());

  /** What the service asks: a new array of the same fields asks nothing new. */
  private readonly asked = computed(() => this.fields(), { equal: sameFields });

  constructor() {
    // New fields start as they say; the same ones again keep what was picked and typed.
    effect(() => {
      this.values.set(startingValues(this.asked()));
    });
  }

  private shownField(field: GlzServiceField, index: number): ShownField {
    const value = this.values()[field.name];
    const kind = field.kind;
    const picked: readonly unknown[] = Array.isArray(value) ? value : [];
    const id = `${this.idBase}-${String(index)}`;
    const text = this.textOf(value);
    return {
      name: field.name,
      id,
      inputType: kind.type === 'number' ? 'number' : 'text',
      required: field.required === true,
      label: field.label,
      description: field.description ?? '',
      control: controlOf(kind),
      step: kind.type === 'number' && kind.integer === true ? '1' : 'any',
      text: text ?? '',
      picked: text === '' ? null : text,
      options:
        kind.type === 'choice'
          ? kind.options.map((option) => ({ ...option, checked: picked.includes(option.value) }))
          : [],
    };
  }

  /** What a field's value shows as: `yes` or `no` of a yes or no. */
  private textOf(value: GlzServiceValue | undefined): string | null {
    if (typeof value === 'boolean') {
      return value ? YES : NO;
    }
    return typeof value === 'string' ? value : null;
  }

  private set(name: string, value: GlzServiceValue): void {
    this.values.update((values) => ({ ...values, [name]: value }));
  }

  protected typed(name: string, event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.set(name, event.target.value);
    }
  }

  protected pickOne(name: string, value: string | null): void {
    this.set(name, value ?? '');
  }

  protected pickYesNo(name: string, value: string | null): void {
    this.set(name, value === YES);
  }

  protected pickMany(name: string, value: string, on: boolean): void {
    const picked = this.values()[name];
    this.set(
      name,
      togglePick(Array.isArray(picked) ? (picked as readonly string[]) : [], value, on),
    );
  }

  protected send(): void {
    const answer = this.answering();
    if (answer !== null && !this.busy()) {
      this.answer.emit(answer);
    }
  }
}
