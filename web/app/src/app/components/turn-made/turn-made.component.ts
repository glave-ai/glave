import { ChangeDetectionStrategy, Component, input } from '@angular/core';
import type { MadeFile } from '@glave/client';

import { MadeFileComponent } from '../made-file/made-file.component';

/**
 * The files a turn made, after its reply: a card each, then what they leave out, in the server's
 * sentence ("1 more file was made than a chat keeps…"). Until the server describes them, each
 * card's room and the note's, unseen (`room`), so nothing below moves as they come in. Its cards
 * sit among the turn's other blocks, spaced as they are.
 */
@Component({
  selector: 'app-turn-made',
  imports: [MadeFileComponent],
  templateUrl: './turn-made.component.html',
  styleUrl: './turn-made.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TurnMadeComponent {
  readonly files = input.required<readonly MadeFile[]>();
  /** Whether the cards are only room kept for them, drawn unseen. */
  readonly room = input(false);
  /** What the files leave out, in the server's sentence; `null` when nothing. */
  readonly note = input<string | null>(null);
}
