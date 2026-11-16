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
import { GlzButtonComponent, GlzIconButtonComponent, GlzIconComponent } from '@glave/glaze';
import { Download, File, FileSpreadsheet, FileText, FileType, Image, Presentation } from 'lucide';

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
  picture: Image,
  other: File,
} as const satisfies Record<AssetKind, unknown>;

/**
 * A file the thread made, as one compact card after its turn: its kind's mark, its name (the
 * server's), and under it its kind, its pages, sheets or slides (else its size) and where it is
 * kept, in the server's words. Open, whose label names the file, shows it in a tab of the chat's
 * panel (on a phone the whole card opens it), and a PDF in a tab of the browser's own, whose
 * viewer pages through it. The arrow saves the real file under its name. A file the server
 * doesn't preview only downloads, and one that was not kept does neither: its line says why. The
 * card of the file open in the panel is marked. Why it couldn't be opened or saved shows in its
 * line.
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
  /** Why the file couldn't be opened or saved, last time; `null` when nothing went wrong. */
  protected readonly failed = signal<string | null>(null);
  protected readonly line = computed(() => this.failed() ?? madeLine(this.file(), this.locale));
  /** Whether it was kept, so it can be saved. */
  protected readonly kept = computed(() => this.file().sha256 != null);
  /** Whether it opens: kept, and previewed by the server or drawn by the browser. */
  protected readonly opens = computed(() => this.kept() && this.file().previews);
  /** Whether it is the file open in the chat's panel now. */
  protected readonly selected = computed(() => {
    const open = this.panel.file();
    return this.panel.visible() && open !== null && open.sha256 === this.file().sha256;
  });
  protected readonly openLabel = computed(
    () => $localize`:@@madeFile.openNamed:Open ${this.file().name}:NAME:`,
  );
  protected readonly downloadLabel = computed(
    () => $localize`:@@madeFile.download:Download ${this.file().name}:NAME:`,
  );

  protected async open(): Promise<void> {
    const file = this.file();
    const thread = this.thread.thread();
    if (file.kind === 'pdf' && thread !== null && file.sha256 != null) {
      this.failed.set(null);
      try {
        await this.files.openInTab(thread, file.sha256, file.media_type);
      } catch (error: unknown) {
        this.failed.set(whyNot(error));
      }
      return;
    }
    this.panel.openFile(file);
  }

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
