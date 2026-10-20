import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
  output,
} from '@angular/core';
import { File, X } from 'lucide';

import { testIds } from '../../lib/test-ids';
import { GlzIconButtonComponent } from '../icon-button/icon-button.component';
import { GlzIconComponent } from '../icon/icon.component';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { remove: 'remove' } as const;

/**
 * A file added to a message: a small chip with the image itself when there is one to show
 * (`preview`, a URL), else a file's mark, its name, and under it a muted `detail` (its size,
 * or what is happening to it). While it is still on its way (`busy`) the chip says so to
 * screen readers; when it was refused (`failed`), its detail is the reason, in the danger
 * tone, and never in colour alone. With a `removeLabel`, a small button takes it off the
 * message (`remove`). A composer holds chips in its `[glzComposerAttachments]` slot, and a
 * message shows the files it was sent with.
 *
 * ```html
 * <glz-attachment
 *   testId="composer-file"
 *   name="screenshot.png"
 *   detail="214 KB"
 *   [preview]="url"
 *   removeLabel="Remove screenshot.png"
 *   (remove)="drop(file)"
 * />
 * ```
 */
@Component({
  selector: 'glz-attachment',
  imports: [GlzIconButtonComponent, GlzIconComponent],
  templateUrl: './attachment.component.html',
  styleUrl: './attachment.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    '[attr.data-testid]': 'testId()',
    '[attr.aria-busy]': 'busyState()',
    '[class.glz-attachment--failed]': 'failed()',
  },
})
export class GlzAttachmentComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** The file's name. */
  readonly name = input.required<string>();
  /** A quiet line under the name: its size, what is happening to it, or why it was refused. */
  readonly detail = input('');
  /** A URL of the image to show, when the file is one; a file's mark otherwise. */
  readonly preview = input<string | null>(null);
  /** Whether it is still on its way, such as uploading. */
  readonly busy = input(false, { transform: booleanAttribute });
  /** Whether it was refused: its detail says why. */
  readonly failed = input(false, { transform: booleanAttribute });
  /** Names the button that takes it off the message; with none, there is no button. */
  readonly removeLabel = input<string | null>(null);
  readonly remove = output();

  protected readonly icons = { file: File, remove: X };
  protected readonly busyState = computed(() => (this.busy() ? 'true' : null));

  protected removed(): void {
    this.remove.emit();
  }
}
