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
import { ThreadService } from '../../services/thread.service';
import { ThreadItemComponent } from '../thread-item/thread-item.component';
import { TurnChangesComponent } from '../turn-changes/turn-changes.component';
import { TurnPartComponent } from '../turn-part/turn-part.component';

/**
 * One turn: the person's message; then, in order, what the model said at each step, each run of
 * calls between as the server's line, which opens to its calls, and the questions and approvals
 * already answered, with the pages the reply drew on from the web under it; what the model
 * writes now, as markdown; what changed; the receipt, a quiet line with how it went, how long it
 * took, what it cost and the model, which opens to where it ran; and last, what waits for the
 * person.
 *
 * While the turn works with nothing of its own to show, a line stands in the reply's place: the
 * step a tool takes, in the server's words, else a smithy's word while the model thinks, the next
 * every couple of seconds; and beside it how long the turn has worked, counted from the start
 * the server recorded. The model's reasoning, as it streams, is folded, as each step's is, for the
 * person to open. As the person watches, a
 * reply rises into place as it begins, what changed settles in as it appears, and a card that
 * needs the person rises into place.
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
    TurnPartComponent,
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
  /** Whether the turn changed anything, once the server has said; `null` until then. */
  private readonly changed = computed(() => {
    const files = this.turn().files;
    return files === null ? null : files.length > 0;
  });
  /**
   * Whether what changed settles in: only once it appears as the person watches, never when the
   * turn shows with it already there, nor when it was there as the thread opened and shows once
   * the server has said so.
   */
  private readonly settles = linkedSignal<boolean | null, boolean>({
    source: this.changed,
    computation: (changed, previous) =>
      changed === true && previous !== undefined && (previous.value || previous.source === false),
  });
  /**
   * What changed, while the turn changed anything: its files, and whether it settles in.
   * `settles` is read whatever it holds, from the turn's first showing, so it knows what came
   * before.
   */
  protected readonly changes = computed(() => {
    const settles = this.settles();
    const files = this.turn().files ?? [];
    return files.length > 0 ? { settles, files } : null;
  });
  /** Whether the thread is being opened, its history read: what shows meanwhile was there. */
  private readonly opening = inject(ThreadService).opening;
  /**
   * What waits for the person that appeared as the person watched, by key: never what the turn
   * showed with as it first showed, nor what the thread's history brought as it opened.
   */
  private readonly risen = linkedSignal<
    { readonly keys: readonly string[]; readonly opening: boolean },
    ReadonlySet<string>
  >({
    source: () => ({ keys: this.turn().waiting.map((item) => item.key), opening: this.opening() }),
    computation: ({ keys, opening }, previous) => {
      if (previous === undefined) {
        return new Set();
      }
      const before = new Set(previous.source.keys);
      return new Set(
        keys.filter((key) => previous.value.has(key) || (!opening && !before.has(key))),
      );
    },
  });
  /** What waits for the person, each with whether it rises into place. */
  protected readonly waiting = computed(() => {
    const risen = this.risen();
    return this.turn().waiting.map((item) => ({ item, rises: risen.has(item.key) }));
  });
  /** The reasoning streaming in, without the blank lines at its ends. */
  protected readonly reasoning = computed(() => trimBlankLines(this.turn().live?.reasoning ?? ''));
  /** Whether the model is still thinking: its reasoning streams, and no answer yet. */
  protected readonly thinking = computed(() => this.turn().live?.text === '');

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
