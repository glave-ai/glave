import {
  CdkMenu,
  CdkMenuGroup,
  CdkMenuItem,
  CdkMenuItemRadio,
  CdkMenuTrigger,
} from '@angular/cdk/menu';
import type { ConnectedPosition } from '@angular/cdk/overlay';
import { ChangeDetectionStrategy, Component, computed, input, model, output } from '@angular/core';
import { Check, Ellipsis } from 'lucide';

import type { GlzOption } from '../../lib/options';
import { testIds } from '../../lib/test-ids';
import { injectId } from '../../tokens/ids';
import { GlzIconButtonComponent } from '../icon-button/icon-button.component';
import { GlzIconComponent } from '../icon/icon.component';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { trigger: 'trigger', action: 'action', choice: 'choice' } as const;

/** A choice as the menu shows it. */
interface ShownChoice {
  readonly value: string;
  readonly label: string;
  readonly checked: boolean;
}

/** Where the menu opens: below the button first, its start under the button's start. */
const POSITIONS: readonly ConnectedPosition[] = [
  { originX: 'start', originY: 'bottom', overlayX: 'start', overlayY: 'top', offsetY: 4 },
  { originX: 'start', originY: 'top', overlayX: 'start', overlayY: 'bottom', offsetY: -4 },
  { originX: 'end', originY: 'bottom', overlayX: 'end', overlayY: 'top', offsetY: 4 },
  { originX: 'end', originY: 'top', overlayX: 'end', overlayY: 'bottom', offsetY: -4 },
];

/**
 * A small ⋯ button that opens a menu of what can be done with something, such as a project in
 * a side nav: its `actions`, each emitted by value (`action`) when picked, then one choice
 * under its `choiceLabel`, such as how a list is sorted, its `choice` bound two ways. The
 * CDK's menu (`menuitem`, `menuitemradio`), so arrow keys, typeahead, Escape and focus return
 * work. The button is named by `label`.
 *
 * ```html
 * <glz-menu-button
 *   label="Options for glave"
 *   [actions]="[{ value: 'rename', label: 'Rename' }]"
 *   choiceLabel="Sort by"
 *   [choices]="sorts"
 *   [(choice)]="sort"
 *   (action)="act($event)"
 * />
 * ```
 */
@Component({
  selector: 'glz-menu-button',
  imports: [
    CdkMenu,
    CdkMenuGroup,
    CdkMenuItem,
    CdkMenuItemRadio,
    CdkMenuTrigger,
    GlzIconButtonComponent,
    GlzIconComponent,
  ],
  templateUrl: './menu-button.component.html',
  styleUrl: './menu-button.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.data-testid]': 'testId()' },
})
export class GlzMenuButtonComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** What the menu is for, such as "Options for glave": names the button and the menu. */
  readonly label = input.required<string>();
  /** What can be done, in the menu's order; picking one emits its value. */
  readonly actions = input<readonly GlzOption[]>([]);
  /** What the choice under the actions chooses, such as "Sort by". */
  readonly choiceLabel = input('');
  /** The choice's options; none, and the menu has no choice. */
  readonly choices = input<readonly GlzOption[]>([]);
  /** The chosen option's value. */
  readonly choice = model<string | null>(null);
  /** Emitted with an action's value when the person picks it. */
  readonly action = output<string>();

  protected readonly positions = POSITIONS;
  protected readonly icons = { more: Ellipsis, check: Check };
  protected readonly choiceId = injectId('glz-menu-button-choice');
  protected readonly hasChoices = computed(() => this.choices().length > 0);
  protected readonly shownChoices = computed<readonly ShownChoice[]>(() =>
    this.choices().map((option) => ({
      value: option.value,
      label: option.label,
      checked: option.value === this.choice(),
    })),
  );

  protected act(value: string): void {
    this.action.emit(value);
  }

  protected choose(value: string): void {
    this.choice.set(value);
  }
}
