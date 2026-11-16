import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  input,
  signal,
} from '@angular/core';
import type { FileKind, MadeFile } from '@glave/client';
import { GlzButtonComponent, GlzIconButtonComponent, GlzIconComponent } from '@glave/glaze';
import { Download, File, FileSpreadsheet, FileText, FileType, Presentation } from 'lucide';

import { madeLine, whyNot } from '../../lib/made';
import { FilesService } from '../../services/files.service';
import { ThreadPanelService } from '../../services/thread-panel.service';
import { ThreadService } from '../../services/thread.service';

/** The mark of each kind of file. */
const MARKS = {
  document: FileText,
  spreadsheet: FileSpreadsheet,
  deck: Presentation,
  pdf: FileType,
  other: File,
} as const satisfies Record<FileKind, unknown>;

/**
 * A file the thread made, as one compact card after its turn: its kind's mark, its title, and
 * under it its kind, its size and where it is kept, in the server's words; Open shows it in a
 * tab of the chat's panel (full screen on a phone), and the arrow saves the real file under its
 * name. Why it couldn't be saved shows in its place under the title.
 */
@Component({
  selector: 'app-made-file',
  imports: [GlzButtonComponent, GlzIconButtonComponent, GlzIconComponent],
  templateUrl: './made-file.component.html',
  styleUrl: './made-file.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class MadeFileComponent {
  readonly file = input.required<MadeFile>();

  private readonly locale = inject(LOCALE_ID);
  private readonly files = inject(FilesService);
  private readonly panel = inject(ThreadPanelService);
  private readonly thread = inject(ThreadService);

  protected readonly copy = {
    open: $localize`:@@madeFile.open:Open`,
  } as const;
  protected readonly icons = { download: Download } as const;
  protected readonly mark = computed(() => MARKS[this.file().kind]);
  /** Why the file couldn't be saved, last time; `null` when nothing went wrong. */
  protected readonly failed = signal<string | null>(null);
  protected readonly line = computed(() => this.failed() ?? madeLine(this.file(), this.locale));
  protected readonly downloadLabel = computed(
    () => $localize`:@@madeFile.download:Download ${this.file().name}:NAME:`,
  );

  protected open(): void {
    this.panel.openFile(this.file());
  }

  protected async download(): Promise<void> {
    const thread = this.thread.thread();
    if (thread === null) {
      return;
    }
    const file = this.file();
    this.failed.set(null);
    try {
      await this.files.save(thread, file.sha256, file.media_type, file.name);
    } catch (error: unknown) {
      this.failed.set(whyNot(error));
    }
  }
}
