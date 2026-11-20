import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  input,
} from '@angular/core';
import { RouterLink } from '@angular/router';
import type { ThreadSummary } from '@glave/client';
import {
  GlzApprovalCardComponent,
  GlzAttachmentComponent,
  type GlzCardLine,
  GlzCopyButtonComponent,
  type GlzDecision,
  GlzIconComponent,
  type GlzIconData,
  GlzMessageComponent,
  GlzQuestionCardComponent,
  GlzReceiptComponent,
  GlzServiceCardComponent,
} from '@glave/glaze';
import { Clock, MessageSquare, Repeat, TextQuote } from 'lucide';

import { sizeOf } from '../../lib/attachments';
import type {
  ApprovalItem,
  CallItem,
  MessageItem,
  QuestionItem,
  RepeatAsk,
  ServiceItem,
  ThreadItem,
} from '../../lib/conversation';
import { toolIcon } from '../../lib/steps';
import { threadTitle } from '../../lib/threads';
import { ProjectsService } from '../../services/projects.service';
import { SessionService } from '../../services/session.service';
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

/** What a service asks as the thread shows it: its button naming who the answer goes to. */
interface ShownService extends ServiceItem {
  readonly sendLabel: string;
}

/** The icon a card asking to repeat work draws before each of its lines, by what it says. */
const LINE_ICONS = { when: Clock, chat: MessageSquare, prompt: TextQuote } as const;

/**
 * A card asking to repeat work, as the thread shows it: the server's question, its lines, each
 * after the icon of what it says, and its caution, while it waits; once the call made the
 * schedule, a line that opens the schedule's chat, in the words the server gives that chat:
 * when its schedule repeats, else its title; while the call goes on, once it failed or was
 * declined, nothing (the run of calls says how the call went).
 */
interface ShownRepeat {
  readonly kind: 'repeat';
  readonly approval: string;
  readonly ask: string;
  readonly lines: readonly GlzCardLine[];
  readonly caution: string;
  readonly waiting: boolean;
  readonly made: { readonly words: string; readonly link: readonly string[] } | null;
}

type ShownItem =
  ShownMessage | ShownCall | ApprovalItem | QuestionItem | ShownService | ShownRepeat;

/** What the server says of a chat, by its id, as the person's lists hold it, if they do. */
type Chats = (chat: string) => ThreadSummary | undefined;

/** `item`, a card asking to repeat work, as the thread shows it, its chat found in `chats`. */
function shownRepeat(item: ApprovalItem, repeat: RepeatAsk, chats: Chats): ShownRepeat {
  const allowed = item.decision === 'allow_once' || item.decision === 'allow_for_thread';
  const chat = allowed && repeat.chat !== null ? chats(repeat.chat) : undefined;
  const words = chat === undefined ? null : (chat.schedule?.repeats ?? threadTitle(chat));
  return {
    kind: 'repeat',
    approval: item.approval,
    ask: item.ask,
    lines: repeat.lines.map(({ says, text }) =>
      says === null ? { text } : { text, icon: LINE_ICONS[says] },
    ),
    caution: item.caution,
    waiting: item.decision === null,
    made: chat === undefined || words === null ? null : { words, link: ['/t', chat.id] },
  };
}

/**
 * `item` as the thread shows it, its sizes written for `locale`: a message of the model's said
 * along the way (`interim`) named an update, its last words its reply.
 */
function shown(item: ThreadItem, locale: string, interim: boolean, chats: Chats): ShownItem {
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
      return item.repeat === undefined ? item : shownRepeat(item, item.repeat, chats);
    case 'question':
      return item;
    case 'service':
      return {
        ...item,
        sendLabel: $localize`:@@threadItem.serviceSend:Send to ${item.service}:SERVICE:`,
      };
  }
}

/**
 * One thing in a thread, as its kind shows: a message (the person's as a bubble of what they
 * typed, saying so when the model read it as it worked; a reply as markdown with no name over
 * it, each with a copy button; its reasoning in a line, which opens to it in full), a tool call
 * as a quiet line, an approval, questions or what a service asks as a card that needs the person,
 * answered in place, which rises into place when it `rises`; a card asking to repeat work,
 * answered Repeat it or Not now, folds to a line in the server's words that opens the schedule's
 * chat, or away.
 */
@Component({
  selector: 'app-thread-item',
  imports: [
    RouterLink,
    GlzApprovalCardComponent,
    GlzAttachmentComponent,
    GlzCopyButtonComponent,
    GlzIconComponent,
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
  private readonly session = inject(SessionService);
  /** Whether an answer is being sent, so the cards wait. */
  protected readonly sending = this.thread.sending;
  private readonly locale = inject(LOCALE_ID);
  private readonly projects = inject(ProjectsService);
  protected readonly shown = computed(() =>
    shown(this.item(), this.locale, this.interim(), (chat) => this.projects.thread(chat)),
  );
  protected readonly copy = {
    read: $localize`:@@threadItem.read:Read while working`,
    repeatIt: $localize`:@@threadItem.repeatIt:Repeat it`,
    notNow: $localize`:@@threadItem.notNow:Not now`,
    repeats: $localize`:@@threadItem.repeating:Repeats`,
  } as const;
  protected readonly icons = { repeat: Repeat };

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
    const thread = this.thread.thread();
    if (thread !== null) {
      const answer = values === null ? ('declined' as const) : { answered: { values } };
      void this.thread.act(() =>
        this.session.call('service/answer', { thread, call, asked, answer }),
      );
    }
  }
}
