import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  input,
  signal,
} from '@angular/core';
import type { AssetKind, MadeFile } from '@glave/client';
import { GlzIconButtonComponent, GlzIconComponent } from '@glave/glaze';
import { Download, File, FileSpreadsheet, FileText, FileType, Image, Presentation } from 'lucide';

import { madeLine, whyNot } from '../../lib/made';
import { FilesService } from '../../services/files.service';
import { ThreadService } from '../../services/thread.service';

/** The mark of each kind of file. */
const MARKS = {
  document: FileText,
  spreadsheet: FileSpreadsheet,
  deck: Presentation,
  pdf: FileType,
  picture: Image,
  other: File,
} as const satisfies Record<AssetKind, unknown>;

/**
 * A file the thread made, as one compact card after its turn: its kind's mark, its name (the
 * server's), and under it its kind, its pages, sheets or slides (else its size) and where it is
 * kept, or why it only downloads, in the server's words. The arrow saves the real file under its
 * name; a file that was not kept has none, and its line says why. Why it couldn't be saved shows
 * in its line.
 */
@Component({
  selector: 'app-made-file',
  imports: [GlzIconButtonComponent, GlzIconComponent],
  templateUrl: './made-file.component.html',
  styleUrl: './made-file.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class MadeFileComponent {
  readonly file = input.required<MadeFile>();

  private readonly locale = inject(LOCALE_ID);
  private readonly files = inject(FilesService);
  private readonly thread = inject(ThreadService);

  protected readonly icons = { download: Download } as const;
  protected readonly mark = computed(() => MARKS[this.file().kind]);
  /** Why the file couldn't be saved, last time; `null` when nothing went wrong. */
  protected readonly failed = signal<string | null>(null);
  protected readonly line = computed(() => this.failed() ?? madeLine(this.file(), this.locale));
  /** Whether it was kept, so it can be saved. */
  protected readonly kept = computed(() => this.file().sha256 != null);
  protected readonly downloadLabel = computed(
    () => $localize`:@@madeFile.download:Download ${this.file().name}:NAME:`,
  );

  protected async download(): Promise<void> {
    const thread = this.thread.thread();
    const file = this.file();
    if (thread === null || file.sha256 == null) {
      return;
    }
    this.failed.set(null);
    try {
      await this.files.save(thread, file.sha256, file.media_type, file.file_name);
    } catch (error: unknown) {
      this.failed.set(whyNot(error));
    }
  }
}
