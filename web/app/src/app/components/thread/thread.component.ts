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
  GlzButtonComponent,
  type GlzComposed,
  GlzComposerComponent,
  type GlzDecision,
  GlzMessageComponent,
  GlzPanelLayoutComponent,
  GlzQuestionCardComponent,
  GlzReasoningComponent,
  GlzReceiptComponent,
  GlzStickToBottomDirective,
} from '@glave/glaze';
import { type MessageItem, type ThreadItem } from '../../lib/conversation';
import { threadMark, threadTitle } from '../../lib/threads';
import { ModelsService } from '../../services/models.service';
import { SpacesService } from '../../services/spaces.service';
import { ThreadService } from '../../services/thread.service';
import { ToolsPanelService } from '../../services/tools-panel.service';
import { ApprovalModeComponent } from '../approval-mode/approval-mode.component';
import type { FirstPrompt } from '../home/home.component';
import { ModelPickerComponent } from '../model-picker/model-picker.component';
import { RenameDialogComponent } from '../rename-dialog/rename-dialog.component';
import { ThreadHeaderComponent } from '../thread-header/thread-header.component';
import { ToolsPanelComponent } from '../tools-panel/tools-panel.component';

const COPY = {
  you: 'You',
  model: 'Model',
  latest: 'Jump to latest',
  failed: 'Something went wrong:',
  placeholder: 'Reply',
  conversation: 'Conversation',
  tools: 'Tools',
  resize: 'Resize the tools',
  hint: 'Drag or use the arrow keys to resize; Enter collapses. Show or hide:',
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
 * One thread: its top bar floating over it (title, space, rename; on narrow screens the button
 * that opens the tools); its conversation
 * (messages named by the model that wrote them, reasoning, receipts, approvals and questions,
 * answered in place) in a reading column, the reply streaming in as a turn runs; the composer
 * docked below it, with the approval mode and the model picker; and the tools panel beside it,
 * resized and collapsed by its edge and shown or hidden by ⌘. or Ctrl+. anywhere in the thread.
 */
@Component({
  selector: 'app-thread',
  imports: [
    ApprovalModeComponent,
    GlzApprovalCardComponent,
    GlzButtonComponent,
    GlzComposerComponent,
    GlzMessageComponent,
    GlzPanelLayoutComponent,
    GlzQuestionCardComponent,
    GlzReasoningComponent,
    GlzReceiptComponent,
    GlzStickToBottomDirective,
    ModelPickerComponent,
    RenameDialogComponent,
    ThreadHeaderComponent,
    ToolsPanelComponent,
  ],
  providers: [ThreadService],
  templateUrl: './thread.component.html',
  styleUrl: './thread.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '(document:keydown)': 'keydown($event)' },
})
export class ThreadComponent {
  /** The thread's id, from the route. */
  readonly thread = input.required<string>();

  protected readonly copy = COPY;
  protected readonly service = inject(ThreadService);
  private readonly spaces = inject(SpacesService);
  private readonly models = inject(ModelsService);
  private readonly panel = inject(ToolsPanelService);
  private readonly stick = viewChild.required(GlzStickToBottomDirective);
  protected readonly draft = signal('');
  protected readonly atBottom = signal(true);
  protected readonly awayFromBottom = computed(() => !this.atBottom());
  /** The model the person picked for the next turn, or `null` for the thread's own. */
  protected readonly picked = signal<string | null>(null);
  protected readonly renaming = signal(false);
  protected readonly panelOpen = this.panel.open;
  protected readonly panelWidth = this.panel.width;
  protected readonly panelShortcut = this.panel.ariaShortcut;
  protected readonly panelHint = `${COPY.hint} ${this.panel.shortcut}`;
  protected readonly shown = computed<readonly ShownItem[]>(() =>
    this.service.items().map((item) => {
      if (item.kind !== 'message') {
        return item;
      }
      const label =
        item.from === 'person'
          ? { name: COPY.you, hope: false }
          : this.models.label(item.model, COPY.model);
      return { ...item, author: label.name, hope: label.hope };
    }),
  );
  /** Who is writing the reply streaming in: the model the running turn was sent to. */
  private readonly replyLabel = computed(() => this.models.label(this.service.model(), COPY.model));
  protected readonly replyAuthor = computed(() => this.replyLabel().name);
  protected readonly replyHope = computed(() => this.replyLabel().hope);
  protected readonly summary = computed(() => this.spaces.thread(this.thread()));
  protected readonly title = computed(() => {
    const summary = this.summary();
    return summary === undefined ? '' : threadTitle(summary);
  });
  protected readonly spaceName = computed(() => this.spaces.spaceOf(this.thread())?.name ?? null);
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
        this.picked.set(null);
        void this.service
          .open(thread)
          .then(() => (prompt === null ? undefined : this.service.send(prompt)));
      });
    });
  }

  protected send(composed: GlzComposed): void {
    // The pick goes with the turn only when it changes the thread's model.
    const picked = this.picked();
    const model = picked !== null && picked !== this.service.chosen() ? picked : null;
    void this.service.send(composed.text, model);
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

  protected startRename(): void {
    this.renaming.set(true);
  }

  protected rename(title: string): void {
    void this.service.rename(title);
  }

  protected keydown(event: KeyboardEvent): void {
    this.panel.keydown(event);
  }
}
