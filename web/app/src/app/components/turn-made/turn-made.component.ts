import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  input,
  signal,
} from '@angular/core';
import { GlzIconComponent } from '@glave/glaze';
import { ChevronDown, ChevronUp } from 'lucide';

import { type MadeCard, type MadeShown, NO_MADE } from '../../lib/made';
import { plural } from '../../lib/plural';
import { MadeFileComponent } from '../made-file/made-file.component';

/** How many cards a turn that made more shows until the person unfolds the rest. */
const FOLDED = 3;

/** A card as the turn draws it: described, or only its room until the server describes it. */
interface Drawn {
  readonly card: MadeCard;
  readonly room: boolean;
}

/**
 * The files a turn made, after its reply: a card each, then what they leave out, in the server's
 * sentence ("1 more file was made than a chat keeps…"). Until the server describes a file its
 * log names, its card's room, unseen, and the note's while the log says something was left out,
 * so nothing below moves as they come in. A turn that made 4 or more shows the first 3, then a
 * row that unfolds the rest in place and folds them again, as this tab alone remembers: how many
 * there are is known from the log, so the row is there from the start. Its cards sit among the
 * turn's other blocks, spaced as they are.
 */
@Component({
  selector: 'app-turn-made',
  imports: [GlzIconComponent, MadeFileComponent],
  templateUrl: './turn-made.component.html',
  styleUrl: './turn-made.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TurnMadeComponent {
  readonly made = input<MadeShown>(NO_MADE);

  private readonly locale = inject(LOCALE_ID);
  /** Whether the person unfolded every card. */
  protected readonly unfolded = signal(false);
  private readonly drawn = computed<readonly Drawn[]>(() => [
    ...this.made().files.map((card) => ({ card, room: false })),
    ...this.made().room.map((card) => ({ card, room: true })),
  ]);
  /** Whether there are more cards than a folded turn shows. */
  protected readonly folds = computed(() => this.drawn().length > FOLDED);
  protected readonly cards = computed(() =>
    this.folds() && !this.unfolded() ? this.drawn().slice(0, FOLDED) : this.drawn(),
  );
  /** Whether the row that folds them is only its room, unseen, until the server describes one. */
  protected readonly foldRoom = computed(() => this.made().files.length === 0);
  protected readonly foldLabel = computed(() => {
    if (this.unfolded()) {
      return $localize`:@@turnMade.fewer:Show fewer`;
    }
    const hidden = this.drawn().length - FOLDED;
    return plural(
      $localize`:@@turnMade.more:{${hidden}:COUNT:, plural, one {# more file} other {# more files}}`,
      this.locale,
    );
  });
  protected readonly foldIcon = computed(() => (this.unfolded() ? ChevronUp : ChevronDown));
  protected readonly note = computed(() => this.made().note);
  protected readonly noteRoom = computed(() => this.made().noteRoom);

  /** Unfolds every card, or folds them back to the first. */
  protected toggle(): void {
    this.unfolded.update((unfolded) => !unfolded);
  }
}
