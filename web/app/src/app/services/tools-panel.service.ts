import { DOCUMENT } from '@angular/common';
import { Service, computed, effect, inject, signal } from '@angular/core';

import { TOOL_TABS, type ToolTab, toPanelState } from '../lib/panel';
import { localStorageOf, readStored, writeStored } from '../lib/storage';

const KEY = 'glave-tools-panel';

/**
 * The thread's tools panel as this person left it: open or closed (closed at first), its
 * width and its tab. Kept in this browser's storage when it allows, so it is the same in
 * every thread and after a reload; without storage it lasts for the page.
 */
@Service()
export class ToolsPanelService {
  private readonly storage = localStorageOf(inject(DOCUMENT));
  private readonly stored = toPanelState(readStored(this.storage, KEY));

  /** Whether the panel shows. */
  readonly open = signal(this.stored.open);
  /** Its width in pixels, or `null` for the default. */
  readonly width = signal<number | null>(this.stored.width);
  readonly tab = signal<ToolTab>(this.stored.tab);
  /** The tab's place among the tabs, for the tab list. */
  readonly tabIndex = computed(() => TOOL_TABS.indexOf(this.tab()));

  constructor() {
    effect(() => {
      writeStored(this.storage, KEY, { open: this.open(), width: this.width(), tab: this.tab() });
    });
  }

  toggle(): void {
    this.open.update((open) => !open);
  }

  /** Shows the tab at `index` of the tab list. */
  showTab(index: number): void {
    const tab = TOOL_TABS.at(index);
    if (tab !== undefined) {
      this.tab.set(tab);
    }
  }
}
