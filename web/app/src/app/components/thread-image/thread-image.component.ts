import {
  ChangeDetectionStrategy,
  Component,
  effect,
  inject,
  input,
  signal,
  untracked,
} from '@angular/core';
import { GlzFigureComponent } from '@glave/glaze';

import type { HeldImage } from '../../lib/conversation';
import { FilesService } from '../../services/files.service';
import { ThreadService } from '../../services/thread.service';

/**
 * An image the thread holds, such as one the agent made: read back through the server once,
 * drawn in its place in the conversation, and saved under its name from the link beneath it,
 * which also names it for those who can't see it. Its address is held while it is drawn, and
 * let go of when it no longer is.
 */
@Component({
  selector: 'app-thread-image',
  imports: [GlzFigureComponent],
  templateUrl: './thread-image.component.html',
  styleUrl: './thread-image.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThreadImageComponent {
  readonly image = input.required<HeldImage>();

  private readonly files = inject(FilesService);
  private readonly threads = inject(ThreadService);
  /** Its address once read; `null` while it comes. */
  protected readonly url = signal<string | null>(null);
  /** Why it can't be shown; `null` when nothing went wrong. */
  protected readonly failed = signal<string | null>(null);

  constructor() {
    effect((onCleanup) => {
      const thread = this.threads.thread();
      const image = this.image();
      untracked(() => {
        this.url.set(null);
        this.failed.set(null);
      });
      if (thread === null) {
        return;
      }
      const held = this.files.hold(thread, image.sha256, image.mediaType);
      let live = true;
      onCleanup(() => {
        live = false;
        held.release();
      });
      held.url.then(
        (url) => {
          if (live) {
            this.url.set(url);
          }
        },
        (error: unknown) => {
          if (live) {
            this.failed.set(error instanceof Error ? error.message : String(error));
          }
        },
      );
    });
  }
}
