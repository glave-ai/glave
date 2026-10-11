import {
  ChangeDetectionStrategy,
  Component,
  type ElementRef,
  booleanAttribute,
  computed,
  effect,
  input,
  model,
  viewChild,
} from '@angular/core';
import { X } from 'lucide';

import { injectId } from '../../lib/ids';
import { GlzIconComponent } from '../icon/icon.component';
import { GlzIconButtonComponent } from '../icon-button/icon-button.component';

/** How a dialog shows: centred, or as a drawer along one side. */
export type GlzDialogKind = 'dialog' | 'drawer-start' | 'drawer-end';

/**
 * A modal dialog or drawer: the native `<dialog>` opened with `showModal()`, so the browser
 * puts it in the top layer, makes the page behind it inert, moves focus in, closes it on
 * Escape and returns focus after. Named by its heading; a close button, the content, and
 * actions at the end (`[glzDialogActions]`). `open` is bound two ways.
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
  readonly heading = input.required<string>();
  /** Whether the dialog is open. */
  readonly open = model(false);
  readonly kind = input<GlzDialogKind>('dialog');
  /** Whether a click on the backdrop closes it, as Escape always does. */
  readonly dismissible = input(true, { transform: booleanAttribute });
  /** The close button's name. */
  readonly closeLabel = input('Close');

  protected readonly headingId = injectId('glz-dialog-heading');
  protected readonly closeIcon = X;
  protected readonly kindClass = computed(() => `dialog ${this.kind()}`);
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
    if (this.dismissible() && event.target === this.dialog().nativeElement) {
      this.close();
    }
  }
}
