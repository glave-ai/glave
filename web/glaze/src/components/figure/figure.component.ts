import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { glzText } from '../../lib/words';
import { testIds } from '../../lib/test-ids';
import { GlzButtonComponent } from '../button/button.component';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { image: 'image', download: 'download' } as const;

/**
 * An image in a conversation, such as one the agent made: the image whole in a square frame
 * (`--glz-figure-size`, no wider than the column), and under it a link that saves it as
 * `name`. While it is still on its way (`src` is `null`) the frame holds its place and says so
 * to screen readers; when it can't be shown (`failed`), the reason stands in it. The frame is
 * the same in each, so nothing moves as the image arrives. `alt` says what it shows.
 *
 * ```html
 * <glz-figure testId="thread-image" [src]="url" alt="A red fox at dawn" name="fox.png" />
 * ```
 */
@Component({
  selector: 'glz-figure',
  imports: [GlzButtonComponent],
  templateUrl: './figure.component.html',
  styleUrl: './figure.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[attr.data-testid]': 'testId()',
    '[attr.aria-busy]': 'busy()',
  },
})
export class GlzFigureComponent {
  private readonly words = glzText('figure', { downloadLabel: 'Download' });
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** A URL of the image; `null` while it is still on its way. */
  readonly src = input<string | null>(null);
  /** What the image shows, for those who can't see it. */
  readonly alt = input.required<string>();
  /** The name it is saved as. */
  readonly name = input.required<string>();
  /** Why it can't be shown, in its place; `null` when nothing went wrong. */
  readonly failed = input<string | null>(null);
  /** Names the link that saves it. */
  readonly downloadLabel = input(this.words.downloadLabel);

  protected readonly busy = computed(() =>
    this.src() === null && this.failed() === null ? 'true' : null,
  );
}
