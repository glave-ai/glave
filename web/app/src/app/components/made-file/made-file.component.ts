import { LiveAnnouncer } from '@angular/cdk/a11y';
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

import { madeLine, shareOf, whyNot } from '../../lib/made';
import { FilesService } from '../../services/files.service';
import { MadeFilesService } from '../../services/made-files.service';
import { ServerService } from '../../services/server.service';
import { ThreadPanelService } from '../../services/thread-panel.service';
import { ThreadService } from '../../services/thread.service';
import { ShareDialogComponent } from '../share-dialog/share-dialog.component';

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
 * kept, or why it only downloads, in the server's words. Open, whose label names the file, shows
 * it in a tab of the chat's panel (on a phone the whole card opens it), and a PDF in a tab of the
 * browser's own, whose viewer pages through it. The arrow saves the real file under its name,
 * one save at a time. A file the server doesn't preview only downloads, and one that was not kept
 * does neither: its line says why. The card of the file open in the panel is marked. Why it
 * couldn't be opened or saved shows in its line, and is said aloud. A line that gives a reason
 * wraps, and keeps the room of its lines whatever it says, so a card's room before the server
 * describes it is as tall as the card.
 */
@Component({
  selector: 'app-made-file',
  imports: [GlzButtonComponent, GlzIconButtonComponent, GlzIconComponent, ShareDialogComponent],
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
  private readonly announcer = inject(LiveAnnouncer);
  private readonly server = inject(ServerService);
  private readonly made = inject(MadeFilesService);

  protected readonly copy = {
    open: $localize`:@@madeFile.open:Open`,
    share: $localize`:@@madeFile.share:Share`,
  } as const;
  protected readonly icons = { download: Download } as const;
  protected readonly mark = computed(() => MARKS[this.file().kind]);
  /** Why the file couldn't be opened or saved, last time; `null` when nothing went wrong. */
  protected readonly failed = signal<string | null>(null);
  /** Whether it is being read to be saved now. */
  protected readonly saving = signal(false);
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
  /** Whether its line gives a reason: why it wasn't kept or read, or why it couldn't be saved. */
  protected readonly reason = computed(
    () => this.failed() !== null || !this.kept() || this.file().unread != null,
  );
  protected readonly busy = computed(() => (this.saving() ? 'true' : null));
  /** The file as sharing names it, where the server shares files and it was kept. */
  protected readonly shareOf = computed(() =>
    this.server.sharing() ? shareOf(this.thread.thread(), this.file()) : null,
  );
  protected readonly shareLabel = computed(
    () => $localize`:@@madeFile.shareNamed:Share ${this.file().name}:NAME:`,
  );
  /** Whether its Share dialog is open. */
  protected readonly sharing = signal(false);
  /** What the open Share dialog shares: only while it is open, so a chat holds one at most. */
  protected readonly sharedOf = computed(() => (this.sharing() ? this.shareOf() : null));
  protected readonly downloadLabel = computed(
    () => $localize`:@@madeFile.download:Download ${this.file().name}:NAME:`,
  );

  protected async open(): Promise<void> {
    const file = this.file();
    const thread = this.thread.thread();
    if (file.kind === 'pdf' && thread !== null && file.sha256 != null) {
      this.failed.set(null);
      try {
        await this.files.openInTab(thread, file.sha256, file.media_type, file.file_name);
      } catch (error: unknown) {
        const why = whyNot(error);
        this.failed.set(why);
        void this.announcer.announce(why);
      }
      return;
    }
    this.panel.openFile(file);
  }

  protected share(): void {
    this.sharing.set(true);
  }

  /** Reads the files again once the share changed, so the card says who it is shared with. */
  protected shared(): void {
    void this.made.refresh();
  }

  protected async download(): Promise<void> {
    const thread = this.thread.thread();
    const file = this.file();
    if (thread === null || file.sha256 == null || this.saving()) {
      return;
    }
    this.failed.set(null);
    this.saving.set(true);
    try {
      await this.files.save(thread, file.sha256, file.media_type, file.file_name);
    } catch (error: unknown) {
      const why = whyNot(error);
      this.failed.set(why);
      void this.announcer.announce(why);
    } finally {
      this.saving.set(false);
    }
  }
}
