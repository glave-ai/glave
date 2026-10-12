import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import { type GlzComposed, GlzComposerComponent } from '@glave/glaze';

import { SessionService } from '../../services/session.service';
import { SpacesService } from '../../services/spaces.service';

const COPY = {
  heading: 'What shall we work on?',
  placeholder: 'Ask anything, or describe a task',
  failed: 'Could not start a thread:',
} as const;

/** The navigation state that carries a new thread's first prompt (never the URL). */
export interface FirstPrompt {
  readonly prompt: string;
}

/** A new chat: the composer; sending starts a thread in the first space and opens it. */
@Component({
  selector: 'app-home',
  imports: [GlzComposerComponent],
  templateUrl: './home.component.html',
  styleUrl: './home.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class HomeComponent {
  protected readonly copy = COPY;
  protected readonly draft = signal('');
  protected readonly starting = signal(false);
  protected readonly error = signal<string | null>(null);
  private readonly session = inject(SessionService);
  private readonly spaces = inject(SpacesService);
  private readonly router = inject(Router);

  protected async send(composed: GlzComposed): Promise<void> {
    this.starting.set(true);
    this.error.set(null);
    try {
      const space = this.spaces.home();
      const { thread } = await this.session.call(
        'thread/start',
        space === undefined ? {} : { space },
      );
      const state: FirstPrompt = { prompt: composed.text };
      await this.router.navigate(['/t', thread.id], { state });
      void this.spaces.refresh();
    } catch (error: unknown) {
      this.draft.set(composed.text);
      this.error.set(error instanceof Error ? error.message : String(error));
    } finally {
      this.starting.set(false);
    }
  }
}
