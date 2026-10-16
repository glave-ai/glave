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
  GlzComposerComponent,
  type GlzDecision,
  GlzMarkdownComponent,
  type GlzIconData,
  GlzMenuSelectComponent,
  GlzMessageComponent,
  type GlzMenuOption,
  type GlzQuestion,
  type GlzReceiptStatus,
  GlzQuestionCardComponent,
  GlzReasoningComponent,
  GlzReceiptComponent,
  GlzStepsComponent,
  GlzStickToBottomDirective,
} from '@glave/glaze';
import { SquareTerminal, TextSearch } from 'lucide';

const COPY = {
  heading: 'Conversation',
  thread: 'A thread',
  you: 'You',
  replyAuthor: 'Reply',
  steps: 'Ran 2 steps, 1 failed',
  ask: 'Make the tests pass and tell me what was wrong.',
  reasoning: 'The failure is in the date parser; the test expects UTC.',
  reply:
    'The parser read local time. I changed `parse_date` to **UTC**, and the tests pass now:\n\n' +
    '```sh\ncargo test dates\n```',
  noteAuthor: 'glave',
  note: 'The conversation was compacted to fit.',
  latest: 'Jump to latest',
  model: 'Model',
  answer: 'Done. Anything else?',
  streamed:
    'Streaming text arrives a few words at a time, and the list keeps to the bottom while you are there.',
} as const;

const MODELS: readonly GlzMenuOption[] = [
  { value: 'hope', label: 'Hope', hint: 'Default' },
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

/** The demo's tool calls. */
const RECEIPTS: readonly {
  readonly icon: GlzIconData;
  readonly tool: string;
  readonly summary: string;
  readonly status: GlzReceiptStatus;
  readonly evidence: string;
}[] = [
  {
    icon: TextSearch,
    tool: 'search',
    summary: 'parse_date',
    status: 'done',
    evidence: 'src/dates.rs:41',
  },
  {
    icon: SquareTerminal,
    tool: 'bash',
    summary: 'cargo test',
    status: 'failed',
    evidence: 'test dates::parses_utc ... FAILED',
  },
];

/** The demo's approval. */
const APPROVAL = {
  tool: 'bash',
  summary: 'Run the whole test suite again',
  command: 'cargo test --workspace',
} as const;

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
    GlzComposerComponent,
    GlzMarkdownComponent,
    GlzMenuSelectComponent,
    GlzMessageComponent,
    GlzQuestionCardComponent,
    GlzReasoningComponent,
    GlzReceiptComponent,
    GlzStepsComponent,
    GlzStickToBottomDirective,
  ],
  templateUrl: './showcase-ai.component.html',
  styleUrl: './showcase-ai.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ShowcaseAiComponent {
  protected readonly copy = COPY;
  protected readonly models = MODELS;
  protected readonly questions = QUESTIONS;
  protected readonly receipts = RECEIPTS;
  /** The demo's steps start open, to show each call. */
  protected readonly stepsOpen = signal(true);
  protected readonly approval = APPROVAL;
  protected readonly decision = signal<GlzDecision | null>(null);
  protected readonly answers = signal<readonly string[] | null>(null);
  protected readonly draft = signal('');
  protected readonly model = signal<string | null>('hope');
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

  protected send(text: string): void {
    this.add('person', COPY.you, text);
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
    this.add('model', COPY.replyAuthor, '');
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
