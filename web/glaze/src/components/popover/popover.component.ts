import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  computed,
  inject,
  input,
  signal,
} from '@angular/core';

import { type GlzSide, positionPopover } from '../../lib/popover';
import { injectId } from '../../tokens/ids';

/**
 * A popover beside the button that opens it, for a few links or actions (`glzNavItem`s): who
 * is signed in and their pages, or a short list to jump to. The native popover API, so it sits
 * in the top layer, closes on Escape or a click outside, and comes right after its button in
 * the focus order. Its button is a `button[glzPopoverTrigger]` pointing at it, which opens and
 * closes it natively and says whether it is open (`aria-expanded`). Following a link or
 * pressing a button in it closes it. It is placed on `side` of its button: by CSS anchor
 * positioning where the browser has it, else measured.
 *
 * Not an ARIA menu: its items are plain links and buttons, reached with Tab.
 *
 * ```html
 * <button glzNavItem [glzPopoverTrigger]="mine">You</button>
 * <glz-popover #mine>
 *   <a glzNavItem routerLink="/settings">Settings</a>
 *   <button glzNavItem (click)="signOut()">Sign out</button>
 * </glz-popover>
 * ```
 */
@Component({
  selector: 'glz-popover',
  templateUrl: './popover.component.html',
  styleUrl: './popover.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    popover: 'auto',
    '[id]': 'id',
    '[attr.data-testid]': 'testId()',
    '(beforetoggle)': 'toggling($event)',
    '(toggle)': 'toggling($event)',
    '(click)': 'clicked($event)',
  },
})
export class GlzPopoverComponent {
  /** The test hook for this component. */
  readonly testId = input<string | null>(null);
  /** Where it opens beside its button. */
  readonly side = input<GlzSide>('above');

  /** Its id, which its button controls. */
  readonly id = injectId('glz-popover');
  /** The popover element, which its button opens. */
  readonly element = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
  private readonly shown = signal(false);
  /** Whether it is open. */
  readonly open = this.shown.asReadonly();
  /** Whether it is open, as `aria-expanded` says it. */
  readonly expanded = computed(() => (this.shown() ? 'true' : 'false'));
  private anchor: HTMLElement | null = null;

  /** Sets the button it opens beside. Its trigger calls this. */
  attach(anchor: HTMLElement): void {
    this.anchor = anchor;
  }

  /** Closes it, if it is open. */
  close(): void {
    if (this.shown()) {
      this.element.hidePopover();
    }
  }

  // Placed as it opens (where anchor positioning does it, before it shows), and again once it
  // shows, where it is measured; `toggle` also follows closing by Escape or a click outside.
  protected toggling(event: Event): void {
    const opening = 'newState' in event && event.newState === 'open';
    if (event.type === 'toggle') {
      this.shown.set(opening);
    }
    if (opening && this.anchor !== null) {
      positionPopover(this.anchor, this.element, this.side(), `--${this.id}`);
    }
  }

  /** Closes it once a link in it is followed or a button in it pressed. */
  protected clicked(event: Event): void {
    if (event.target instanceof Element && event.target.closest('a[href], button') !== null) {
      this.close();
    }
  }
}
