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
  GlzButtonComponent,
  GlzComposerComponent,
  GlzMessageComponent,
  GlzPanelLayoutComponent,
  GlzStickToBottomDirective,
  type GlzTrustAnswer,
  GlzTrustCardComponent,
} from '@glave/glaze';

import { type TurnResults, turnResults } from '../../lib/results';
import { threadTitle } from '../../lib/threads';
import type { Turn } from '../../lib/turns';
import { CommandsService } from '../../services/commands.service';
import { SpacesService } from '../../services/spaces.service';
import { ThreadService } from '../../services/thread.service';
import { ToolsPanelService } from '../../services/tools-panel.service';
import { ApprovalModeComponent } from '../approval-mode/approval-mode.component';
import type { FirstPrompt } from '../home/home.component';
import { MessageTextComponent } from '../message-text/message-text.component';
import { ModelPickerComponent } from '../model-picker/model-picker.component';
import { RenameDialogComponent } from '../rename-dialog/rename-dialog.component';
import { ThreadHeaderComponent } from '../thread-header/thread-header.component';
import { ToolsPanelComponent } from '../tools-panel/tools-panel.component';
import { TurnComponent } from '../turn/turn.component';

const COPY = {
  latest: 'Jump to latest',
  failed: 'Something went wrong:',
  placeholder: 'Reply',
  conversation: 'Conversation',
  tools: 'Tools',
  resize: 'Resize the tools',
  hint: 'Drag or use the arrow keys to resize; Enter collapses. Show or hide:',
  trust: 'This project brings its own tools and settings',
  you: 'You',
  queued: 'Waiting to be read',
} as const;

/** A turn with nothing in it yet, for a reply that streams in before anything is recorded. */
const STARTING: Turn = {
  key: 'turn-starting',
  items: [],
  files: [],
  took: null,
  outcome: null,
  cost: null,
};

function isFirstPrompt(state: unknown): state is FirstPrompt {
  return (
    typeof state === 'object' &&
    state !== null &&
    'prompt' in state &&
    typeof state.prompt === 'string'
  );
}

/**
 * One thread: its bar across the top (title, project and where it runs, its changes); its
 * conversation in a reading column, turn by turn, results first (the reply, what changed, the
 * receipt that opens to the steps, then what waits for the person), and after them what its
 * project asks the person to trust, while it asks; the reply streaming in as
 * markdown as a turn runs, and after it what the person sent meanwhile, waiting until the
 * model reads it; the composer pinned below it, usable while a turn runs, with the approval
 * mode and the model in its quiet line; and the tools panel beside it, resized and collapsed by its edge and shown
 * or hidden by ⌘. or Ctrl+. anywhere in the thread.
 */
@Component({
  selector: 'app-thread',
  imports: [
    ApprovalModeComponent,
    GlzButtonComponent,
    GlzComposerComponent,
    GlzMessageComponent,
    GlzPanelLayoutComponent,
    GlzStickToBottomDirective,
    GlzTrustCardComponent,
    MessageTextComponent,
    ModelPickerComponent,
    RenameDialogComponent,
    ThreadHeaderComponent,
    ToolsPanelComponent,
    TurnComponent,
  ],
  providers: [ThreadService, CommandsService],
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
  /** The composer's draft, which a message the model never read comes back to. */
  protected readonly draft = this.service.composer;
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
  /** The turns, results first; the reply being written goes to the last, which runs. */
  protected readonly turns = computed<readonly TurnResults[]>(() => {
    const live = this.service.hasReply() ? this.service.reply() : null;
    const recorded = this.service.turns();
    const turns = recorded.length === 0 && live !== null ? [STARTING] : recorded;
    const last = turns.length - 1;
    return turns.map((turn, index) =>
      turnResults(turn, index === last ? live : null, index === last && this.service.busy()),
    );
  });
  protected readonly summary = computed(() => this.spaces.thread(this.thread()));
  protected readonly title = computed(() => {
    const summary = this.summary();
    return summary === undefined ? '' : threadTitle(summary);
  });
  /** The thread's own approval mode, from its summary, or `null` until it is read. */
  protected readonly mode = computed(() => this.summary()?.approval ?? null);
  /** The project the thread is in, by name; `null` for a chat. */
  protected readonly project = computed(() => {
    const space = this.spaces.spaceOf(this.thread());
    return space?.kind === 'project' ? space.name : null;
  });
  protected readonly changes = computed(() => this.service.files().length);

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

  protected send(text: string): void {
    // A pick goes with the turn only when it changes the thread's model or mode.
    const picked = this.picked();
    const model = picked !== null && picked !== this.service.chosen() ? picked : null;
    const pickedMode = this.pickedMode();
    const mode = pickedMode !== null && pickedMode !== this.mode() ? pickedMode : null;
    void this.service.send(text, model, mode);
  }

  protected stop(): void {
    void this.service.stop();
  }

  protected trust(answer: GlzTrustAnswer): void {
    void this.service.answerTrust(answer);
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
