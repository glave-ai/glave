import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  input,
} from '@angular/core';
import {
  GlzApprovalCardComponent,
  GlzAttachmentComponent,
  GlzCopyButtonComponent,
  type GlzDecision,
  type GlzIconData,
  GlzMessageComponent,
  GlzQuestionCardComponent,
  GlzReceiptComponent,
  GlzServiceCardComponent,
} from '@glave/glaze';

import { sizeOf } from '../../lib/attachments';
import type {
  ApprovalItem,
  CallItem,
  MessageItem,
  QuestionItem,
  ServiceItem,
  ThreadItem,
} from '../../lib/conversation';
import { toolIcon } from '../../lib/steps';
import { ThreadService } from '../../services/thread.service';
import { MessageTextComponent } from '../message-text/message-text.component';
import { ThoughtComponent } from '../thought/thought.component';

/**
 * A message as the thread shows it: named for screen readers, "You" or "Reply", each file it
 * was sent with by its name and size.
 */
interface ShownMessage extends Omit<MessageItem, 'files'> {
  readonly author: string;
  readonly files: readonly { readonly name: string; readonly size: string }[];
  /** Whether it is a reply, shown as markdown; the person's own text shows as typed. */
  readonly markdown: boolean;
  /** Whether it has text to copy. */
  readonly copyable: boolean;
}

/** A tool call as the thread shows it: with its tool's icon. */
interface ShownCall extends CallItem {
  readonly icon: GlzIconData;
}

/** What a service asks as the thread shows it: with where the answer goes, naming it. */
interface ShownService extends ServiceItem {
  readonly sendLabel: string;
  readonly note: string;
}

type ShownItem = ShownMessage | ShownCall | ApprovalItem | QuestionItem | ShownService;

/**
 * `item` as the thread shows it, its sizes written for `locale`: a message of the model's said
 * along the way (`interim`) named an update, its last words its reply.
 */
function shown(item: ThreadItem, locale: string, interim: boolean): ShownItem {
  switch (item.kind) {
    case 'message': {
      const person = item.from === 'person';
      return {
        ...item,
        // The person's approval of a plan reads as what they pressed.
        text: item.built === true ? $localize`:@@threadItem.built:Build it` : item.text,
        author: person
          ? $localize`:@@threadItem.you:You`
          : interim
            ? $localize`:@@threadItem.update:Update`
            : $localize`:@@threadItem.reply:Reply`,
        files: item.files.map((file) => ({ name: file.name, size: sizeOf(file.bytes, locale) })),
        markdown: !person,
        // Only a reply is copied; what the person wrote is theirs already.
        copyable: !person && item.text !== '',
      };
    }
    case 'call':
      return { ...item, icon: toolIcon(item.tool) };
    case 'approval':
    case 'question':
      return item;
    case 'service':
      return {
        ...item,
        sendLabel: $localize`:@@threadItem.serviceSend:Send to ${item.service}:SERVICE:`,
        note: $localize`:@@threadItem.serviceNote:Your answer goes only to ${item.service}:SERVICE:. The model sees that you answered, not what you typed.`,
      };
  }
}

/**
 * One thing in a thread, as its kind shows: a message (the person's as a bubble of what they
 * typed, saying so when the model read it as it worked; a reply as markdown with no name over
 * it, each with a copy button; its reasoning in a line, which opens to it in full), a tool call
 * as a quiet line, an approval, questions or what a service asks as a card that needs the person,
 * answered in place, which rises into place when it `rises`.
 */
@Component({
  selector: 'app-thread-item',
  imports: [
    GlzApprovalCardComponent,
    GlzAttachmentComponent,
    GlzCopyButtonComponent,
    GlzMessageComponent,
    GlzQuestionCardComponent,
    GlzReceiptComponent,
    GlzServiceCardComponent,
    MessageTextComponent,
    ThoughtComponent,
  ],
  templateUrl: './thread-item.component.html',
  styleUrl: './thread-item.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThreadItemComponent {
  readonly item = input.required<ThreadItem>();
  /** Whether a card that needs the person rises into place: it appeared as the person watched. */
  readonly rises = input(false);
  /** Whether it is a message of the model's said along the way, not its turn's reply. */
  readonly interim = input(false);

  private readonly thread = inject(ThreadService);
  /** Whether an answer is being sent, so the cards wait. */
  protected readonly sending = this.thread.sending;
  private readonly locale = inject(LOCALE_ID);
  protected readonly shown = computed(() => shown(this.item(), this.locale, this.interim()));
  protected readonly copy = { read: $localize`:@@threadItem.read:Read while working` } as const;

  protected decide(approval: string, decision: GlzDecision): void {
    void this.thread.decide(approval, decision);
  }

  protected answer(question: string, answers: readonly string[]): void {
    void this.thread.answer(question, answers);
  }

  protected answerService(
    call: string,
    asked: number,
    values: Readonly<Record<string, unknown>> | null,
  ): void {
    void this.thread.answerService(call, asked, values);
  }
}
