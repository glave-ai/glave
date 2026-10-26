import { Service, effect } from '@angular/core';
import { GLZ_PANEL_NARROW } from '@glave/glaze';

import { PANEL_DEFAULT, toPaneState } from '../lib/panel';
import { SHORTCUTS } from '../lib/shortcuts';
import { PaneService } from './pane.service';

/**
 * A thread's panel as this person left it: beside the page on wide screens, shown at first,
 * hidden or shown and at the width they dragged it to, kept in this browser's storage when it
 * allows, so it is the same in every thread and after a reload; a sheet on narrow screens,
 * closed at first and never stored. The thread's header button, its edge and ⌘. or Ctrl+.
 * (`SHORTCUTS.panel`) show or hide it.
 */
@Service()
export class PanelService extends PaneService {
  constructor() {
    super({
      key: 'glave-thread-panel',
      read: (stored) => toPaneState(stored, PANEL_DEFAULT),
      narrowQuery: GLZ_PANEL_NARROW,
      keys: SHORTCUTS.panel,
    });
    effect(() => {
      if (!this.narrow()) {
        this.keep({ open: this.open(), width: this.width() });
      }
    });
  }
}
