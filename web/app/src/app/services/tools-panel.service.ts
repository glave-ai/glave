import { BreakpointObserver } from '@angular/cdk/layout';
import { DOCUMENT } from '@angular/common';
import { Service, computed, effect, inject, signal } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { GLZ_PANEL_NARROW } from '@glave/glaze';
import { map } from 'rxjs';

import { TOOL_TABS, type ToolTab, toPanelState } from '../lib/panel';
import { SHORTCUTS, ariaShortcut, onApple, pressed, shortcutLabel } from '../lib/shortcuts';
import { localStorageOf, readStored, writeStored } from '../lib/storage';

const KEY = 'glave-tools-panel';

/**
 * The thread's tools panel as this person left it: open or closed (closed at first), its
 * width and its tab. Kept in this browser's storage when it allows, so it is the same in
 * every thread and after a reload; without storage it lasts for the page. ⌘. or Ctrl+.
 * (`SHORTCUTS.tools`) shows or hides it.
 */
@Service()
export class ToolsPanelService {
  private readonly document = inject(DOCUMENT);
  private readonly storage = localStorageOf(this.document);
  private readonly stored = toPanelState(readStored(this.storage, KEY));
  private readonly apple = onApple(this.document);

  /** Whether the panel shows. */
  readonly open = signal(this.stored.open);
  /** Its width in pixels, or `null` for the default. */
  readonly width = signal<number | null>(this.stored.width);
  readonly tab = signal<ToolTab>(this.stored.tab);
  /** The tab's place among the tabs, for the tab list. */
  readonly tabIndex = computed(() => TOOL_TABS.indexOf(this.tab()));
  /** Whether the screen is narrow, so the panel is a sheet opened from a button. */
  readonly narrow = toSignal(
    inject(BreakpointObserver)
      .observe(GLZ_PANEL_NARROW)
      .pipe(map((state) => state.matches)),
    { initialValue: false },
  );
  /** The shortcut as the person reads it (`⌘.`). */
  readonly shortcut = shortcutLabel(SHORTCUTS.tools, this.apple);
  /** The shortcut for `aria-keyshortcuts` (`Meta+.`). */
  readonly ariaShortcut = ariaShortcut(SHORTCUTS.tools, this.apple);

  constructor() {
    effect(() => {
      writeStored(this.storage, KEY, { open: this.open(), width: this.width(), tab: this.tab() });
    });
  }

  toggle(): void {
    this.open.update((open) => !open);
  }

  /** Toggles the panel if `event` presses its shortcut. */
  keydown(event: KeyboardEvent): void {
    if (pressed(event, SHORTCUTS.tools, this.apple)) {
      event.preventDefault();
      this.toggle();
    }
  }

  /** Shows the tab at `index` of the tab list. */
  showTab(index: number): void {
    const tab = TOOL_TABS.at(index);
    if (tab !== undefined) {
      this.tab.set(tab);
    }
  }
}
