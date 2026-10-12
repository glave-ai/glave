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
import { GlzCopyDirective, GlzIconButtonComponent, GlzIconComponent } from '@glave/glaze';
import { Check, Copy } from 'lucide';

const COPY = {
  copy: 'Copy',
  copied: 'Copied',
  failed: 'Could not copy',
} as const;

/** How long the button says it copied, in milliseconds. */
const CONFIRM_MS = 2000;

/**
 * A small button that copies a message's text, as plain text: it says "Copied" for a moment,
 * with a check, and announces it to screen readers.
 */
@Component({
  selector: 'app-copy-button',
  imports: [GlzCopyDirective, GlzIconButtonComponent, GlzIconComponent],
  templateUrl: './copy-button.component.html',
  styleUrl: './copy-button.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class CopyButtonComponent {
  /** The text it copies. */
  readonly text = input.required<string>();

  private readonly announcer = inject(LiveAnnouncer);
  private readonly done = signal(false);
  private timer: ReturnType<typeof setTimeout> | undefined;

  protected readonly icon = computed(() => (this.done() ? Check : Copy));
  protected readonly label = computed(() => (this.done() ? COPY.copied : COPY.copy));

  constructor() {
    inject(DestroyRef).onDestroy(() => {
      clearTimeout(this.timer);
    });
  }

  protected copied(ok: boolean): void {
    void this.announcer.announce(ok ? COPY.copied : COPY.failed);
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
