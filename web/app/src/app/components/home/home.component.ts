import {
  ChangeDetectionStrategy,
  Component,
  afterRenderEffect,
  computed,
  effect,
  inject,
  input,
  linkedSignal,
  signal,
  untracked,
  viewChild,
} from '@angular/core';
import { Router } from '@angular/router';
import type { ApprovalMode, ThreadId, TurnStartParams } from '@glave/client';
import {
  GlzAttachmentComponent,
  GlzComposerComponent,
  GlzIconComponent,
  type GlzMenuOption,
  GlzMenuSelectComponent,
} from '@glave/glaze';
import { Folder, Lock, MessageCircle } from 'lucide';

import { AttachmentsService } from '../../services/attachments.service';
import { effortToSend } from '../../lib/effort';
import { type Unsent, answerLost, idempotencyKey, sameSend } from '../../lib/idempotency';
import { ModelsService } from '../../services/models.service';
import { NavService } from '../../services/nav.service';
import { SessionService } from '../../services/session.service';
import { ProjectsService } from '../../services/projects.service';
import { ApprovalModeComponent } from '../approval-mode/approval-mode.component';
import { EffortPickerComponent } from '../effort-picker/effort-picker.component';
import { ModelPickerComponent } from '../model-picker/model-picker.component';
import { PageHeaderComponent } from '../page-header/page-header.component';

/**
 * A new chat: the question, then the composer that starts a thread, with where it goes (a
 * chat, or a project) in its box, and the approval mode and the model in the quiet line under
 * it. Sending starts the thread there and its first turn, on the model and in the approval mode
 * picked, and opens the thread once the server has the message, held in the box until then:
 * the thread's page only shows what the server has. When glave runs on this computer, a line
 * under it says what stays private. Opened for a project, the composer takes focus, and the page
 * has Back.
 */
@Component({
  selector: 'app-home',
  imports: [
    ApprovalModeComponent,
    GlzAttachmentComponent,
    EffortPickerComponent,
    GlzComposerComponent,
    GlzIconComponent,
    GlzMenuSelectComponent,
    ModelPickerComponent,
    PageHeaderComponent,
  ],
  templateUrl: './home.component.html',
  styleUrl: './home.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  providers: [AttachmentsService],
})
export class HomeComponent {
  /**
   * The project to start in: the project page's (`/p/:project`, where glave run in a folder
   * opens), or the link's `?project=` (a project's +).
   */
  readonly project = input<string | undefined>(undefined);

  protected readonly copy = {
    heading: $localize`:@@home.heading:What should we work on?`,
    attach: $localize`:@@home.attach:Add files or photos`,
    placeholder: $localize`:@@home.placeholder:Ask anything, or describe a task`,
    where: $localize`:@@home.where:Where`,
    chat: $localize`:@@home.chat:A chat`,
    private: $localize`:@@home.private:Private on this computer. Only the model you pick sees what you send.`,
  } as const;
  protected readonly lock = Lock;
  protected readonly draft = signal('');
  protected readonly starting = signal(false);
  /**
   * Why the thread could not start, until the next try: the server's words, or `lost` when its
   * answer never came.
   */
  protected readonly error = signal<{ readonly said: string } | 'lost' | null>(null);
  protected readonly problem = computed(() => {
    const error = this.error();
    if (error === 'lost') {
      return $localize`:@@home.lost:No answer came back. Send it again: it will not start twice.`;
    }
    return error === null
      ? null
      : $localize`:@@home.failed:Could not start a thread: ${error.said}:ERROR:`;
  });
  /** The model the person picked, or `null` for the server's default. */
  protected readonly picked = signal<string | null>(null);
  /** The approval mode the person picked, or `null` for the server's. */
  protected readonly pickedMode = signal<ApprovalMode | null>(null);
  /** How much the person picked the model reasons, or `null` for the model's own way. */
  protected readonly pickedEffort = signal<string | null>(null);
  private readonly models = inject(ModelsService);
  private readonly session = inject(SessionService);
  private readonly projects = inject(ProjectsService);
  private readonly router = inject(Router);
  /** Whether this is a project's page, which has a way back, rather than the Chats. */
  protected readonly inProject = computed(() => {
    const project = this.project();
    return project !== undefined && project !== this.projects.home();
  });
  /** Whether glave runs on this computer, so what is sent stays here but for the model. */
  protected readonly local = this.session.local;
  /** Where the thread starts: the project the link names, else a chat; the person may pick. */
  protected readonly where = linkedSignal<string | null>(
    () => this.project() ?? this.projects.home() ?? null,
  );
  /** The places a thread can start: a chat, or each project. */
  protected readonly places = computed<readonly GlzMenuOption[]>(() => {
    const home = this.projects.home();
    const chat =
      home === undefined ? [] : [{ value: home, label: this.copy.chat, icon: MessageCircle }];
    const projects = this.projects
      .projects()
      .filter(({ project }) => project.kind === 'project')
      .map(({ project }) => ({ value: project.id, label: project.name, icon: Folder }));
    return [...chat, ...projects];
  });
  private readonly narrow = inject(NavService).narrow;
  /**
   * Where it goes sits in the composer's box beside a wide page, and on a phone, where the box
   * is one row, behind its options button with the rest.
   */
  protected readonly whereInBox = computed(() => this.places().length > 0 && !this.narrow());
  protected readonly whereInStatus = computed(() => this.places().length > 0 && this.narrow());
  private readonly composer = viewChild.required(GlzComposerComponent);
  /** The files added to the first message, kept where the thread will start. */
  protected readonly attachments = inject(AttachmentsService);
  /**
   * The thread made for the first message, until the server has its turn: tried again in the
   * same place after the server said why not, the message goes there, so no empty thread is
   * left behind. With it, the last send to it, until the server has that or says why not: sent
   * again as it was after its answer was lost, it is the same send, with the same key, to that
   * thread wherever the person picks meanwhile, since the server may have it there.
   */
  private made: {
    readonly where: string | null;
    readonly thread: ThreadId;
    readonly unsent: Unsent | null;
  } | null = null;

