import {
  ChangeDetectionStrategy,
  Component,
  DOCUMENT,
  computed,
  effect,
  inject,
  input,
  signal,
  untracked,
} from '@angular/core';
import type { MadeFile } from '@glave/client';
import { GlzButtonComponent, GlzMarkdownComponent } from '@glave/glaze';

import { whyNot } from '../../lib/made';
import { FilesService } from '../../services/files.service';
import { SessionService } from '../../services/session.service';
import { ThreadService } from '../../services/thread.service';

/**
 * A file the thread made, opened in a tab of the chat's panel (the sheet that fills a phone's
 * screen): its name, then the server's preview of it (`file/preview`), Markdown drawn as a
 * reply is, with the server's sentence when it leaves the rest out, and Download across the top
 * beside a computer's panel and Download and Share at the thumb on a phone (Share where the
 * browser can share the file itself, the file read as it opens, so the tap that shares it goes
 * straight to the Share sheet, as iOS asks). A PDF never opens here: the browser's own viewer
 * draws it in a tab of its own. Why it can't be shown is said in the server's words.
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
  private readonly document = inject(DOCUMENT);

  protected readonly copy = {
    download: $localize`:@@filePreview.download:Download`,
    share: $localize`:@@filePreview.share:Share`,
    opening: $localize`:@@filePreview.opening:Opening…`,
  } as const;
  /** The server's preview, once read; `null` otherwise. */
  protected readonly markdown = signal<string | null>(null);
  /** What the preview leaves out, in the server's sentence; `null` when it is whole. */
  protected readonly cut = signal<string | null>(null);
  /** Why the file can't be shown, shared or saved; `null` when nothing went wrong. */
  protected readonly failed = signal<string | null>(null);
  /** The file itself, read for the Share sheet where the browser shares files; `null` until. */
  protected readonly shared = signal<File | null>(null);
  /** Whether this browser can share the file itself, as a phone's Share sheet does. */
  protected readonly shareable = computed(() => {
    const file = this.file();
    const navigator = this.document.defaultView?.navigator;
    // Not every browser has it, whatever its types say: Firefox on a computer has none.
    if (navigator === undefined || !('canShare' in navigator)) {
      return false;
    }
    const probe = new File([], file.file_name, { type: file.media_type });
    return navigator.canShare({ files: [probe] });
  });

  constructor() {
    effect((onCleanup) => {
      const thread = this.threads.thread();
      const file = this.file();
      const shareable = this.shareable();
      untracked(() => {
        this.markdown.set(null);
        this.cut.set(null);
        this.failed.set(null);
        this.shared.set(null);
      });
      if (thread === null || file.sha256 == null) {
        return;
      }
      let live = true;
      onCleanup(() => {
        live = false;
      });
      if (shareable) {
        this.files.blob(thread, file.sha256, file.media_type).then(
          (blob) => {
            if (live) {
              this.shared.set(new File([blob], file.file_name, { type: file.media_type }));
            }
          },
          // Share stays away; Download says why when it is tapped.
          () => undefined,
        );
      }
      this.session.call('file/preview', { thread, sha256: file.sha256 }).then(
        ({ preview }) => {
          if (live) {
            this.markdown.set(preview.markdown);
            this.cut.set(preview.cut ?? null);
          }
        },
        (error: unknown) => {
          if (live) {
            this.failed.set(whyNot(error));
          }
        },
      );
    });
  }

  protected async download(): Promise<void> {
    const thread = this.threads.thread();
    const file = this.file();
    if (thread === null || file.sha256 == null) {
      return;
    }
    try {
      await this.files.save(thread, file.sha256, file.media_type, file.file_name);
    } catch (error: unknown) {
      this.failed.set(whyNot(error));
    }
  }

  /**
   * Hands the file itself, read as it opened, to the phone's Share sheet, straight from the tap;
   * nothing when the person closes it.
   */
  protected async share(): Promise<void> {
    const shared = this.shared();
    const navigator = this.document.defaultView?.navigator;
    if (shared === null || navigator === undefined) {
      return;
    }
    try {
      await navigator.share({ files: [shared], title: this.file().name });
    } catch (error: unknown) {
      // Closing the Share sheet is the person's choice, not a failure.
      if (!(error instanceof DOMException && error.name === 'AbortError')) {
        this.failed.set(whyNot(error));
      }
    }
  }
}
