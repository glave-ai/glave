import { Service, computed, effect, signal } from '@angular/core';
import { GLZ_PANEL_NARROW } from '@glave/glaze';

import { PANEL_DEFAULT, toPaneState } from '../lib/panel';
import { SHORTCUTS } from '../lib/shortcuts';
import { PaneService } from './pane.service';

/**
 * A thread's panel as this person left it, the one place that knows: beside the page on wide
 * screens, shown at first, shown or hidden (`open`) and at the width they dragged it to, kept in
 * this browser's storage when it allows, so it is the same in every thread and after a reload;
 * a sheet on narrow screens, kept apart, closed at first and never stored, so opening or closing
 * it never changes the wide choice. The thread's header button, its edge and ⌘. or Ctrl+.
 * (`SHORTCUTS.panel`) show or hide whichever applies.
 */
@Service()
export class PanelService extends PaneService {
  /** Whether the sheet is open over a narrow page. */
  private readonly sheet = signal(false);
  /** Whether it shows now: beside a wide page, or as a sheet over a narrow one. */
  readonly showing = computed(() => (this.narrow() ? this.sheet() : this.open()));

  constructor() {
    super({
      key: 'glave-thread-panel',
      read: (stored) => toPaneState(stored, PANEL_DEFAULT),
      narrowQuery: GLZ_PANEL_NARROW,
      keys: SHORTCUTS.panel,
    });
    effect(() => {
      this.keep({ open: this.open(), width: this.width() });
    });
  }

  /** Shows or hides it where it is now: the wide choice, or the sheet. */
  setShowing(showing: boolean): void {
    if (this.narrow()) {
      this.sheet.set(showing);
    } else {
      this.open.set(showing);
    }
  }

  override toggle(): void {
    this.setShowing(!this.showing());
  }

  /** Closes the sheet, as a thread with nothing to show, or another thread, starts. */
  closeSheet(): void {
    this.sheet.set(false);
  }
}
