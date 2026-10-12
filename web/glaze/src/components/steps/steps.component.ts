import { ChangeDetectionStrategy, Component, computed, input, model } from '@angular/core';
import { ChevronRight } from 'lucide';

import { testIds } from '../../lib/test-ids';
import { GlzIconComponent } from '../icon/icon.component';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { summary: 'summary', steps: 'steps' } as const;

/**
 * Tool calls in a row, folded under one line that says what they came to ("Ran 4 steps"), as
 * Claude's and ChatGPT's apps fold them: opened, each call is its own `glz-receipt` line, with
 * its tool and what it acted on. A native `<details>`, so it is a disclosure: it opens by
 * click, Enter or Space, and screen readers say whether it is open.
 *
 * ```html
 * <glz-steps [summary]="'Ran 2 steps'">
 *   <glz-receipt tool="bash" summary="cargo test" />
 *   <glz-receipt tool="edit" summary="src/parse.rs" />
 * </glz-steps>
 * ```
 */
@Component({
  selector: 'glz-steps',
  imports: [GlzIconComponent],
  templateUrl: './steps.component.html',
  styleUrl: './steps.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.data-testid]': 'testId()' },
})
export class GlzStepsComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** What the steps came to, in a few words: "Ran 4 steps", "Running 2 steps". */
  readonly summary = input.required<string>();
  readonly open = model(false);

  protected readonly chevron = ChevronRight;

  protected toggled(event: Event): void {
    if (event.target instanceof HTMLDetailsElement) {
      this.open.set(event.target.open);
    }
  }
}
