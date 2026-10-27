import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  input,
} from '@angular/core';

import { type ChangedFile, type FileChange, lineCounts } from '../../lib/files';

/**
 * Changes made to files with a thread's own `write` and `edit` calls, in full: each of `files`
 * (a turn's, or the thread's) open on its diffs, what each call asked for, which the tool
 * applied exactly; one file's alone, given its `path`.
 */
@Component({
  selector: 'app-thread-files',
  templateUrl: './thread-files.component.html',
  styleUrl: './thread-files.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ThreadFilesComponent {
  readonly files = input.required<readonly ChangedFile[]>();
  /** The file whose changes show, or `null` for every file's. */
  readonly path = input<string | null>(null);

  protected readonly copy = {
    commands: $localize`:@@threadFiles.commands:Changes made by commands, or by subagents, are not listed.`,
    wrote: $localize`:@@threadFiles.wrote:Wrote the whole file`,
    edited: $localize`:@@threadFiles.edited:Edited`,
    editedAll: $localize`:@@threadFiles.editedAll:Edited every match`,
  } as const;
  private readonly locale = inject(LOCALE_ID);

  protected readonly shown = computed(() => {
    const path = this.path();
    const files = this.files().filter((file) => path === null || file.path === path);
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
