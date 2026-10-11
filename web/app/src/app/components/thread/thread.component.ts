import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  signal,
  untracked,
  viewChild,
} from '@angular/core';
import { Router } from '@angular/router';
import {
  GlzApprovalCardComponent,
  GlzBadgeComponent,
  type GlzComposed,
  GlzComposerComponent,
  type GlzDecision,
  GlzMessageComponent,
  GlzQuestionCardComponent,
  GlzReasoningComponent,
  GlzReceiptComponent,
  GlzStickToBottomDirective,
  GlzButtonComponent,
} from '@glave/glaze';

import { type MessageItem, type ThreadItem, isHope } from '../../lib/conversation';
import { threadMark, threadTitle } from '../../lib/threads';
import { SpacesService } from '../../services/spaces.service';
import { ThreadService } from '../../services/thread.service';
import type { FirstPrompt } from '../home/home.component';

const COPY = {
  you: 'You',
  model: 'Model',
  latest: 'Jump to latest',
  failed: 'Something went wrong:',
  placeholder: 'Reply',
  conversation: 'Conversation',
} as const;

/** A message as the thread shows it: with who wrote it. */
interface ShownMessage extends MessageItem {
  readonly author: string;
  readonly hope: boolean;
}

type ShownItem = Exclude<ThreadItem, MessageItem> | ShownMessage;

function isFirstPrompt(state: unknown): state is FirstPrompt {
  return (
    typeof state === 'object' &&
    state !== null &&
    'prompt' in state &&
    typeof state.prompt === 'string'
  );
}

/**
 * One thread: its conversation (messages, reasoning, receipts, approvals and questions,
 * answered in place), the reply streaming in as a turn runs, and the composer.
 */
@Component({
  selector: 'app-thread',
  imports: [
    GlzApprovalCardComponent,
    GlzBadgeComponent,
    GlzButtonComponent,
    GlzComposerComponent,
    GlzMessageComponent,
    GlzQuestionCardComponent,
    GlzReasoningComponent,
    GlzReceiptComponent,
    GlzStickToBottomDirective,
  ],
  providers: [ThreadService],
  templateUrl: './thread.component.html',
  styleUrl: './thread.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThreadComponent {
  /** The thread's id, from the route. */
  readonly thread = input.required<string>();

  protected readonly copy = COPY;
  protected readonly service = inject(ThreadService);
  private readonly spaces = inject(SpacesService);
  private readonly stick = viewChild.required(GlzStickToBottomDirective);
  protected readonly draft = signal('');
  protected readonly atBottom = signal(true);
  protected readonly awayFromBottom = computed(() => !this.atBottom());
  protected readonly shown = computed<readonly ShownItem[]>(() =>
    this.service.items().map((item) =>
      item.kind === 'message'
        ? {
            ...item,
            author: item.from === 'person' ? COPY.you : (item.model ?? COPY.model),
            hope: isHope(item.model),
          }
        : item,
    ),
  );
  /** Who is writing the reply streaming in: the latest turn's model. */
  protected readonly replyAuthor = computed(() => this.service.model() ?? COPY.model);
  protected readonly replyHope = computed(() => isHope(this.service.model()));
  protected readonly summary = computed(() => this.spaces.thread(this.thread()));
  protected readonly title = computed(() => {
    const summary = this.summary();
    return summary === undefined ? '' : threadTitle(summary);
  });
  protected readonly mark = computed(() => {
    const summary = this.summary();
    return summary === undefined ? null : threadMark(summary);
  });

  constructor() {
    const navigation = inject(Router).currentNavigation();
    const state: unknown = navigation?.extras.state;
    let first = isFirstPrompt(state) ? state.prompt : null;
    effect(() => {
      const thread = this.thread();
      untracked(() => {
        const prompt = first;
        first = null;
        void this.service
          .open(thread)
          .then(() => (prompt === null ? undefined : this.service.send(prompt)));
      });
    });
  }

  protected send(composed: GlzComposed): void {
    void this.service.send(composed.text);
  }

  protected stop(): void {
    void this.service.stop();
  }

  protected decide(approval: string, decision: GlzDecision): void {
    void this.service.decide(approval, decision);
  }

  protected answer(question: string, answers: readonly string[]): void {
    void this.service.answer(question, answers);
  }

  protected jumpToLatest(): void {
    this.stick().scrollToBottom();
  }
}
