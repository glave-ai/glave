import {
  ChangeDetectionStrategy,
  Component,
  effect,
  inject,
  input,
  signal,
  untracked,
} from '@angular/core';
import { DomSanitizer, type SafeResourceUrl } from '@angular/platform-browser';
import type { MadeFile } from '@glave/client';
import { GlzButtonComponent, GlzMarkdownComponent } from '@glave/glaze';

import { shownBy, whyNot } from '../../lib/made';
import { FilesService } from '../../services/files.service';
import { SessionService } from '../../services/session.service';
import { ThreadService } from '../../services/thread.service';

/**
 * A file the thread made, opened in a tab of the chat's panel: its title and Download across
 * the top, then the file. A PDF is drawn by the browser's own viewer, from the bytes read back
 * through the server (`file/read`) as an address the page made; a spreadsheet, a document or a
 * deck is the server's preview of it (`file/preview`), Markdown drawn as a reply is: sheets as
 * tables, documents and decks as their text. A file the server can't preview says so, in its
 * words, beside Download. What it read is let go of when another file opens in its place.
 */
@Component({
  selector: 'app-file-preview',
  imports: [GlzButtonComponent, GlzMarkdownComponent],
  templateUrl: './file-preview.component.html',
  styleUrl: './file-preview.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class FilePreviewComponent {
  readonly file = input.required<MadeFile>();

  private readonly files = inject(FilesService);
  private readonly session = inject(SessionService);
  private readonly threads = inject(ThreadService);
  private readonly sanitizer = inject(DomSanitizer);

  protected readonly copy = {
    download: $localize`:@@filePreview.download:Download`,
    opening: $localize`:@@filePreview.opening:Opening…`,
  } as const;
  /** A PDF's address, once read, for the browser to draw; `null` otherwise. */
  protected readonly frame = signal<SafeResourceUrl | null>(null);
  /** The server's preview, once read; `null` otherwise. */
  protected readonly markdown = signal<string | null>(null);
  /** Why the file can't be shown, or saved; `null` when nothing went wrong. */
  protected readonly failed = signal<string | null>(null);

  constructor() {
    effect((onCleanup) => {
      const thread = this.threads.thread();
      const file = this.file();
      untracked(() => {
        this.frame.set(null);
        this.markdown.set(null);
        this.failed.set(null);
      });
      if (thread === null) {
        return;
      }
      let live = true;
      const fail = (error: unknown): void => {
        if (live) {
          this.failed.set(whyNot(error));
        }
      };
      if (shownBy(file) === 'browser') {
        const held = this.files.hold(thread, file.sha256, file.media_type);
        onCleanup(() => {
          live = false;
          held.release();
        });
        held.url.then((url) => {
          if (live) {
            this.frame.set(this.sanitizer.bypassSecurityTrustResourceUrl(url));
          }
        }, fail);
        return;
      }
      onCleanup(() => {
        live = false;
      });
      this.session.call('file/preview', { thread, sha256: file.sha256 }).then(({ markdown }) => {
        if (live) {
          this.markdown.set(markdown);
        }
      }, fail);
    });
  }

  protected async download(): Promise<void> {
    const thread = this.threads.thread();
    const file = this.file();
    if (thread === null) {
      return;
    }
    try {
      await this.files.save(thread, file.sha256, file.media_type, file.name);
    } catch (error: unknown) {
      this.failed.set(whyNot(error));
    }
  }
}
