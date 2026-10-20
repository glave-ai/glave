import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import {
  GlzApprovalCardComponent,
  GlzAttachmentComponent,
  GlzCopyButtonComponent,
  type GlzDecision,
  type GlzIconData,
  GlzMessageComponent,
  GlzQuestionCardComponent,
  GlzReasoningComponent,
  GlzReceiptComponent,
} from '@glave/glaze';

import { type Ask, askOf } from '../../lib/approvals';
import type {
  ApprovalItem,
  CallItem,
  MessageItem,
  QuestionItem,
  ThreadItem,
} from '../../lib/conversation';
import { toolIcon } from '../../lib/steps';
import { ThreadService } from '../../services/thread.service';
import { MessageTextComponent } from '../message-text/message-text.component';

const COPY = {
  you: 'You',
  reply: 'Reply',
  read: 'Read while working',
} as const;

/** A message as the thread shows it: named for screen readers, "You" or "Reply". */
interface ShownMessage extends MessageItem {
  readonly author: string;
  /** Whether it is a reply, shown as markdown; the person's own text shows as typed. */
  readonly markdown: boolean;
  /** Whether it has text to copy. */
  readonly copyable: boolean;
}

/** A tool call as the thread shows it: with its tool's icon. */
interface ShownCall extends CallItem {
  readonly icon: GlzIconData;
}

/** An approval as the thread shows it: what it asks, in a sentence. */
interface ShownApproval extends ApprovalItem {
  readonly ask: Ask;
}

type ShownItem = ShownMessage | ShownCall | ShownApproval | QuestionItem;

function shown(item: ThreadItem): ShownItem {
  switch (item.kind) {
    case 'message': {
      const person = item.from === 'person';
      return {
        ...item,
        author: person ? COPY.you : COPY.reply,
        markdown: !person,
        // Only a reply is copied; what the person wrote is theirs already.
        copyable: !person && item.text !== '',
      };
    }
    case 'call':
      return { ...item, icon: toolIcon(item.tool) };
    case 'approval':
      return { ...item, ask: askOf(item.tool, item.summary) };
    case 'question':
      return item;
  }
}

/**
 * One thing in a thread, as its kind shows: a message (the person's as a bubble of what they
 * typed, saying so when the model read it as it worked; a reply as markdown with no name over
 * it, each with a copy button; its reasoning), a tool call as a quiet line, an approval or
 * questions as a card that needs the person, answered in place.
 */
@Component({
  selector: 'app-thread-item',
  imports: [
    GlzApprovalCardComponent,
    GlzAttachmentComponent,
    GlzCopyButtonComponent,
    GlzMessageComponent,
    GlzQuestionCardComponent,
    GlzReasoningComponent,
    GlzReceiptComponent,
    MessageTextComponent,
  ],
  templateUrl: './thread-item.component.html',
  styleUrl: './thread-item.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThreadItemComponent {
  readonly item = input.required<ThreadItem>();

  private readonly thread = inject(ThreadService);
  /** Whether an answer is being sent, so the cards wait. */
  protected readonly sending = this.thread.sending;
  protected readonly shown = computed(() => shown(this.item()));
  protected readonly copy = COPY;

  protected decide(approval: string, decision: GlzDecision): void {
    void this.thread.decide(approval, decision);
  }

  protected answer(question: string, answers: readonly string[]): void {
    void this.thread.answer(question, answers);
  }
}
