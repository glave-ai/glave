import { Service, computed, effect, signal } from '@angular/core';
import { GLZ_PANEL_NARROW } from '@glave/glaze';

import { type PanelState, TOOL_TABS, type ToolTab, toPanelState } from '../lib/panel';
import { SHORTCUTS } from '../lib/shortcuts';
import { PaneService } from './pane.service';

/**
 * The thread's tools panel as this person left it: open or closed (closed at first), its
 * width and its tab. Kept in this browser's storage when it allows, so it is the same in
 * every thread and after a reload; without storage it lasts for the page. ⌘. or Ctrl+.
 * (`SHORTCUTS.tools`) shows or hides it.
 */
@Service()
export class ToolsPanelService extends PaneService<PanelState> {
  readonly tab = signal<ToolTab>(this.stored.tab);
  /** The tab's place among the tabs, for the tab list. */
  readonly tabIndex = computed(() => TOOL_TABS.indexOf(this.tab()));
  /** Whether it is open on the thread's changes. */
  readonly changesShown = computed(() => this.open() && this.tab() === 'changes');
  /** Whether it is open on the thread's commands in the background. */
  readonly commandsShown = computed(() => this.open() && this.tab() === 'commands');

  constructor() {
    super({
      key: 'glave-tools-panel',
      read: toPanelState,
      narrowQuery: GLZ_PANEL_NARROW,
      keys: SHORTCUTS.tools,
    });
    effect(() => {
      this.keep({ open: this.open(), width: this.width(), tab: this.tab() });
    });
  }

  /** Opens it on the thread's changes, or closes it when it already shows them. */
  toggleChanges(): void {
    if (this.changesShown()) {
      this.open.set(false);
      return;
    }
    this.tab.set('changes');
    this.open.set(true);
  }

  /** Opens it on the thread's changes. */
  showChanges(): void {
    this.tab.set('changes');
    this.open.set(true);
  }

  /** Shows the tab at `index` of the tab list. */
  showTab(index: number): void {
    const tab = TOOL_TABS.at(index);
    if (tab !== undefined) {
      this.tab.set(tab);
    }
  }
}
