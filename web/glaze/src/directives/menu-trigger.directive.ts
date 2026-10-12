import { CdkMenuTrigger } from '@angular/cdk/menu';
import { Directive } from '@angular/core';

/**
 * Opens a `<glz-menu>` from a button: click, Enter, Space or the down arrow open it; Escape
 * closes it and returns focus. Point it at the `<ng-template>` that holds the menu.
 *
 * ```html
 * <button glzButton [glzMenuTrigger]="actions">Actions</button>
 * ```
 */
@Directive({
  selector: '[glzMenuTrigger]',
  hostDirectives: [
    {
      directive: CdkMenuTrigger,
      inputs: ['cdkMenuTriggerFor: glzMenuTrigger', 'cdkMenuPosition: glzMenuPosition'],
      outputs: ['cdkMenuOpened: opened', 'cdkMenuClosed: closed'],
    },
  ],
})
export class GlzMenuTriggerDirective {}
