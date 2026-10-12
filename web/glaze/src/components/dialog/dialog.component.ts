import {
  ChangeDetectionStrategy,
  Component,
  type ElementRef,
  computed,
  effect,
  input,
  model,
  viewChild,
} from '@angular/core';
import { X } from 'lucide';

import { injectId } from '../../tokens/ids';
import { GlzIconComponent } from '../icon/icon.component';
import { GlzIconButtonComponent } from '../icon-button/icon-button.component';
import { testIds } from '../../lib/test-ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { close: 'close' } as const;

/**
 * A modal dialog: the native `<dialog>` opened with `showModal()`, so the browser puts it in
 * the top layer, makes the page behind it inert, moves focus in, closes it on Escape and
 * returns focus after; a click on the backdrop closes it too. Named by its heading; a close
 * button, the content, and actions at the end (`[glzDialogActions]`). `open` is bound two ways.
 *
 * ```html
 * <glz-dialog heading="Rename the thread" [(open)]="renaming">
 *   <glz-form-field label="Title"><input glzInput [formControl]="title" /></glz-form-field>
 *   <div glzDialogActions>
 *     <button glzButton (click)="renaming.set(false)">Cancel</button>
 *     <button glzButton variant="primary" (click)="save()">Save</button>
 *   </div>
 * </glz-dialog>
 * ```
 */
@Component({
  selector: 'glz-dialog',
  imports: [GlzIconButtonComponent, GlzIconComponent],
  templateUrl: './dialog.component.html',
  styleUrl: './dialog.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GlzDialogComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  readonly heading = input.required<string>();
  /** Whether the dialog is open. */
  readonly open = model(false);

  protected readonly headingId = injectId('glz-dialog-heading');
  protected readonly closeIcon = X;
  protected readonly closeLabel = 'Close';
  private readonly dialog = viewChild.required<ElementRef<HTMLDialogElement>>('dialog');

  constructor() {
    effect(() => {
      const dialog = this.dialog().nativeElement;
      if (this.open() && !dialog.open) {
        dialog.showModal();
      } else if (!this.open() && dialog.open) {
        dialog.close();
      }
    });
  }

  /** Closes the dialog. */
  close(): void {
    this.open.set(false);
  }

  /** The browser closed it (Escape, or `close()`): keep `open` in step. */
  protected closed(): void {
    this.open.set(false);
  }

  /** A click on the dialog element itself, outside its content, is on the backdrop. */
  protected clicked(event: MouseEvent): void {
    if (event.target === this.dialog().nativeElement) {
      this.close();
    }
  }
}
