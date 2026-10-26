import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  input,
} from '@angular/core';
import { injectId } from '@glave/glaze';

import { type ChangedFile, lineCounts } from '../../lib/files';
import { plural } from '../../lib/plural';
import { ThreadPanelService } from '../../services/thread-panel.service';

/** How many rows a narrow card shows before it says how many more there are. */
const NARROW_ROWS = 2;

/**
 * What a turn changed, after its reply: a card with how many files and lines added and removed
 * (each in its colour, the words read by screen readers), then a row for each file, its path in
 * monospace, cut short at the end when it is long, and its own counts. A row opens that file's
 * changes in full, and the last row, when there are more, every file's. On a narrow card the counts give way to the paths, and after
 * two rows a last one says how many more files there are.
 */
@Component({
  selector: 'app-turn-changes',
  templateUrl: './turn-changes.component.html',
  styleUrl: './turn-changes.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TurnChangesComponent {
  readonly files = input.required<readonly ChangedFile[]>();

  protected readonly copy = { heading: $localize`:@@turnChanges.heading:What changed` } as const;
  protected readonly headingId = injectId('app-turn-changes');
  private readonly panel = inject(ThreadPanelService);
  private readonly locale = inject(LOCALE_ID);
  protected readonly count = computed(() => {
    const count = this.files().length;
    return plural(
      $localize`:@@turnChanges.files:{${count}:COUNT:, plural, one {# file} other {# files}}`,
      this.locale,
    );
  });
  protected readonly total = computed(() =>
    lineCounts(
      this.files().reduce((sum, file) => sum + file.added, 0),
      this.files().reduce((sum, file) => sum + file.removed, 0),
      this.locale,
    ),
  );
  protected readonly rows = computed(() =>
    this.files().map((file) => ({
      path: file.path,
      ...lineCounts(file.added, file.removed, this.locale),
    })),
  );
  /** What the last row of a narrow card says: how many more files, if any. */
  protected readonly more = computed(() => {
    const hidden = this.files().length - NARROW_ROWS;
    return hidden > 0
      ? plural(
          $localize`:@@turnChanges.more:{${hidden}:COUNT:, plural, one {# more file} other {# more files}}`,
          this.locale,
        )
      : null;
  });

  /** Shows the changes in full: the file at `path`, or every file's with `null`. */
  protected show(path: string | null): void {
    this.panel.showChanges(path);
  }
}
