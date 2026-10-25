import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  effect,
  inject,
  input,
  linkedSignal,
  signal,
  untracked,
  viewChild,
} from '@angular/core';
import type { Permissions } from '@glave/client';
import {
  GlzAttachmentComponent,
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
import { askedTurns } from '../../lib/turns';
import { AttachmentsService } from '../../services/attachments.service';
import { effortToSend } from '../../lib/effort';
import { CommandsService } from '../../services/commands.service';
import { ModelsService } from '../../services/models.service';
import { ProjectsService } from '../../services/projects.service';
import { ThreadService } from '../../services/thread.service';
import { ToolsPanelService } from '../../services/tools-panel.service';
import { FolderGoneComponent } from '../folder-gone/folder-gone.component';
import { MessageTextComponent } from '../message-text/message-text.component';
import { EffortPickerComponent } from '../effort-picker/effort-picker.component';
import { ModelPickerComponent } from '../model-picker/model-picker.component';
import { PermissionsComponent } from '../permissions/permissions.component';
import { RenameDialogComponent } from '../rename-dialog/rename-dialog.component';
import { ThreadHeaderComponent } from '../thread-header/thread-header.component';
import { ToolsPanelComponent } from '../tools-panel/tools-panel.component';
import { TurnComponent } from '../turn/turn.component';

/**
 * One thread: its bar across the top (title, project and where it runs, its changes); its
 * conversation in a reading column, turn by turn, results first (the reply, what changed, the
 * receipt that opens to the steps, then what waits for the person), and after them what its
 * project asks the person to trust, while it asks; the reply streaming in as
 * markdown as a turn runs, and after it what the person sent meanwhile, waiting until the
 * model reads it; the composer pinned below it, usable while a turn runs, with what the agent
 * may do and the model in its quiet line, or in its place, when the project's folder isn't there
 * anymore, what the server says of it and what can be done; and the tools panel beside it, resized and collapsed by its edge and shown
 * or hidden by ⌘. or Ctrl+. anywhere in the thread.
 */
@Component({
  selector: 'app-thread',
  imports: [
    FolderGoneComponent,
    GlzAttachmentComponent,
    GlzButtonComponent,
    GlzComposerComponent,
    GlzMessageComponent,
    GlzPanelLayoutComponent,
    GlzStickToBottomDirective,
    GlzTrustCardComponent,
    MessageTextComponent,
    ModelPickerComponent,
    PermissionsComponent,
    RenameDialogComponent,
    ThreadHeaderComponent,
    ToolsPanelComponent,
    TurnComponent,
    EffortPickerComponent,
  ],
  providers: [ThreadService, CommandsService, AttachmentsService],
  templateUrl: './thread.component.html',
  styleUrl: './thread.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '(document:keydown)': 'keydown($event)',
    '(document:pointerdown)': 'pressed($event)',
    '(document:pointerup)': 'released()',
    '(document:pointercancel)': 'released()',
    '(document:contextmenu)': 'released()',
    '(document:dragstart)': 'released()',
  },
})
export class ThreadComponent {
  /** The thread's id, from the route. */
  readonly thread = input.required<string>();

