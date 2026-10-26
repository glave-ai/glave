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
import type { Permissions, ThreadStartParams } from '@glave/client';
import {
  GlzAttachmentComponent,
  GlzComposerComponent,
  GlzIconComponent,
  type GlzMenuOption,
  GlzMenuSelectComponent,
} from '@glave/glaze';
import { Folder, Globe, Lock, MessageCircle } from 'lucide';

import { AttachmentsService } from '../../services/attachments.service';
import { FirstTurnService } from '../../services/first-turn.service';
import { effortToSend } from '../../lib/effort';
import { ModelsService } from '../../services/models.service';
import { NavService } from '../../services/nav.service';
import { SessionService } from '../../services/session.service';
import { ProjectsService } from '../../services/projects.service';
import { SearchService } from '../../services/search.service';
import { EffortPickerComponent } from '../effort-picker/effort-picker.component';
import { ModelPickerComponent } from '../model-picker/model-picker.component';
import { PageHeaderComponent } from '../page-header/page-header.component';
import { PermissionsComponent } from '../permissions/permissions.component';

/**
 * A new chat: the question, then the composer that starts a thread, with where it goes (a
 * chat, or a project) in its box, and what the agent may do and the model in the quiet line
 * under it. Sending starts the thread there, on the model and with the preset picked, and its
 * first turn, and opens the thread at once. When glave runs on this
 * computer, a line under it says what stays private. Opened for a project, the composer takes
 * focus, and the page has Back.
 */
@Component({
  selector: 'app-home',
  imports: [
    GlzAttachmentComponent,
    EffortPickerComponent,
    GlzComposerComponent,
    GlzIconComponent,
    GlzMenuSelectComponent,
    ModelPickerComponent,
    PageHeaderComponent,
    PermissionsComponent,
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
    searching: $localize`:@@home.searching:Web search is on`,
  } as const;
  protected readonly lock = Lock;
  protected readonly globe = Globe;
  /** Whether the agent may search the web, which the composer shows while it may. */
  protected readonly searching = inject(SearchService).on;
  protected readonly draft = signal('');
  protected readonly starting = signal(false);
  /** Why the thread could not start, as the server says, until the next try. */
  protected readonly error = signal<string | null>(null);
  protected readonly problem = computed(() => {
    const error = this.error();
    return error === null
      ? null
      : $localize`:@@home.failed:Could not start a thread: ${error}:ERROR:`;
  });
  /** The model the person picked, or `null` for the server's default. */
  protected readonly picked = signal<string | null>(null);
  /** The preset the person picked, or `null` for the server's. */
  protected readonly pickedPermissions = signal<Permissions | null>(null);
  /** How much the person picked the model reasons, or `null` for the model's own way. */
  protected readonly pickedEffort = signal<string | null>(null);
  private readonly models = inject(ModelsService);
  private readonly session = inject(SessionService);
  private readonly projects = inject(ProjectsService);
  private readonly router = inject(Router);
  private readonly firstTurns = inject(FirstTurnService);
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

  constructor() {
    effect(() => {
      const where = this.where();
      untracked(() => {
        this.attachments.keepIn(where);
      });
    });
    // A link that names a project (a project just made, a project's +) is there to start a
    // thread in it: the cursor goes to the composer.
    afterRenderEffect(() => {
      if (this.project() !== undefined) {
        this.composer().focus();
      }
    });
  }

  protected async send(prompt: string): Promise<void> {
    this.starting.set(true);
    this.error.set(null);
    try {
      const project = this.where();
      const model = this.picked();
      const permissions = this.pickedPermissions();
      const next = model ?? this.models.default();
      const info = this.models.models().find((each) => each.id === next);
      const effort = effortToSend(this.pickedEffort(), null, info);
      const params: ThreadStartParams = {
        ...(project === null ? {} : { project }),
        ...(model === null ? {} : { model }),
        ...(permissions === null ? {} : { permissions }),
        ...(effort === null ? {} : { effort }),
      };
      const { thread } = await this.session.call('thread/start', params);
      const files = this.attachments.attached();
      this.firstTurns.start({
        thread: thread.id,
        prompt,
        ...(files.length === 0 ? {} : { files }),
      });
      await this.router.navigate(['/t', thread.id]);
    } catch (error: unknown) {
      this.draft.set(prompt);
      this.error.set(error instanceof Error ? error.message : String(error));
    } finally {
      this.starting.set(false);
    }
  }
}
