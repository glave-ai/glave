import { LiveAnnouncer } from '@angular/cdk/a11y';
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  input,
  signal,
} from '@angular/core';
import { Check, Copy } from 'lucide';

import { GlzCopyDirective } from '../../directives/copy.directive';
import { GlzIconButtonComponent } from '../icon-button/icon-button.component';
import { GlzIconComponent } from '../icon/icon.component';
import { glzText } from '../../lib/words';

/** How long the button says it copied, in milliseconds. */
const CONFIRM_MS = 2000;

/**
 * A small icon button that copies `text` as plain text: it says `copiedLabel` for a moment,
 * with a check, and announces it to screen readers (or `failedLabel` when the clipboard
 * refuses). A message's actions use it, and so do markdown's code blocks.
 *
 * ```html
 * <glz-copy-button glzMessageActions testId="message-copy" [text]="message.text" />
 * ```
 */
@Component({
  selector: 'glz-copy-button',
  imports: [GlzCopyDirective, GlzIconButtonComponent, GlzIconComponent],
  templateUrl: './copy-button.component.html',
  styleUrl: './copy-button.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GlzCopyButtonComponent {
  private readonly words = glzText('copyButton', {
    label: 'Copy',
    copiedLabel: 'Copied',
    failedLabel: 'Could not copy',
  });
  /** The text it copies. */
  readonly text = input.required<string>();
  /** What it does, in words: names the button. */
  readonly label = input(this.words.label);
  readonly copiedLabel = input(this.words.copiedLabel);
  readonly failedLabel = input(this.words.failedLabel);
  /** The test hook for the button. */
  readonly testId = input<string | null>(null);

  private readonly announcer = inject(LiveAnnouncer);
  private readonly done = signal(false);
  private timer: ReturnType<typeof setTimeout> | undefined;

  protected readonly icon = computed(() => (this.done() ? Check : Copy));
  protected readonly shownLabel = computed(() => (this.done() ? this.copiedLabel() : this.label()));

  constructor() {
    inject(DestroyRef).onDestroy(() => {
      clearTimeout(this.timer);
    });
  }

  protected copied(ok: boolean): void {
    void this.announcer.announce(ok ? this.copiedLabel() : this.failedLabel());
    if (!ok) {
      return;
    }
    this.done.set(true);
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.done.set(false);
    }, CONFIRM_MS);
  }
}
