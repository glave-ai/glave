import { Clipboard } from '@angular/cdk/clipboard';
import { Directive, inject, input, output } from '@angular/core';

/**
 * Copies text to the clipboard when its element is clicked, through the CDK's Clipboard.
 * `copied` says whether it worked, to confirm with a toast or a changed label.
 *
 * ```html
 * <button glzButton [glzCopy]="code()" (copied)="toasts.show('Copied')">Copy</button>
 * ```
 */
@Directive({
  selector: '[glzCopy]',
  host: { '(click)': 'copy()' },
})
export class GlzCopyDirective {
  readonly text = input.required<string>({ alias: 'glzCopy' });
  /** Whether the text reached the clipboard. */
  readonly copied = output<boolean>();

  private readonly clipboard = inject(Clipboard);

  protected copy(): void {
    this.copied.emit(this.clipboard.copy(this.text()));
  }
}
