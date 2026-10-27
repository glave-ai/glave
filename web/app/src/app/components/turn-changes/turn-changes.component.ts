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
 * changes in full, and the last row, when there are more, every one of these files'. On a narrow
 * card the paths are cut short before the counts, and after two rows a last one says how many
 * more files there are. The thread's panel shows the thread's changes the same way, under its own `heading`
 * one `level` down, with what it adds at the foot (`[appTurnChangesFoot]`).
 */
@Component({
  selector: 'app-turn-changes',
  templateUrl: './turn-changes.component.html',
  styleUrl: './turn-changes.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TurnChangesComponent {
  readonly files = input.required<readonly ChangedFile[]>();
  /** What the card is called; "What changed" by default, a turn's. */
  readonly heading = input<string | null>(null);
  /** Its heading's level: 2 in the conversation, 3 in the panel, under the panel's own. */
  readonly level = input<2 | 3>(2);
  /** The test hook; its parts get `<testId>-<part>`: `-count`, `-file`, `-more`. */
  readonly testId = input('turn-changes');

  protected readonly copy = { heading: $localize`:@@turnChanges.heading:What changed` } as const;
  protected readonly title = computed(() => this.heading() ?? this.copy.heading);
  protected readonly third = computed(() => this.level() === 3);
  protected readonly ids = computed(() => {
    const id = this.testId();
    return { count: `${id}-count`, file: `${id}-file`, more: `${id}-more` };
  });
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

  /** Shows these files' changes in full: the file at `path`, or every one's with `null`. */
  protected show(path: string | null): void {
    this.panel.showChanges(this.files(), path);
  }
}
