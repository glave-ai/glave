import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
  model,
  output,
} from '@angular/core';

import { type GlzButtonVariant, GlzButtonComponent } from '../button/button.component';
import { GlzDialogComponent } from '../dialog/dialog.component';
import { testIds } from '../../lib/test-ids';
import { glzText } from '../../lib/words';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { cancel: 'cancel', ok: 'ok' } as const;

/**
 * Asks the person to confirm before something happens: a question, Cancel, and the action.
 * `decided` says which they chose (`true` to go ahead); Escape and the close button count as
 * Cancel. The dialog's first control, its close button, takes focus first, so Enter never
 * destroys anything by accident. What it holds, such as a choice that goes with the action,
 * shows under the message.
 *
 * ```html
 * <glz-confirm-dialog heading="Delete the thread?" confirmLabel="Delete" danger
 *   [(open)]="confirming" (decided)="$event && remove()" />
 * ```
 */
@Component({
  selector: 'glz-confirm-dialog',
  imports: [GlzDialogComponent, GlzButtonComponent],
  templateUrl: './confirm-dialog.component.html',
  styleUrl: './confirm-dialog.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GlzConfirmDialogComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));
  private readonly words = glzText('confirmDialog', {
    confirmLabel: 'Continue',
    cancelLabel: 'Cancel',
  });

  readonly open = model(false);
  readonly heading = input.required<string>();
  /** What will happen, in a sentence or two. */
  readonly message = input('');
  readonly confirmLabel = input(this.words.confirmLabel);
  readonly cancelLabel = input(this.words.cancelLabel);
  /** Whether going ahead destroys something, so the action says so in red. */
  readonly danger = input(false, { transform: booleanAttribute });
  /** The person's choice: `true` to go ahead. */
  readonly decided = output<boolean>();

  protected readonly variant = computed<GlzButtonVariant>(() =>
    this.danger() ? 'danger' : 'primary',
  );
  protected readonly hasMessage = computed(() => this.message() !== '');

  protected cancel(): void {
    this.decide(false);
  }

  protected confirm(): void {
    this.decide(true);
  }

  /** The dialog closed some other way (Escape, the close button): that is a no. */
  protected openChanged(open: boolean): void {
    if (!open && this.open()) {
      this.decide(false);
    }
  }

  private decide(yes: boolean): void {
    this.open.set(false);
    this.decided.emit(yes);
  }
}