  /** Files added go where the thread will start. */
  protected readonly keeping = effect(() => {
    const where = this.where();
    untracked(() => {
      this.attachments.keepIn(where);
    });
  });
  /**
   * A link that names a project (a project just made, a project's +) is there to start a thread
   * in it: the cursor goes to the composer.
   */
  protected readonly focusing = afterRenderEffect(() => {
    if (this.project() !== undefined) {
      this.composer().focus();
    }
  });

  protected async send(prompt: string): Promise<void> {
    // The message stays in the box, held, until the server has it: a second press meanwhile,
    // before the box shows it held, sends nothing more.
    if (this.starting()) {
      return;
    }
    this.starting.set(true);
    this.error.set(null);
    const where = this.where();
    const from = this.router.url;
    const again = sameSend(prompt, this.made?.unsent ?? null);
    const made = again !== null || this.made?.where === where ? this.made : null;
    let thread: ThreadId;
    try {
      thread = made?.thread ?? (await this.startThread(where));
      const key = again?.key ?? idempotencyKey();
      this.made = { where: made?.where ?? where, thread, unsent: { prompt, key } };
      await this.startTurn(thread, prompt, key);
      this.made = null;
    } catch (error: unknown) {
      // An answer that says why ends the send; one lost leaves it to be sent again as it was.
      const lost = answerLost(error);
      if (!lost && this.made !== null) {
        this.made = { ...this.made, unsent: null };
      }
      this.error.set(
        lost ? 'lost' : { said: error instanceof Error ? error.message : String(error) },
      );
      this.starting.set(false);
      return;
    }
    if (this.router.url === from) {
      await this.router.navigate(['/t', thread]);
    } else {
      // The person went elsewhere meanwhile: it is sent, and the box is theirs again.
      this.draft.set('');
      this.starting.set(false);
    }
  }

  /** Starts a thread in `project`. */
  private async startThread(project: string | null): Promise<ThreadId> {
    const { thread } = await this.session.call('thread/start', project === null ? {} : { project });
    return thread.id;
  }

  /**
   * Starts the thread's first turn with `prompt`, the files added and the model, mode and level
   * the person picked, and waits only for the server's first word back, which says it has the
   * message: the turn goes on in the server, and the thread's page follows it.
   */
  private async startTurn(thread: ThreadId, prompt: string, key: string): Promise<void> {
    const files = this.attachments.attached();
    const model = this.picked();
    const approval = this.pickedMode();
    const next = model ?? this.models.default();
    const info = this.models.models().find((each) => each.id === next);
    const effort = effortToSend(this.pickedEffort(), null, info);
    const params: TurnStartParams = {
      thread,
      prompt,
      ...(files.length === 0 ? {} : { files }),
      ...(model === null ? {} : { model }),
      ...(approval === null ? {} : { approval }),
      ...(effort === null ? {} : { effort }),
      _meta: { idempotency_key: key },
    };
    const controller = new AbortController();
    try {
      await this.session.stream('turn/start', params, controller.signal).next();
    } finally {
      controller.abort();
    }
  }
}
