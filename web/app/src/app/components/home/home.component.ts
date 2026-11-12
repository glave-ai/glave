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
import type { Copies, Permissions, ProjectId, ThreadStartParams } from '@glave/client';
import {
  GlzAttachmentComponent,
  GlzCheckboxComponent,
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

/** What the server said of a new chat's copies in a project (`project/get`). */
interface Offered {
  readonly project: ProjectId;
  /** Whether one may work on a separate copy, and does unless told; `null` when it may not. */
  readonly copies: Copies | null;
}

/**
 * A new chat: the question, then the composer that starts a thread, with where it goes (a
 * chat, or a project) in its box, and what the agent may do and the model in the quiet line
 * under it; under that, in a project the server says may give a chat a separate copy of its
 * folder, a checkbox to work on one, checked as the server would start it, its room kept unseen
 * in any project until the server says. Sending starts the
 * thread there, on the model and with the mode picked, on a copy or not, and its first turn,
 * and opens the thread at once. When glave runs on this computer, a line under it says what
 * stays private. Opened for a project, the composer takes focus, and the page has Back.
 */
@Component({
  selector: 'app-home',
  imports: [
    GlzAttachmentComponent,
    GlzCheckboxComponent,
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
    separate: $localize`:@@home.separate:Work on a separate copy`,
    separateHint: $localize`:@@home.separateHint:so it doesn't touch your folder or other chats`,
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
      : $localize`:@@home.failed:Could not start a chat: ${error}:ERROR:`;
  });
  /** The model the person picked, or `null` for the server's default. */
  protected readonly picked = signal<string | null>(null);
  /** The mode the person picked, or `null` for the server's. */
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
  /** The project of a folder's the thread would start in, if it starts in one. */
  private readonly inFolder = computed(() => {
    const where = this.where();
    return where !== null && this.projects.project(where)?.kind === 'project' ? where : null;
  });
  private readonly offered = signal<Offered | null>(null);
  /** Counts the reads of what is offered, so an answer a later one overtook is dropped. */
  private asked = 0;
  /** Whether a new chat where it starts may work on a separate copy, and does unless told. */
  protected readonly copies = computed(() => {
    const offered = this.offered();
    return offered !== null && offered.project === this.inFolder() ? offered.copies : null;
  });
  /**
   * Whether it would start in a project, where the choice keeps its room: from the first draw
   * of a project's page, before the projects are read.
   */
  protected readonly inProjectFolder = computed(() => {
    const where = this.where();
    return where !== null && where !== this.projects.home();
  });
  /** Whether no copy is offered there, or not yet known to be. */
  protected readonly unoffered = computed(() => this.copies() === null);
  /** Whether it works on one: as the server would start it, until the person picks. */
  protected readonly separate = linkedSignal(() => this.copies()?.by_default ?? false);
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
    // What the server says of a new chat's copies in the project it would start in.
    effect(() => {
      const project = this.inFolder();
      if (project !== null) {
        untracked(() => void this.readCopies(project));
      }
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
        ...(this.copies() === null ? {} : { worktree: this.separate() }),
        ...(model === null ? {} : { model }),
        ...(permissions === null ? {} : { permissions }),
        ...(effort === undefined ? {} : { effort }),
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

  /** Reads what `project/get` says of a new chat's copies in `project`. */
  private async readCopies(project: ProjectId): Promise<void> {
    this.asked += 1;
    const asked = this.asked;
    try {
      const { copies } = await this.session.call('project/get', { project });
      if (asked === this.asked) {
        this.offered.set({ project, copies: copies ?? null });
      }
    } catch {
      // Unread, nothing is offered: the thread starts as the server would start it.
    }
  }
}
