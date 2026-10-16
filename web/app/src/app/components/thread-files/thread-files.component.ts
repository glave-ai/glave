import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { GlzEmptyStateComponent } from '@glave/glaze';
import { FileDiff } from 'lucide';

import type { FileChange } from '../../lib/files';
import { ThreadService } from '../../services/thread.service';

const COPY = {
  emptyHeading: 'No files changed',
  emptyText: 'Files this thread writes or edits show here, with what changed.',
  commands: 'Changes made by commands, or by subagents, are not listed.',
  wrote: 'Wrote the whole file',
  edited: 'Edited',
  editedAll: 'Edited every match',
  added: 'added',
  removed: 'removed',
} as const;

function noteOf(change: FileChange): string {
  if (change.tool === 'write') {
    return COPY.wrote;
  }
  return change.all ? COPY.editedAll : COPY.edited;
}

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
  protected readonly copy = COPY;
  protected readonly icon = FileDiff;
  private readonly thread = inject(ThreadService);

  protected readonly files = computed(() =>
    this.thread.files().map((file) => ({
      path: file.path,
      added: `+${String(file.added)}`,
      removed: `−${String(file.removed)}`,
      countsLabel: `${String(file.added)} ${COPY.added}, ${String(file.removed)} ${COPY.removed}`,
      changes: file.changes.map((change) => ({
        key: change.key,
        note: noteOf(change),
        lines: change.lines,
      })),
    })),
  );
  protected readonly hasFiles = computed(() => this.files().length > 0);
}
