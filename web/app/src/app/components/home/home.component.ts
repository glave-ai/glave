import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';
import { Router } from '@angular/router';
import type { ApprovalMode, ThreadStartParams } from '@glave/client';
import { GlzComposerComponent } from '@glave/glaze';

import { SessionService } from '../../services/session.service';
import { SpacesService } from '../../services/spaces.service';
import { ApprovalModeComponent } from '../approval-mode/approval-mode.component';
import { ModelPickerComponent } from '../model-picker/model-picker.component';
import { PageHeaderComponent } from '../page-header/page-header.component';

const COPY = {
  heading: 'What shall we work on?',
  in: 'In',
  placeholder: 'Ask anything, or describe a task',
  failed: 'Could not start a thread:',
} as const;

/** The navigation state that carries a new thread's first prompt (never the URL). */
export interface FirstPrompt {
  readonly prompt: string;
}

/**
 * A new chat: the composer, with the approval mode and the model picker. Sending starts a
 * thread (in the space the link names, else the first space) on the model and in the approval
 * mode picked, and opens it.
 */
@Component({
  selector: 'app-home',
  imports: [ApprovalModeComponent, GlzComposerComponent, ModelPickerComponent, PageHeaderComponent],
  templateUrl: './home.component.html',
  styleUrl: './home.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class HomeComponent {
  /** The space to start in, from the link's `?space=` (a space group's +). */
  readonly space = input<string | undefined>(undefined);

  protected readonly copy = COPY;
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
  private readonly target = computed(() => this.space() ?? this.spaces.home());
  /** The project's name, when the person chose to start in one; a plain chat has none. */
  protected readonly where = computed(() => {
    const id = this.space();
    const found = this.spaces.spaces().find(({ space }) => space.id === id)?.space;
    return found?.kind === 'project' ? `${COPY.in} ${found.name}` : null;
  });

  protected async send(prompt: string): Promise<void> {
    this.starting.set(true);
    this.error.set(null);
    try {
      const space = this.target();
      const model = this.picked();
      const approval = this.pickedMode();
      const params: ThreadStartParams = {
        ...(space === undefined ? {} : { space }),
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
