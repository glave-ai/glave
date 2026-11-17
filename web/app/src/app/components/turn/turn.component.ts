import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  type ElementRef,
  LOCALE_ID,
  computed,
  effect,
  inject,
  input,
  linkedSignal,
  viewChild,
} from '@angular/core';
import {
  GlzMarkdownComponent,
  GlzMessageComponent,
  GlzStepsComponent,
  GlzWorkingComponent,
  formatDuration,
} from '@glave/glaze';

import { trimBlankLines } from '../../lib/conversation';
import { runDate } from '../../lib/repeating';
import type { TurnResults } from '../../lib/results';
import { runningFor } from '../../lib/when';
import { clock } from '../../services/clock';
import { READER_ZONE } from '../../services/zone.service';
import { ThreadService } from '../../services/thread.service';
import { ThoughtComponent } from '../thought/thought.component';
import { ThreadItemComponent } from '../thread-item/thread-item.component';
import { TurnChangesComponent } from '../turn-changes/turn-changes.component';
import { TurnPartComponent } from '../turn-part/turn-part.component';

/**
 * One turn, headed, for a schedule's run, with when the run was due, and what it was asked
 * folded under that, quiet, rather than as the person's message (a run before the schedule's
 * latest reads quieter still); else the person's message; then, in order, what the model said at each step, each run of
 * calls between as the server's line, which opens to its calls, and the questions and approvals
 * already answered, with the pages the reply drew on from the web under it; what the model
 * writes now, as markdown; what changed, its room kept as the thread opens until it is read; the
 * receipt, a quiet line with how it went, how long it took, what it cost and the model, which
 * opens to where it ran; and last, what waits for the person. Opened at it from the notification
 * of its end, a line above its reply says when it finished while the person was away, in view.
 *
 * While the turn works with nothing of its own to show, a line stands in the reply's place: the
 * step a tool takes, in the server's words, else a smithy's word while the model thinks, the next
 * every couple of seconds; and beside it how long the turn has worked, counted from the start
 * the server recorded. The model's reasoning, as it streams, is one line, as each step's is:
 * what it thinks now, in the model's own words, which opens to it in full once done. As the
 * person watches, a
 * reply rises into place as it begins, what changed settles in as it appears, and a card that
 * needs the person rises into place.
 */
@Component({
  selector: 'app-turn',
  imports: [
    GlzMarkdownComponent,
    GlzMessageComponent,
    GlzStepsComponent,
    GlzWorkingComponent,
    ThoughtComponent,
    ThreadItemComponent,
    TurnChangesComponent,
    TurnPartComponent,
  ],
  templateUrl: './turn.component.html',
  styleUrl: './turn.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[class.earlier]': 'earlier()' },
})
export class TurnComponent {
  readonly turn = input.required<TurnResults>();
  /**
   * The line it shows above its reply, brought into view, when a tapped notification of its end
   * opened the chat at it: when it finished, while the person was away.
   */
  readonly away = input<string | null>(null);
  /** Whether it is a schedule's run before its latest, which reads quieter. */
  readonly earlier = input(false, { transform: booleanAttribute });
  private readonly locale = inject(LOCALE_ID);
  private readonly zone = inject(READER_ZONE);
  /**
   * For a schedule's run, its heading: when it was due, as its start recorded it, on the reader's
   * clocks, so no later change to the schedule moves it; and what it was asked, its prompt,
   * folded under that rather than shown as the person's message. `null` for any other turn.
   */
  protected readonly run = computed(() => {
    const at = this.turn().scheduled;
    const reader = { locale: this.locale, timeZone: this.zone };
    const text = at === null ? null : runDate(at, this.zone, reader);
    const asked = this.turn().prompt?.text ?? '';
    return at === null || text === null ? null : { at, text, asked };
  });
  /** Where the reply is among its parts: the last of the model's words that are no aside. */
  private readonly reply = computed(() =>
    this.turn().parts.reduce(
      (found, part, at) =>
        part.kind === 'item' &&
        !part.interim &&
        part.item.kind === 'message' &&
        part.item.from === 'model'
          ? at
          : found,
      -1,
    ),
  );
  /** Its parts, the reply with the line above it when it shows one. */
  protected readonly parts = computed(() => {
    const [away, reply] = [this.away(), this.reply()];
    return this.turn().parts.map((part, at) => ({ part, away: at === reply ? away : null }));
  });
  /** The line after its parts, when it shows one and has no reply to show it above. */
  protected readonly awayLast = computed(() => (this.reply() === -1 ? this.away() : null));
  private readonly line = viewChild<ElementRef<HTMLElement>>('away');

  protected readonly copy = {
    reply: $localize`:@@turn.reply:Reply`,
    thinking: $localize`:@@turn.thinking:Thinking…`,
    asked: $localize`:@@turn.asked:What it was asked`,
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
  /** Until what changed is known, the files whose room it keeps, unseen; else `null`. */
  protected readonly room = computed(() => {
    const room = this.turn().room;
    return room.length > 0 ? room : null;
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

  /** When the turn started, as the server recorded it, while the working line shows. */
  private readonly since = computed(() => this.turn().working?.since ?? null);
  /** Now, in milliseconds since the epoch, moving on each second while the time shows. */
  private readonly now = clock(() => this.since() !== null);
  /** How long the turn has worked, for the reader, while the working line shows. */
  protected readonly elapsed = computed(() => {
    const ran = runningFor(this.since(), this.now());
    return ran === null ? null : formatDuration(ran, this.locale);
  });

  constructor() {
    // The chat opens at the line, the reply below it in view.
    effect(() => {
      this.line()?.nativeElement.scrollIntoView({ block: 'start' });
    });
  }
}
