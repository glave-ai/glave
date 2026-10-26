import { Service, effect } from '@angular/core';
import { GLZ_NARROW } from '@glave/glaze';

import { SIDE_NAV_DEFAULT, toPaneState } from '../lib/panel';
import { SHORTCUTS } from '../lib/shortcuts';
import { PaneService } from './pane.service';

/**
 * The side nav as this person left it: beside the page on wide screens, open or collapsed and
 * at the width they dragged it to, kept in this browser's storage when it allows; a drawer on
 * narrow screens, closed at first and never stored. The workspace's layout binds it, and
 * ⌘B or Ctrl+B (`SHORTCUTS.nav`) shows or hides it.
 */
@Service()
export class NavService extends PaneService {
  constructor() {
    super({
      key: 'glave-side-nav',
      read: (stored) => toPaneState(stored, SIDE_NAV_DEFAULT),
      narrowQuery: GLZ_NARROW,
      keys: SHORTCUTS.nav,
    });
    effect(() => {
      if (!this.narrow()) {
        this.keep({ open: this.open(), width: this.width() });
      }
    });
  }
}
