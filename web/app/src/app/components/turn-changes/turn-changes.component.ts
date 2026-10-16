import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import { injectId } from '@glave/glaze';

import type { ChangedFile } from '../../lib/files';
import { ToolsPanelService } from '../../services/tools-panel.service';

const COPY = {
  heading: 'What changed',
  added: 'added',
  removed: 'removed',
  lines: 'lines',
} as const;

/** How many rows a narrow card shows before it says how many more there are. */
const NARROW_ROWS = 2;

function files(n: number, what = ''): string {
  return `${String(n)} ${what}${n === 1 ? 'file' : 'files'}`;
}

/** Lines added and removed, as the card shows them and as screen readers read them. */
function counts(
  added: number,
  removed: number,
): {
  readonly added: string;
  readonly removed: string;
  readonly label: string;
} {
  return {
    added: `+${String(added)}`,
    removed: `−${String(removed)}`,
    label: `${String(added)} ${COPY.lines} ${COPY.added}, ${String(removed)} ${COPY.removed}`,
  };
}

/**
 * What a turn changed, after its reply: a card with how many files and lines added and removed
 * (each in its colour, the words read by screen readers), then a row for each file, its path in
 * monospace, cut short at the end when it is long, and its own counts. A row opens the tools
 * panel on the thread's changes. On a narrow card the counts give way to the paths, and after
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

  protected readonly copy = COPY;
  protected readonly headingId = injectId('app-turn-changes');
  private readonly panel = inject(ToolsPanelService);
  protected readonly count = computed(() => files(this.files().length));
  protected readonly total = computed(() =>
    counts(
      this.files().reduce((sum, file) => sum + file.added, 0),
      this.files().reduce((sum, file) => sum + file.removed, 0),
    ),
  );
  protected readonly rows = computed(() =>
    this.files().map((file) => ({ path: file.path, ...counts(file.added, file.removed) })),
  );
  /** What the last row of a narrow card says: how many more files, if any. */
  protected readonly more = computed(() => {
    const hidden = this.files().length - NARROW_ROWS;
    return hidden > 0 ? files(hidden, 'more ') : null;
  });

  protected show(): void {
    this.panel.showChanges();
  }
}
