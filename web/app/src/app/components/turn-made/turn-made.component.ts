import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { type MadeShown, NO_MADE } from '../../lib/made';
import { MadeFileComponent } from '../made-file/made-file.component';

/**
 * The files a turn made, after its reply: a card each, then what they leave out, in the server's
 * sentence ("1 more file was made than a chat keeps…"). Until the server describes a file its
 * log names, its card's room, unseen, and the note's while the log says something was left out,
 * so nothing below moves as they come in. Its cards sit among the turn's other blocks, spaced as
 * they are.
 */
@Component({
  selector: 'app-turn-made',
  imports: [MadeFileComponent],
  templateUrl: './turn-made.component.html',
  styleUrl: './turn-made.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TurnMadeComponent {
  readonly made = input<MadeShown>(NO_MADE);

  protected readonly files = computed(() => this.made().files);
  protected readonly room = computed(() => this.made().room);
  protected readonly note = computed(() => this.made().note);
  protected readonly noteRoom = computed(() => this.made().noteRoom);
}
