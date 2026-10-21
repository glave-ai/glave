import { ChangeDetectionStrategy, Component, computed, input, model } from '@angular/core';
import { Ban, Check, LoaderCircle, TriangleAlert } from 'lucide';

import type { GlzIconData } from '../../lib/icons';
import { GlzIconComponent } from '../icon/icon.component';
import { testIds } from '../../lib/test-ids';
import { glzText } from '../../lib/words';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { summary: 'summary', evidence: 'evidence' } as const;

/** Where a tool call stands. */
export type GlzReceiptStatus = 'running' | 'done' | 'failed' | 'declined';

const ICONS: Readonly<Record<GlzReceiptStatus, GlzIconData>> = {
  running: LoaderCircle,
  done: Check,
  failed: TriangleAlert,
  declined: Ban,
};

/**
 * A compact record of one tool call, quiet beside the conversation: one line with the tool's
 * icon, its name (the verb: `bash`, `edit`, `write`), what it acted on (the command, the
 * path), and its status at the end; opened, its evidence (the command and
 * its output, the diff, the page read). A native `<details>`, so it opens by click or keyboard
 * and screen readers say its state. Several in a row group under `glz-steps`.
 *
 * ```html
 * <glz-receipt [icon]="terminal" tool="bash" summary="cargo test" status="done">
 *   <pre>test result: ok. 112 passed</pre>
 * </glz-receipt>
 * ```
 */
@Component({
  selector: 'glz-receipt',
  imports: [GlzIconComponent],
  templateUrl: './receipt.component.html',
  styleUrl: './receipt.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[attr.data-testid]': 'testId()',
    '[class]': 'statusClass()',
  },
})
export class GlzReceiptComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** What it did, in a few words: the command, the file, the search. */
  readonly summary = input.required<string>();
  /** The tool's name. */
  readonly tool = input('');
  /** The tool's icon, before its name: a terminal for a command, a pen for an edit. */
  readonly icon = input<GlzIconData | null>(null);
  readonly status = input<GlzReceiptStatus>('done');
  readonly open = model(false);

  protected readonly statusIcon = computed(() => ICONS[this.status()]);
  private readonly words = glzText('receipt', {
    statusLabels: { running: 'Running', done: 'Done', failed: 'Failed', declined: 'Declined' },
  });
  protected readonly statusWord = computed(() => this.words.statusLabels[this.status()]);
  protected readonly statusClass = computed(() => `status-${this.status()}`);

  protected toggled(event: Event): void {
    if (event.target instanceof HTMLDetailsElement) {
      this.open.set(event.target.open);
    }
  }
}
