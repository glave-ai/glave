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
import type { ApprovalMode } from '@glave/client';
import {
  GlzApprovalCardComponent,
  GlzButtonComponent,
  type GlzComposed,
  GlzComposerComponent,
  type GlzDecision,
  type GlzIconData,
  GlzMessageComponent,
  GlzPanelLayoutComponent,
  GlzQuestionCardComponent,
  GlzReasoningComponent,
  GlzReceiptComponent,
  GlzStepsComponent,
  GlzStickToBottomDirective,
} from '@glave/glaze';
import type { ApprovalItem, CallItem, MessageItem, QuestionItem } from '../../lib/conversation';
import { type StepsItem, foldSteps, toolIcon } from '../../lib/steps';
import { threadMark, threadTitle } from '../../lib/threads';
import { SpacesService } from '../../services/spaces.service';
import { ThreadService } from '../../services/thread.service';
import { ToolsPanelService } from '../../services/tools-panel.service';
import { ApprovalModeComponent } from '../approval-mode/approval-mode.component';
import { CopyButtonComponent } from '../copy-button/copy-button.component';
import type { FirstPrompt } from '../home/home.component';
import { MessageTextComponent } from '../message-text/message-text.component';
import { ModelPickerComponent } from '../model-picker/model-picker.component';
import { RenameDialogComponent } from '../rename-dialog/rename-dialog.component';
import { ThreadHeaderComponent } from '../thread-header/thread-header.component';
import { ToolsPanelComponent } from '../tools-panel/tools-panel.component';

const COPY = {
  you: 'You',
  reply: 'Reply',
  latest: 'Jump to latest',
  failed: 'Something went wrong:',
  placeholder: 'Reply',
  conversation: 'Conversation',
  tools: 'Tools',
  resize: 'Resize the tools',
  hint: 'Drag or use the arrow keys to resize; Enter collapses. Show or hide:',
} as const;

/** A message as the thread shows it: named for screen readers, "You" or "Reply". */
interface ShownMessage extends MessageItem {
  readonly author: string;
  /** Whether a long text folds: the person's own messages do, replies read in full. */
  readonly foldable: boolean;
}

/** A tool call as the thread shows it: with its tool's icon. */
interface ShownCall extends CallItem {
  readonly icon: GlzIconData;
}

/** Tool calls in a row, folded under one line. */
interface ShownSteps extends Omit<StepsItem, 'calls'> {
  readonly calls: readonly ShownCall[];
}

type ShownItem = ApprovalItem | QuestionItem | ShownMessage | ShownCall | ShownSteps;

function shownCall(call: CallItem): ShownCall {
  return { ...call, icon: toolIcon(call.tool) };
}

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
 * that opens the tools); its conversation (the person's messages as bubbles, replies as plain
 * text with no name over them, each with a copy button; reasoning; tool calls as quiet lines,
 * those in a row folded under "Ran N steps"; approvals and questions, answered in place) in a
 * reading column, the reply streaming in as a turn runs; the composer
 * docked below it, with the approval mode and the model picker; and the tools panel beside it,
 * resized and collapsed by its edge and shown or hidden by ⌘. or Ctrl+. anywhere in the thread.
 */
@Component({
  selector: 'app-thread',
  imports: [
    ApprovalModeComponent,
    CopyButtonComponent,
    GlzApprovalCardComponent,
    GlzButtonComponent,
    GlzComposerComponent,
    GlzMessageComponent,
    GlzPanelLayoutComponent,
    GlzQuestionCardComponent,
    GlzReasoningComponent,
    GlzReceiptComponent,
    GlzStepsComponent,
    GlzStickToBottomDirective,
    MessageTextComponent,
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
  private readonly panel = inject(ToolsPanelService);
  private readonly stick = viewChild.required(GlzStickToBottomDirective);
  protected readonly draft = signal('');
  protected readonly atBottom = signal(true);
  protected readonly awayFromBottom = computed(() => !this.atBottom());
  /** The model the person picked for the next turn, or `null` for the thread's own. */
  protected readonly picked = signal<string | null>(null);
  /** The approval mode the person picked for the next turn, or `null` for the thread's own. */
  protected readonly pickedMode = signal<ApprovalMode | null>(null);
  protected readonly renaming = signal(false);
  protected readonly panelOpen = this.panel.open;
  protected readonly panelWidth = this.panel.width;
  protected readonly panelShortcut = this.panel.ariaShortcut;
  protected readonly panelHint = `${COPY.hint} ${this.panel.shortcut}`;
  protected readonly shown = computed<readonly ShownItem[]>(() =>
    foldSteps(this.service.items()).map((item): ShownItem => {
      switch (item.kind) {
        case 'message': {
          const person = item.from === 'person';
          return { ...item, author: person ? COPY.you : COPY.reply, foldable: person };
        }
        case 'call':
          return shownCall(item);
        case 'steps':
          return { ...item, calls: item.calls.map(shownCall) };
        default:
          return item;
      }
    }),
  );
  protected readonly summary = computed(() => this.spaces.thread(this.thread()));
  protected readonly title = computed(() => {
    const summary = this.summary();
    return summary === undefined ? '' : threadTitle(summary);
  });
  /** The thread's own approval mode, from its summary, or `null` until it is read. */
  protected readonly mode = computed(() => this.summary()?.approval ?? null);
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
        this.pickedMode.set(null);
        void this.service
          .open(thread)
          .then(() => (prompt === null ? undefined : this.service.send(prompt)));
      });
    });
  }

  protected send(composed: GlzComposed): void {
    // A pick goes with the turn only when it changes the thread's model or mode.
    const picked = this.picked();
    const model = picked !== null && picked !== this.service.chosen() ? picked : null;
    const pickedMode = this.pickedMode();
    const mode = pickedMode !== null && pickedMode !== this.mode() ? pickedMode : null;
    void this.service.send(composed.text, model, mode);
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
