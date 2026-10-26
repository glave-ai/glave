import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  input,
} from '@angular/core';

import { type FileChange, lineCounts } from '../../lib/files';
import { ThreadService } from '../../services/thread.service';

/**
 * The changes this thread made to files with its own `write` and `edit` calls, in full: each
 * file open on its diffs, read from the thread's events (what each call asked for, which the
 * tool applied exactly); one file's alone, given its `path`.
 */
@Component({
  selector: 'app-thread-files',
  templateUrl: './thread-files.component.html',
  styleUrl: './thread-files.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThreadFilesComponent {
  /** The file whose changes show, or `null` for every file's. */
  readonly path = input<string | null>(null);

  protected readonly copy = {
    commands: $localize`:@@threadFiles.commands:Changes made by commands, or by subagents, are not listed.`,
    wrote: $localize`:@@threadFiles.wrote:Wrote the whole file`,
    edited: $localize`:@@threadFiles.edited:Edited`,
    editedAll: $localize`:@@threadFiles.editedAll:Edited every match`,
  } as const;
  private readonly thread = inject(ThreadService);
  private readonly locale = inject(LOCALE_ID);

  protected readonly files = computed(() => {
    const path = this.path();
    const files = this.thread.files().filter((file) => path === null || file.path === path);
    return files.map((file) => ({
      path: file.path,
      ...lineCounts(file.added, file.removed, this.locale),
      changes: file.changes.map((change) => ({
        key: change.key,
        note: this.noteOf(change),
        lines: change.lines,
      })),
    }));
  });

  private noteOf(change: FileChange): string {
    if (change.tool === 'write') {
      return this.copy.wrote;
    }
    return change.all ? this.copy.editedAll : this.copy.edited;
  }
}
