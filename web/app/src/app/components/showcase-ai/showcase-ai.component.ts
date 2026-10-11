import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  signal,
  viewChild,
} from '@angular/core';
import {
  GlzApprovalCardComponent,
  GlzButtonComponent,
  GlzComboboxComponent,
  type GlzComposed,
  GlzComposerComponent,
  type GlzDecision,
  type GlzMoney,
  GlzMessageComponent,
  type GlzOption,
  type GlzQuestion,
  GlzQuestionCardComponent,
  GlzReasoningComponent,
  GlzReceiptComponent,
  GlzStickToBottomDirective,
  GlzSwitchComponent,
} from '@glave/glaze';

const COPY = {
  heading: 'Conversation',
  thread: 'A thread',
  you: 'You',
  hope: 'Hope',
  ask: 'Make the tests pass and tell me what was wrong.',
  reasoning: 'The failure is in the date parser; the test expects UTC.',
  reply: 'The parser read local time. I changed it to UTC and the tests pass now.',
  noteAuthor: 'glave',
  note: 'The conversation was compacted to fit.',
  latest: 'Jump to latest',
  model: 'Model',
  cloud: 'Run in the cloud',
  answer: 'Done. Anything else?',
  streamed:
    'Streaming text arrives a few words at a time, and the list keeps to the bottom while you are there.',
} as const;

const MODELS: readonly GlzOption[] = [
  { value: 'hope', label: 'Hope' },
  { value: 'haiku', label: 'Claude Haiku' },
  { value: 'luna', label: 'GPT-6 Luna' },
];

const QUESTIONS: readonly GlzQuestion[] = [
  {
    question: 'Which database should the space use?',
    options: [
      { label: 'SQLite', description: 'One file on this computer' },
      { label: 'Postgres', description: "The organisation's server" },
    ],
  },
  {
    question: 'Which checks should run first?',
    options: [{ label: 'Lint' }, { label: 'Tests' }],
    multiple: true,
  },
];

/** A message in the demo thread. */
interface DemoMessage {
  readonly id: number;
  readonly from: 'person' | 'model';
  readonly author: string;
  readonly text: string;
}

/** The showcase's conversation: messages, reasoning, receipts, approvals, questions, composer. */
@Component({
  selector: 'app-showcase-ai',
  imports: [
    GlzApprovalCardComponent,
    GlzButtonComponent,
    GlzComboboxComponent,
    GlzComposerComponent,
    GlzMessageComponent,
    GlzQuestionCardComponent,
    GlzReasoningComponent,
    GlzReceiptComponent,
    GlzStickToBottomDirective,
    GlzSwitchComponent,
  ],
  templateUrl: './showcase-ai.component.html',
  styleUrl: './showcase-ai.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ShowcaseAiComponent {
  protected readonly copy = COPY;
  protected readonly models = MODELS;
  protected readonly questions = QUESTIONS;
  protected readonly cost: GlzMoney = { micros: 4200, currency: 'USD' };
  protected readonly decision = signal<GlzDecision | null>(null);
  protected readonly answers = signal<readonly string[] | null>(null);
  protected readonly draft = signal('');
  protected readonly model = signal<string | null>('hope');
  protected readonly cloud = signal(false);
  protected readonly busy = signal(false);
  protected readonly atBottom = signal(true);
  protected readonly awayFromBottom = computed(() => !this.atBottom());
  private readonly thread = viewChild.required(GlzStickToBottomDirective);
  protected readonly messages = signal<readonly DemoMessage[]>([]);
  private nextId = 1;
  private timer: ReturnType<typeof setInterval> | undefined;

  constructor() {
    inject(DestroyRef).onDestroy(() => {
      clearInterval(this.timer);
    });
  }

  protected decide(decision: GlzDecision): void {
    this.decision.set(decision);
  }

  protected answer(answers: readonly string[]): void {
    this.answers.set(answers);
  }

  protected send(composed: GlzComposed): void {
    this.add('person', COPY.you, composed.text);
    this.stream(`${COPY.answer} ${COPY.streamed}`);
  }

  protected jumpToLatest(): void {
    this.thread().scrollToBottom();
  }

  protected stop(): void {
    clearInterval(this.timer);
    this.busy.set(false);
  }

  private add(from: DemoMessage['from'], author: string, text: string): void {
    this.messages.update((messages) => [...messages, { id: this.nextId, from, author, text }]);
    this.nextId += 1;
  }

  /** Shows `text` arriving a few words at a time, as a model's reply does. */
  private stream(text: string): void {
    const words = text.split(' ');
    this.add('model', COPY.hope, '');
    this.busy.set(true);
    let shown = 0;
    clearInterval(this.timer);
    this.timer = setInterval(() => {
      shown += 2;
      const partial = words.slice(0, shown).join(' ');
      this.messages.update((messages) =>
        messages.map((message, index) =>
          index === messages.length - 1 ? { ...message, text: partial } : message,
        ),
      );
      if (shown >= words.length) {
        this.stop();
      }
    }, 80);
  }
}
