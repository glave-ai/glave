import {
  ChangeDetectionStrategy,
  Component,
  type ElementRef,
  computed,
  input,
  model,
  viewChild,
} from '@angular/core';
import { Ban, Check, ChevronRight, Hand, LoaderCircle, TriangleAlert } from 'lucide';

import type { GlzIconData } from '../../lib/icons';
import { INERT_UNTIL_BOUND } from '../../lib/inert-until-bound';
import { testIds } from '../../lib/test-ids';
import { glzText } from '../../lib/words';
import { GlzIconComponent } from '../icon/icon.component';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { summary: 'summary', steps: 'steps', chevron: 'chevron', fold: 'fold' } as const;

/** Where a run of steps stands: done, failed, stopped, still going, or waiting for the person. */
export type GlzRunStatus = 'done' | 'failed' | 'stopped' | 'running' | 'waiting';

/** The words for each status, read beside its icon. */
export type GlzRunStatusLabels = Readonly<Record<GlzRunStatus, string>>;

const ICONS: Readonly<Record<GlzRunStatus, GlzIconData>> = {
  done: Check,
  failed: TriangleAlert,
  stopped: Ban,
  running: LoaderCircle,
  waiting: Hand,
};

/**
 * What a run of steps came to, as a receipt: one sunken line in monospace with its status (a
 * check when done, which only screen readers read in words; the icon and the word otherwise),
 * a few `facts` such as the time it took and its cost, and at the end how many `steps`
 * ("4 steps ›"). The line folds the steps themselves (each a `glz-receipt`, or anything else
 * they did): a native `<details>`, so it opens by click, Enter or Space, and screen readers say
 * whether it is open. The steps slide open and shut.
 *
 * ```html
 * <glz-steps status="done" [facts]="['4m 02s', '$0.04']" steps="2 steps">
 *   <glz-receipt summary="bash: cargo test" />
 *   <glz-receipt summary="edit src/parse.rs" />
 * </glz-steps>
 * ```
 */
@Component({
  selector: 'glz-steps',
  imports: [GlzIconComponent],
  templateUrl: './steps.component.html',
  styleUrl: './steps.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    ...INERT_UNTIL_BOUND,
    '[attr.data-testid]': 'testId()',
    '[class]': 'statusClass()',
  },
})
export class GlzStepsComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));
  private readonly words = glzText('steps', {
    statusLabels: {
      done: 'Done',
      failed: 'failed',
      stopped: 'stopped',
      running: 'working',
      waiting: 'waiting',
    },
  });

  readonly status = input<GlzRunStatus>('done');
  /** What else to say about the run, in a few words each: the time it took, its cost. */
  readonly facts = input<readonly string[]>([]);
  /** How many steps, in words: "4 steps". */
  readonly steps = input.required<string>();
  readonly statusLabels = input<GlzRunStatusLabels>(this.words.statusLabels);
  readonly open = model(false);

  protected readonly chevron = ChevronRight;
  protected readonly icon = computed(() => ICONS[this.status()]);
  protected readonly label = computed(() => this.statusLabels()[this.status()]);
  /** A check says done on its own; any other status says itself in words too. */
  protected readonly quiet = computed(() => this.status() === 'done');
  protected readonly statusClass = computed(() => `status-${this.status()}`);
  /** A running run's ring turns (`glz-spin`), and holds still when the person asks for less motion. */
  protected readonly running = computed(() => this.status() === 'running');

  private readonly details = viewChild.required<ElementRef<HTMLDetailsElement>>('details');
  private readonly fold = viewChild.required<ElementRef<HTMLElement>>('fold');
  /** Whether the steps are sliding shut, before the details close. */
  private shutting = false;

  protected toggled(event: Event): void {
    if (event.target instanceof HTMLDetailsElement) {
      this.open.set(event.target.open);
    }
  }

  /**
   * Shuts the steps once they have slid shut (`shutting`, in the motion tokens' time), so the
   * details hide what they hold only then; at once when the person asks for less motion, where
   * nothing slides. They open as the browser opens them, the fold growing as they do.
   */
  protected shut(event: Event): void {
    const details = this.details().nativeElement;
    if (!details.open) {
      return;
    }
    event.preventDefault();
    if (this.shutting) {
      return;
    }
    details.classList.add('shutting');
    const sliding = this.fold().nativeElement.getAnimations();
    if (sliding.length === 0) {
      this.close(details);
      return;
    }
    this.shutting = true;
    void Promise.all(sliding.map((each) => each.finished.catch(() => undefined))).then(() => {
      this.shutting = false;
      this.close(details);
    });
  }

  private close(details: HTMLDetailsElement): void {
    details.open = false;
    details.classList.remove('shutting');
  }
}
