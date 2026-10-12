import { BreakpointObserver } from '@angular/cdk/layout';
import { DOCUMENT } from '@angular/common';
import { Service, effect, inject, signal } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { GLZ_NARROW } from '@glave/glaze';
import { map } from 'rxjs';

import { toSideNavState } from '../lib/panel';
import { SHORTCUTS, ariaShortcut, onApple, pressed, shortcutLabel } from '../lib/shortcuts';
import { localStorageOf, readStored, writeStored } from '../lib/storage';

const KEY = 'glave-side-nav';

/**
 * The side nav as this person left it: beside the page on wide screens, open or collapsed and
 * at the width they dragged it to, kept in this browser's storage when it allows; a drawer on
 * narrow screens, closed at first and never stored. The workspace's layout binds it, and
 * ⌘B or Ctrl+B (`SHORTCUTS.nav`) shows or hides it.
 */
@Service()
export class NavService {
  private readonly document = inject(DOCUMENT);
  private readonly storage = localStorageOf(this.document);
  private readonly stored = toSideNavState(readStored(this.storage, KEY));
  private readonly apple = onApple(this.document);

  readonly open = signal(this.stored.open);
  /** Its width in pixels, or `null` for the default. */
  readonly width = signal<number | null>(this.stored.width);
  /** Whether the screen is narrow, so the side nav is a drawer opened from a button. */
  readonly narrow = toSignal(
    inject(BreakpointObserver)
      .observe(GLZ_NARROW)
      .pipe(map((state) => state.matches)),
    { initialValue: false },
  );
  /** The shortcut as the person reads it (`⌘B`). */
  readonly shortcut = shortcutLabel(SHORTCUTS.nav, this.apple);
  /** The shortcut for `aria-keyshortcuts` (`Meta+B`). */
  readonly ariaShortcut = ariaShortcut(SHORTCUTS.nav, this.apple);

  constructor() {
    effect(() => {
      const state = { open: this.open(), width: this.width() };
      if (!this.narrow()) {
        writeStored(this.storage, KEY, state);
      }
    });
  }

  toggle(): void {
    this.open.update((open) => !open);
  }

  /** Toggles the side nav if `event` presses its shortcut. */
  keydown(event: KeyboardEvent): void {
    if (pressed(event, SHORTCUTS.nav, this.apple)) {
      event.preventDefault();
      this.toggle();
    }
  }
}
