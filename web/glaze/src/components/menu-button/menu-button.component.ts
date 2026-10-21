import { CdkMenu, CdkMenuItem, CdkMenuTrigger } from '@angular/cdk/menu';
import type { ConnectedPosition } from '@angular/cdk/overlay';
import { ChangeDetectionStrategy, Component, computed, input, output } from '@angular/core';
import { Ellipsis } from 'lucide';

import type { GlzOption } from '../../lib/options';
import { testIds } from '../../lib/test-ids';
import { GlzIconButtonComponent } from '../icon-button/icon-button.component';
import { GlzIconComponent } from '../icon/icon.component';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { trigger: 'trigger', action: 'action' } as const;

/** Where the menu opens: below the button first, its start under the button's start. */
const POSITIONS: readonly ConnectedPosition[] = [
  { originX: 'start', originY: 'bottom', overlayX: 'start', overlayY: 'top', offsetY: 4 },
  { originX: 'start', originY: 'top', overlayX: 'start', overlayY: 'bottom', offsetY: -4 },
  { originX: 'end', originY: 'bottom', overlayX: 'end', overlayY: 'top', offsetY: 4 },
  { originX: 'end', originY: 'top', overlayX: 'end', overlayY: 'bottom', offsetY: -4 },
];

/**
 * A small ⋯ button that opens a menu of what can be done with something, such as a project in
 * a side nav: its `actions`, each emitted by value (`action`) when picked. The CDK's menu
 * (`menuitem`), so arrow keys, typeahead, Escape and focus return work. The button is named by
 * `label`.
 *
 * ```html
 * <glz-menu-button
 *   label="Options for Atlas"
 *   [actions]="[{ value: 'rename', label: 'Rename' }]"
 *   (action)="act($event)"
 * />
 * ```
 */
@Component({
  selector: 'glz-menu-button',
  imports: [CdkMenu, CdkMenuItem, CdkMenuTrigger, GlzIconButtonComponent, GlzIconComponent],
  templateUrl: './menu-button.component.html',
  styleUrl: './menu-button.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.data-testid]': 'testId()' },
})
export class GlzMenuButtonComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** What the menu is for, such as "Options for Atlas": names the button and the menu. */
  readonly label = input.required<string>();
  /** What can be done, in the menu's order; picking one emits its value. */
  readonly actions = input<readonly GlzOption[]>([]);
  /** Emitted with an action's value when the person picks it. */
  readonly action = output<string>();

  protected readonly positions = POSITIONS;
  protected readonly icons = { more: Ellipsis };

  protected act(value: string): void {
    this.action.emit(value);
  }
}
