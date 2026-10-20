import {
  ChangeDetectionStrategy,
  Component,
  afterRenderEffect,
  computed,
  inject,
  input,
  linkedSignal,
  signal,
  viewChild,
} from '@angular/core';
import { Router } from '@angular/router';
import type { ApprovalMode, ThreadStartParams } from '@glave/client';
import {
  GlzComposerComponent,
  GlzIconComponent,
  type GlzMenuOption,
  GlzMenuSelectComponent,
} from '@glave/glaze';
import { Folder, Lock, MessageCircle } from 'lucide';

import { effortToSend } from '../../lib/effort';
import { ModelsService } from '../../services/models.service';
import { NavService } from '../../services/nav.service';
import { SessionService } from '../../services/session.service';
import { ProjectsService } from '../../services/projects.service';
import { ApprovalModeComponent } from '../approval-mode/approval-mode.component';
import { EffortPickerComponent } from '../effort-picker/effort-picker.component';
import { ModelPickerComponent } from '../model-picker/model-picker.component';
import { PageHeaderComponent } from '../page-header/page-header.component';

const COPY = {
  heading: 'What should we work on?',
  placeholder: 'Ask anything, or describe a task',
  failed: 'Could not start a thread:',
  where: 'Where',
  chat: 'A chat',
  private: 'Private on this computer. Only the model you pick sees what you send.',
} as const;

/** The navigation state that carries a new thread's first prompt (never the URL). */
export interface FirstPrompt {
  readonly prompt: string;
}

/**
 * A new chat: the question, then the composer that starts a thread, with where it goes (a
 * chat, or a project) in its box, and the approval mode and the model in the quiet line under
 * it. Sending starts the thread there,
 * on the model and in the approval mode picked, and opens it. When glave runs on this
 * computer, a line under it says what stays private. Opened for a project, the composer takes
 * focus.
 */
@Component({
  selector: 'app-home',
  imports: [
    ApprovalModeComponent,
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
})
export class HomeComponent {
  /**
   * The project to start in: the project page's (`/p/:project`, where glave run in a folder
   * opens), or the link's `?project=` (a project's +).
   */
  readonly project = input<string | undefined>(undefined);

  protected readonly copy = COPY;
  protected readonly lock = Lock;
  protected readonly draft = signal('');
  protected readonly starting = signal(false);
  protected readonly error = signal<string | null>(null);
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
  /** Whether glave runs on this computer, so what is sent stays here but for the model. */
  protected readonly local = this.session.local;
  /** Where the thread starts: the project the link names, else a chat; the person may pick. */
  protected readonly where = linkedSignal<string | null>(
    () => this.project() ?? this.projects.home() ?? null,
  );
  /** The places a thread can start: a chat, or each project. */
  protected readonly places = computed<readonly GlzMenuOption[]>(() => {
    const home = this.projects.home();
    const chat = home === undefined ? [] : [{ value: home, label: COPY.chat, icon: MessageCircle }];
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

  constructor() {
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
      const approval = this.pickedMode();
      const next = model ?? this.models.default();
      const info = this.models.models().find((each) => each.id === next);
      const effort = effortToSend(this.pickedEffort(), null, info);
      const params: ThreadStartParams = {
        ...(project === null ? {} : { project }),
        ...(model === null ? {} : { model }),
        ...(approval === null ? {} : { approval }),
        ...(effort === null ? {} : { effort }),
      };
      const { thread } = await this.session.call('thread/start', params);
      const state: FirstPrompt = { prompt };
      await this.router.navigate(['/t', thread.id], { state });
      void this.projects.refresh();
    } catch (error: unknown) {
      this.draft.set(prompt);
      this.error.set(error instanceof Error ? error.message : String(error));
    } finally {
      this.starting.set(false);
    }
  }
}
