import {
  ChangeDetectionStrategy,
  Component,
  type ElementRef,
  afterRenderEffect,
  computed,
  effect,
  input,
  model,
  viewChild,
} from '@angular/core';
import { X } from 'lucide';

import { injectId } from '../../tokens/ids';
import { glzText } from '../../lib/words';
import { GlzIconComponent } from '../icon/icon.component';
import { GlzIconButtonComponent } from '../icon-button/icon-button.component';
import { testIds } from '../../lib/test-ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { close: 'close', body: 'body' } as const;

/**
 * A modal dialog: the native `<dialog>` opened with `showModal()`, so the browser puts it in
 * the top layer, makes the page behind it inert, moves focus in, closes it on Escape and
 * returns focus after; a click on the backdrop closes it too. Escape closes it alone, even
 * when it was opened from a drawer. Named by its heading; a close
 * button, the content, and actions at the end (`[glzDialogActions]`). It grows with its content
 * up to the screen's height, then only the content scrolls: the heading and actions always
 * show. `open` is bound two ways.
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
  protected readonly closeLabel = glzText('dialog', { closeLabel: 'Close' }).closeLabel;
  private readonly dialog = viewChild.required<ElementRef<HTMLDialogElement>>('dialog');

  /**
   * Opens the native dialog once the page is rendered: a dialog made open then opens with its
   * controls bound, and the browser moves focus to the first of them, named
   * (`INERT_UNTIL_BOUND` keeps them out of reach until then).
   */
  protected readonly opening = afterRenderEffect(() => {
    const dialog = this.dialog().nativeElement;
    if (this.open() && !dialog.open) {
      dialog.showModal();
    }
  });

  /**
   * Closes it as soon as `open` turns false, so the focus the browser gives back lands before
   * what the page renders next moves it on (a new project's composer).
   */
  protected readonly closing = effect(() => {
    const dialog = this.dialog().nativeElement;
    if (!this.open() && dialog.open) {
      dialog.close();
    }
  });

  /** Closes the dialog. */
  close(): void {
    this.open.set(false);
  }

  /** The browser closed it (Escape, or `close()`): keep `open` in step. */
  protected closed(): void {
    this.open.set(false);
  }

  /**
   * Escape is this dialog's alone: the browser closes it, and nothing around it (a drawer it
   * was opened from) hears the key and closes too.
   */
  protected escaped(event: Event): void {
    event.stopPropagation();
  }

  /** A click on the dialog element itself, outside its content, is on the backdrop. */
  protected clicked(event: MouseEvent): void {
    if (event.target === this.dialog().nativeElement) {
      this.close();
    }
  }
}
