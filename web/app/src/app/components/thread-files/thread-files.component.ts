import { ChangeDetectionStrategy, Component, LOCALE_ID, computed, inject } from '@angular/core';
import { GlzEmptyStateComponent } from '@glave/glaze';
import { FileDiff } from 'lucide';

import { type FileChange, lineCounts } from '../../lib/files';
import { ThreadService } from '../../services/thread.service';

/**
 * The files this thread changed with its own `write` and `edit` calls, each with its diffs,
 * read from the thread's events: what each call asked for, which the tool applied exactly.
 */
@Component({
  selector: 'app-thread-files',
  imports: [GlzEmptyStateComponent],
  templateUrl: './thread-files.component.html',
  styleUrl: './thread-files.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThreadFilesComponent {
  protected readonly copy = {
    emptyHeading: $localize`:@@threadFiles.emptyHeading:No files changed`,
    emptyText: $localize`:@@threadFiles.emptyText:Files this thread writes or edits show here, with what changed.`,
    commands: $localize`:@@threadFiles.commands:Changes made by commands, or by subagents, are not listed.`,
    wrote: $localize`:@@threadFiles.wrote:Wrote the whole file`,
    edited: $localize`:@@threadFiles.edited:Edited`,
    editedAll: $localize`:@@threadFiles.editedAll:Edited every match`,
  } as const;
  protected readonly icon = FileDiff;
  private readonly thread = inject(ThreadService);
  private readonly locale = inject(LOCALE_ID);

  protected readonly files = computed(() =>
    this.thread.files().map((file) => ({
      path: file.path,
      ...lineCounts(file.added, file.removed, this.locale),
      changes: file.changes.map((change) => ({
        key: change.key,
        note: this.noteOf(change),
        lines: change.lines,
      })),
    })),
  );
  protected readonly hasFiles = computed(() => this.files().length > 0);

  private noteOf(change: FileChange): string {
    if (change.tool === 'write') {
      return this.copy.wrote;
    }
    return change.all ? this.copy.editedAll : this.copy.edited;
  }
}
