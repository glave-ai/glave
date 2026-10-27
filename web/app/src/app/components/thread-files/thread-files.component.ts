import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  input,
} from '@angular/core';
import type { ChangeKind, ChangedFile } from '@glave/client';

import { SIGNS, lineCounts } from '../../lib/files';

/**
 * Changes made to files, in full, as the server says them: each of `files` (a turn's, or the
 * thread's) open on its diffs, what each call asked for, which the tool applied exactly; one
 * file's alone, given its `path`; and under them the server's `note` on what they leave out.
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
  /** What the files leave out, in the server's sentence. */
  readonly note = input('');

  private readonly kinds: Readonly<Record<ChangeKind, string>> = {
    wrote: $localize`:@@threadFiles.wrote:Wrote the whole file`,
    edited: $localize`:@@threadFiles.edited:Edited`,
    edited_every: $localize`:@@threadFiles.editedAll:Edited every match`,
  };
  private readonly locale = inject(LOCALE_ID);

  protected readonly shown = computed(() => {
    const path = this.path();
    const files = this.files().filter((file) => path === null || file.path === path);
    return files.map((file) => ({
      path: file.path,
      ...lineCounts(file.added, file.removed, this.locale),
      changes: file.changes.map((change, index) => ({
        key: `${file.path}-${String(index)}`,
        note: this.kinds[change.kind],
        lines: change.lines.map((line) => ({ ...line, sign: SIGNS[line.kind] })),
      })),
    }));
  });
}
