import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  linkedSignal,
  signal,
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

import { NavService } from '../../services/nav.service';
import { SessionService } from '../../services/session.service';
import { SpacesService } from '../../services/spaces.service';
import { ApprovalModeComponent } from '../approval-mode/approval-mode.component';
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
 * computer, a line under it says what stays private.
 */
@Component({
  selector: 'app-home',
  imports: [
    ApprovalModeComponent,
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
  /** The space to start in, from the link's `?space=` (a project's +). */
  readonly space = input<string | undefined>(undefined);

  protected readonly copy = COPY;
  protected readonly lock = Lock;
  protected readonly draft = signal('');
  protected readonly starting = signal(false);
  protected readonly error = signal<string | null>(null);
  /** The model the person picked, or `null` for the server's default. */
  protected readonly picked = signal<string | null>(null);
  /** The approval mode the person picked, or `null` for the server's. */
  protected readonly pickedMode = signal<ApprovalMode | null>(null);
  private readonly session = inject(SessionService);
  private readonly spaces = inject(SpacesService);
  private readonly router = inject(Router);
  /** Whether glave runs on this computer, so what is sent stays here but for the model. */
  protected readonly local = this.session.local;
  /** Where the thread starts: the space the link names, else a chat; the person may pick. */
  protected readonly where = linkedSignal<string | null>(
    () => this.space() ?? this.spaces.home() ?? null,
  );
  /** The places a thread can start: a chat, or each project. */
  protected readonly places = computed<readonly GlzMenuOption[]>(() => {
    const home = this.spaces.home();
    const chat = home === undefined ? [] : [{ value: home, label: COPY.chat, icon: MessageCircle }];
    const projects = this.spaces
      .spaces()
      .filter(({ space }) => space.kind === 'project')
      .map(({ space }) => ({ value: space.id, label: space.name, icon: Folder }));
    return [...chat, ...projects];
  });
  private readonly narrow = inject(NavService).narrow;
  /**
   * Where it goes sits in the composer's box beside a wide page, and on a phone, where the box
   * is one row, behind its options button with the rest.
   */
  protected readonly whereInBox = computed(() => this.places().length > 0 && !this.narrow());
  protected readonly whereInStatus = computed(() => this.places().length > 0 && this.narrow());

  protected async send(prompt: string): Promise<void> {
    this.starting.set(true);
    this.error.set(null);
    try {
      const space = this.where();
      const model = this.picked();
      const approval = this.pickedMode();
      const params: ThreadStartParams = {
        ...(space === null ? {} : { space }),
        ...(model === null ? {} : { model }),
        ...(approval === null ? {} : { approval }),
      };
      const { thread } = await this.session.call('thread/start', params);
      const state: FirstPrompt = { prompt };
      await this.router.navigate(['/t', thread.id], { state });
      void this.spaces.refresh();
    } catch (error: unknown) {
      this.draft.set(prompt);
      this.error.set(error instanceof Error ? error.message : String(error));
    } finally {
      this.starting.set(false);
    }
  }
}
