import { BreakpointObserver } from '@angular/cdk/layout';
import { DOCUMENT } from '@angular/common';
import { type Signal, type WritableSignal, inject, signal } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { map } from 'rxjs';

import type { PaneState } from '../lib/panel';
import { ariaShortcut, onApple, pressed, shortcutLabel } from '../lib/shortcuts';
import { readStored, storageOf, writeStored } from '../lib/storage';

/** What sets one side panel apart from another. */
interface PaneOptions<S extends PaneState> {
  /** Where this browser keeps it. */
  readonly key: string;
  /** Reads back what was kept, whatever it is. */
  readonly read: (stored: unknown) => S;
  /** The media query under which it slides over the page instead of sitting beside it. */
  readonly narrowQuery: string;
  /** The shortcut that shows or hides it, in the form `SHORTCUTS` gives. */
  readonly keys: string;
}

/**
 * A side panel as the person left it: open or not, and the width they dragged it to, read back
 * from this browser's storage when it allows; whether the screen is narrow; and its shortcut.
 * The side nav and the tools panel are each one, keeping what they choose with `keep`.
 */
export abstract class PaneService<S extends PaneState> {
  private readonly document = inject(DOCUMENT);
  private readonly storage = storageOf(this.document);
  private readonly apple = onApple(this.document);

  /** How the person left it, or the default for someone new. */
  protected readonly stored: S;
  readonly open: WritableSignal<boolean>;
  /** Its width in pixels, or `null` for the default. */
  readonly width: WritableSignal<number | null>;
  /** Whether the screen is narrow, so it is opened from a button over the page. */
  readonly narrow: Signal<boolean>;
  /** The shortcut as the person reads it (`⌘B`). */
  readonly shortcut: string;
  /** The shortcut for `aria-keyshortcuts` (`Meta+B`). */
  readonly ariaShortcut: string;

  protected constructor(private readonly options: PaneOptions<S>) {
    this.stored = options.read(readStored(this.storage, options.key));
    this.open = signal(this.stored.open);
    this.width = signal(this.stored.width);
    this.narrow = toSignal(
      inject(BreakpointObserver)
        .observe(options.narrowQuery)
        .pipe(map((state) => state.matches)),
      { initialValue: false },
    );
    this.shortcut = shortcutLabel(options.keys, this.apple);
    this.ariaShortcut = ariaShortcut(options.keys, this.apple);
  }

  toggle(): void {
    this.open.update((open) => !open);
  }

  /** Toggles it if `event` presses its shortcut. */
  keydown(event: KeyboardEvent): void {
    if (pressed(event, this.options.keys, this.apple)) {
      event.preventDefault();
      this.toggle();
    }
  }

  /** Keeps `state` in this browser, if it allows; else it lasts for the page. */
  protected keep(state: S): void {
    writeStored(this.storage, this.options.key, state);
  }
}
