import { ChangeDetectionStrategy, Component, computed, input, model } from '@angular/core';
import { Ban, Check, LoaderCircle, TriangleAlert } from 'lucide';

import { type GlzMoney, formatDuration, formatMoney, formatTokens } from '../../lib/format';
import type { GlzIconData } from '../../lib/icons';
import { GlzIconComponent } from '../icon/icon.component';
import { testIds } from '../../lib/test-ids';

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

const STATUS_WORDS: Readonly<Record<GlzReceiptStatus, string>> = {
  running: 'Running',
  done: 'Done',
  failed: 'Failed',
  declined: 'Declined',
};

/**
 * A compact record of one tool call, quiet beside the conversation: one line with the tool's
 * icon, its name (the verb: `bash`, `edit`, `write`), what it acted on (the command, the
 * path), its time and cost, and its status at the end; opened, its evidence (the command and
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
  readonly durationMs = input<number | null>(null);
  readonly tokens = input<number | null>(null);
  readonly cost = input<GlzMoney | null>(null);
  readonly open = model(false);

  protected readonly statusIcon = computed(() => ICONS[this.status()]);
  protected readonly statusWord = computed(() => STATUS_WORDS[this.status()]);
  protected readonly statusClass = computed(() => `status-${this.status()}`);
  /** Time, tokens and cost, those known, joined: `4.2 s · 1.2K tokens · $0.0042`. */
  protected readonly meta = computed(() => {
    const duration = this.durationMs();
    const tokens = this.tokens();
    const cost = this.cost();
    return [
      duration === null ? '' : formatDuration(duration),
      tokens === null ? '' : `${formatTokens(tokens)} tokens`,
      cost === null ? '' : formatMoney(cost),
    ]
      .filter((part) => part !== '')
      .join(' · ');
  });

  protected toggled(event: Event): void {
    if (event.target instanceof HTMLDetailsElement) {
      this.open.set(event.target.open);
    }
  }
}
