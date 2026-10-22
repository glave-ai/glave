import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  input,
  linkedSignal,
} from '@angular/core';
import {
  GlzMarkdownComponent,
  GlzMessageComponent,
  GlzReasoningComponent,
  GlzStepsComponent,
  GlzWorkingComponent,
  formatDuration,
} from '@glave/glaze';

import { trimBlankLines } from '../../lib/conversation';
import type { TurnResults } from '../../lib/results';
import { runningFor } from '../../lib/when';
import { clock } from '../../services/clock';
import { ThreadItemComponent } from '../thread-item/thread-item.component';
import { TurnChangesComponent } from '../turn-changes/turn-changes.component';

/**
 * One turn, results first: the person's message; the reply (streaming in as markdown while it
 * is written); what changed; the receipt, a sunken line with how it went, how long it took and
 * what it cost, which opens to the steps; and last, what waits for the person. A turn that did
 * no work is its messages in order.
 *
 * While the turn works with nothing of its own to show, a line stands in the reply's place: the
 * step a tool takes, in the server's words, else a smithy's word while the model thinks, the next
 * every couple of seconds; and beside it how long the turn has worked, counted from the start
 * the server recorded. The model's reasoning, as it streams, shows open under it, and folds away
 * once the answer begins; the person can open or close it meanwhile. As the person watches, a
 * reply rises into place as it begins, and what changed settles in as it appears.
 */
@Component({
  selector: 'app-turn',
  imports: [
    GlzMarkdownComponent,
    GlzMessageComponent,
    GlzReasoningComponent,
    GlzStepsComponent,
    GlzWorkingComponent,
    ThreadItemComponent,
    TurnChangesComponent,
  ],
  templateUrl: './turn.component.html',
  styleUrl: './turn.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TurnComponent {
  readonly turn = input.required<TurnResults>();
  private readonly locale = inject(LOCALE_ID);

  protected readonly copy = {
    reply: $localize`:@@turn.reply:Reply`,
    thinking: $localize`:@@turn.thinking:Thinking…`,
    reasoning: $localize`:@@turn.reasoning:Reasoning`,
    /** What the working line says while the model thinks, one word after another. */
    words: [
      $localize`:@@turn.forging:Forging…`,
      $localize`:@@turn.honing:Honing…`,
      $localize`:@@turn.tempering:Tempering…`,
      $localize`:@@turn.whetting:Whetting…`,
      $localize`:@@turn.hammering:Hammering it out…`,
      $localize`:@@turn.sharpening:Sharpening…`,
      $localize`:@@turn.quenching:Quenching…`,
      $localize`:@@turn.polishing:Polishing…`,
    ],
  } as const;
  private readonly changed = computed(() => this.turn().files.length > 0);
  /**
   * Whether what changed settles in: only once it appears as the person watches, never when the
   * turn shows with it already there.
   */
  private readonly settles = linkedSignal<boolean, boolean>({
    source: this.changed,
    computation: (changed, previous) =>
      previous !== undefined && changed && (previous.value || !previous.source),
  });
  /**
   * What changed, while the turn changed anything: whether it settles in. `settles` is read
   * whatever it holds, from the turn's first showing, so it knows what came before.
   */
  protected readonly changes = computed(() => {
    const settles = this.settles();
    return this.changed() ? { settles } : null;
  });
  /** The reasoning streaming in, without the blank lines at its ends. */
  protected readonly reasoning = computed(() => trimBlankLines(this.turn().live?.reasoning ?? ''));
  /** Whether the model is still thinking: its reasoning streams, and no answer yet. */
  protected readonly thinking = computed(() => this.turn().live?.text === '');
  /** Open while the model thinks, folded once it answers; the person may change it meanwhile. */
  protected readonly reasoningOpen = linkedSignal(() => this.thinking());

  /** When the turn started, as the server recorded it, while the working line shows. */
  private readonly since = computed(() => this.turn().working?.since ?? null);
  /** Now, in milliseconds since the epoch, moving on each second while the time shows. */
  private readonly now = clock(() => this.since() !== null);
  /** How long the turn has worked, for the reader, while the working line shows. */
  protected readonly elapsed = computed(() => {
    const ran = runningFor(this.since(), this.now());
    return ran === null ? null : formatDuration(ran, this.locale);
  });
}