  protected readonly copy = {
    latest: $localize`:@@thread.latest:Jump to latest`,
    attach: $localize`:@@thread.attach:Add files or photos`,
    placeholder: $localize`:@@thread.placeholder:Reply`,
    conversation: $localize`:@@thread.conversation:Conversation`,
    tools: $localize`:@@thread.tools:Tools`,
    resize: $localize`:@@thread.resize:Resize the tools`,
    trust: $localize`:@@thread.trust:This project brings its own tools and settings`,
    you: $localize`:@@thread.you:You`,
    queued: $localize`:@@thread.queued:Waiting to be read`,
    again: $localize`:@@thread.again:Send again`,
    lost: $localize`:@@thread.lost:This message may not have reached glave.`,
  } as const;
  protected readonly service = inject(ThreadService);
  /** The files added to the message being written, kept in the thread's project. */
  protected readonly attachments = inject(AttachmentsService);
  private readonly projects = inject(ProjectsService);
  private readonly locale = inject(LOCALE_ID);
  /** What went wrong last, in the server's words. */
  protected readonly problem = computed(() => {
    const error = this.service.error();
    return error === null
      ? null
      : $localize`:@@thread.failed:Something went wrong: ${error}:ERROR:`;
  });
  /** Whether the thread's first message, sent from the new chat, got no answer: it may go again. */
  protected readonly lost = computed(() => this.service.firstTurn()?.failed === 'lost');
  private readonly panel = inject(ToolsPanelService);
  private readonly stick = viewChild.required(GlzStickToBottomDirective);
  /** The composer's draft, which a message the model never read comes back to. */
  protected readonly draft = this.service.composer;
  protected readonly atBottom = signal(true);
  /** Whether the main button is down on the page, from its press to its release. */
  private readonly pressing = signal(false);
  /**
   * Whether "Jump to latest" shows: once the person is away from the bottom, but held as it was
   * from a press to its release. A scroll just before a press, bringing what is pressed into
   * view, would otherwise show it under the pointer before the release, which would land on it,
   * and the press would click nothing.
   */
  protected readonly latestShown = linkedSignal({
    source: () => ({ away: !this.atBottom(), pressing: this.pressing() }),
    computation: (now, before?: { readonly value: boolean }) =>
      now.pressing && before !== undefined ? before.value : now.away,
  });
  /** The model the person picked for the next turn, or `null` for the thread's own. */
  protected readonly picked = signal<string | null>(null);
  /** The preset the person picked for the next turn, or `null` for the thread's own. */
  protected readonly pickedPermissions = signal<Permissions | null>(null);
  /** How much the person picked the next turn's model reasons, or `null` for the thread's own. */
  protected readonly pickedEffort = signal<string | null>(null);
  /** The model the next turn goes to: the person's pick, else the thread's, else the default. */
  protected readonly nextModel = computed(() => this.picked() ?? this.service.chosen());
  private readonly models = inject(ModelsService);
  protected readonly renaming = signal(false);
  protected readonly panelOpen = this.panel.open;
  protected readonly panelWidth = this.panel.width;
  protected readonly panelShortcut = this.panel.ariaShortcut;
  protected readonly panelHint = $localize`:@@thread.hint:Drag or use the arrow keys to resize; Enter collapses. Show or hide: ${this.panel.shortcut}:SHORTCUT:`;
  /**
   * The turns, results first; the reply being written and the step taken now go to the last,
   * which runs. A turn the person just asked for shows at once, before the server records it,
   * and carries on as the turn it records.
   */
  protected readonly turns = computed<readonly TurnResults[]>(() => {
    const live = this.service.hasReply() ? this.service.reply() : null;
    const step = this.service.step();
    const turns = askedTurns(
      this.service.turns(),
      this.service.askedAfter(),
      this.service.starting(),
    );
    const last = turns.length - 1;
    return turns.map((turn, index) =>
      turnResults(
        turn,
        index === last ? live : null,
        index === last ? step : null,
        index === last && this.service.busy(),
        this.locale,
      ),
    );
  });
  protected readonly summary = computed(() => this.projects.thread(this.thread()));
  protected readonly title = computed(() => {
    const summary = this.summary();
    return summary === undefined ? '' : threadTitle(summary);
  });
  /** The thread's own preset, from its summary, or `null` until it is read. */
  protected readonly permissions = computed(() => this.summary()?.permissions ?? null);
  /** The project the thread is in, by name; `null` for a chat. */
  protected readonly project = computed(() => {
    const project = this.projects.projectOf(this.thread());
    return project?.kind === 'project' ? project.name : null;
  });
  protected readonly changes = computed(() => this.service.files().length);
  /** When the thread's project folder isn't there anymore: the project, and what the server says. */
  protected readonly gone = computed(() => {
    const project = this.projects.projectOf(this.thread());
    const message = project === undefined ? null : this.projects.gone(project.id);
    return project === undefined || message === null ? null : { project: project.id, message };
  });
  protected readonly folderGone = computed(() => this.gone() !== null);

  /**
   * Opens the thread the route names as the server has it, nothing picked for its next turn
   * and no files added: it only reads and follows the thread, and never sends. Opening another
   * lets go of what the last one followed.
   */
  protected readonly opening = effect(() => {
    const thread = this.thread();
    untracked(() => {
      this.picked.set(null);
      this.pickedPermissions.set(null);
      this.attachments.clear();
      this.pickedEffort.set(null);
      void this.service.open(thread);
    });
  });
  /** Files added go to the thread's project. */
  protected readonly keeping = effect(() => {
    const project = this.projects.projectOf(this.thread())?.id ?? null;
    untracked(() => {
      this.attachments.keepIn(project);
    });
  });

  protected send(text: string): void {
    // A pick goes with the turn only when it changes the thread's model or preset.
    const picked = this.picked();
    const model = picked !== null && picked !== this.service.chosen() ? picked : null;
    const pickedPermissions = this.pickedPermissions();
    const permissions =
      pickedPermissions !== null && pickedPermissions !== this.permissions()
        ? pickedPermissions
        : null;
    const next = this.nextModel() ?? this.models.default();
    const info = this.models.models().find((each) => each.id === next);
    const effort = effortToSend(this.pickedEffort(), this.service.chosenEffort(), info);
    void this.service.send(text, model, permissions, this.attachments.attached(), effort);
    this.attachments.clear();
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

  protected pressed(event: PointerEvent): void {
    if (event.button === 0) {
      this.pressing.set(true);
    }
  }

  /** The press ends: released, cancelled, or taken by a menu or a drag the browser opened. */
  protected released(): void {
    this.pressing.set(false);
  }
}
