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
  GlzReceiptComponent,
  GlzStepsComponent,
  GlzStickToBottomDirective,
  GlzThoughtComponent,
  type GlzTrustAnswer,
  GlzTrustCardComponent,
  type GlzTrustItem,
} from '@glave/glaze';
import { SquareTerminal, TextSearch } from 'lucide';

const COPY = {
  heading: 'Conversation',
  thread: 'A thread',
  you: 'You',
  replyAuthor: 'Reply',
  steps: '2 steps',
  ask: 'Make the tests pass and tell me what was wrong.',
  thought: 'Thought for 4s',
  reasoning: 'The failure is in the date parser; the test expects UTC.',
  reply:
    'The parser read local time. I changed `parse_date` to **UTC**, and the tests pass now:\n\n' +
    '```sh\ncargo test dates\n```',
  noteAuthor: 'Atlas',
  note: 'The conversation was compacted to fit.',
  latest: 'Jump to latest',
  model: 'Model',
  answer: 'Done. Anything else?',
  trust: 'This project brings its own tools and settings',
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
  readonly summary: string;
  readonly status: GlzReceiptStatus;
  readonly evidence: string;
}[] = [
  {
    icon: TextSearch,
    summary: 'search parse_date',
    status: 'done',
    evidence: 'src/dates.rs:41',
  },
  {
    icon: SquareTerminal,
    summary: 'bash: cargo test',
    status: 'failed',
    evidence: 'test dates::parses_utc ... FAILED',
  },
];

/** What the demo's steps came to besides their status: the time they took and their cost. */
const FACTS: readonly string[] = ['12s', '$0.0042'];

/** The demo's approval: a risky command, which says why it asks apart from the command. */
const APPROVAL = {
  summary: 'Run this command?',
  command: 'git clean -fdx',
  caution: 'It deletes untracked files.',
} as const;

/** What the demo's project brings, waiting for the person's trust. */
const TRUST: readonly GlzTrustItem[] = [
  { text: 'Runs a tool server', detail: 'npx @acme/mcp' },
  { text: 'Runs a hook before each command', detail: './scripts/check.sh' },
  { text: 'Lets the assistant approve actions without asking' },
];

/** A message in the demo thread. */
interface DemoMessage {
  readonly id: number;
  readonly from: 'person' | 'model';
  readonly author: string;
  readonly text: string;
}

/**
 * The showcase's conversation: messages, reasoning, receipts, approvals, questions, a project
 * to trust, composer.
 */
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
    GlzReceiptComponent,
    GlzStepsComponent,
    GlzStickToBottomDirective,
    GlzThoughtComponent,
    GlzTrustCardComponent,
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
  protected readonly facts = FACTS;
  /** The demo's steps start open, to show each call. */
  protected readonly stepsOpen = signal(true);
  protected readonly approval = APPROVAL;
  protected readonly decision = signal<GlzDecision | null>(null);
  protected readonly answers = signal<readonly string[] | null>(null);
  protected readonly trustItems = TRUST;
  protected readonly trusted = signal<GlzTrustAnswer | null>(null);
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

  protected trust(answer: GlzTrustAnswer): void {
    this.trusted.set(answer);
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
