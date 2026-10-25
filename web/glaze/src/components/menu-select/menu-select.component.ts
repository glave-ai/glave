import { CdkMenu, CdkMenuGroup, CdkMenuItemRadio, CdkMenuTrigger } from '@angular/cdk/menu';
import type { ConnectedPosition } from '@angular/cdk/overlay';
import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
  model,
} from '@angular/core';
import { Check, ChevronDown } from 'lucide';

import { GlzFocusOnPressDirective } from '../../directives/focus-on-press.directive';
import type { GlzIconData } from '../../lib/icons';
import { INERT_UNTIL_BOUND } from '../../lib/inert-until-bound';
import type { GlzOption } from '../../lib/options';
import { testIds } from '../../lib/test-ids';
import { GlzIconComponent } from '../icon/icon.component';
import { glzText } from '../../lib/words';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { trigger: 'trigger', option: 'option' } as const;

/** One choice in a menu select: a plain option, with a line about it and an icon. */
export interface GlzMenuOption extends GlzOption {
  /** A short line under the label, such as "Default". */
  readonly hint?: string;
  /** An icon before the label, on the button as in the menu, such as a folder for a project. */
  readonly icon?: GlzIconData;
  /** What the button shows once it is chosen, when that is shorter than its label. */
  readonly short?: string;
}

/** An option as the menu shows it. */
interface ShownOption {
  readonly value: string;
  readonly label: string;
  readonly hint: string;
  readonly hasHint: boolean;
  readonly icon: GlzIconData | null;
  readonly checked: boolean;
}

/** Which edge of the button the menu lines up with. */
type GlzMenuAlign = 'start' | 'end';

const END_ABOVE: ConnectedPosition = {
  originX: 'end',
  originY: 'top',
  overlayX: 'end',
  overlayY: 'bottom',
  offsetY: -4,
};
const END_BELOW: ConnectedPosition = {
  originX: 'end',
  originY: 'bottom',
  overlayX: 'end',
  overlayY: 'top',
  offsetY: 4,
};
const START_ABOVE: ConnectedPosition = {
  originX: 'start',
  originY: 'top',
  overlayX: 'start',
  overlayY: 'bottom',
  offsetY: -4,
};
const START_BELOW: ConnectedPosition = {
  originX: 'start',
  originY: 'bottom',
  overlayX: 'start',
  overlayY: 'top',
  offsetY: 4,
};

/**
 * Where the menu opens: above the button first (a composer sits at the foot of the page), lined
 * up with the `align` edge, and with the other edge only where that doesn't fit.
 */
const POSITIONS: Readonly<Record<GlzMenuAlign, readonly ConnectedPosition[]>> = {
  end: [END_ABOVE, END_BELOW, START_ABOVE, START_BELOW],
  start: [START_ABOVE, START_BELOW, END_ABOVE, END_BELOW],
};

/**
 * A compact choice among a few options: a quiet button that shows the current one and opens
 * a menu of the others, as a composer's model picker does. The CDK's menu with radio items
 * (`menuitemradio`), so arrow keys, typeahead, Escape and focus return work. The button is
 * named by `label` and the choice ("Model: Hope"). `value` is bound two ways. An `outlined`
 * one is a chip with a line round it, for a choice that sets where something goes rather than
 * how; a `quiet` one is small, muted text, for a composer's status line.
 *
 * ```html
 * <glz-menu-select label="Model" [options]="models" [(value)]="model" />
 * <glz-menu-select label="Where" outlined [options]="places" [(value)]="place" />
 * <glz-menu-select label="Permissions" quiet align="start" [options]="presets" [(value)]="preset" />
 * ```
 */
@Component({
  selector: 'glz-menu-select',
  imports: [
    CdkMenu,
    CdkMenuGroup,
    CdkMenuItemRadio,
    CdkMenuTrigger,
    GlzFocusOnPressDirective,
    GlzIconComponent,
  ],
  templateUrl: './menu-select.component.html',
  styleUrl: './menu-select.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    ...INERT_UNTIL_BOUND,
    '[attr.data-testid]': 'testId()',
    '[class.glz-menu-select--outlined]': 'outlined()',
    '[class.glz-menu-select--quiet]': 'quiet()',
  },
})
export class GlzMenuSelectComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));
  private readonly words = glzText('menuSelect', {
    placeholder: 'Choose',
  });

  /** The chosen option's value, or `null` for none. */
  readonly value = model<string | null>(null);
  readonly options = input<readonly GlzMenuOption[]>([]);
  /** What is being chosen, such as "Model": names the button with the choice. */
  readonly label = input.required<string>();
  /** Shown on the button while nothing is chosen. */
  readonly placeholder = input(this.words.placeholder);
  readonly disabled = input(false, { transform: booleanAttribute });
  /** Whether the button is a chip with a line round it. */
  readonly outlined = input(false, { transform: booleanAttribute });
  /** Whether the button is small, muted text, as in a status line. */
  readonly quiet = input(false, { transform: booleanAttribute });
  /**
   * Which edge of the button the menu lines up with: `end` for a button at the end of a row,
   * as a composer's model picker is; `start` for one at its start, so the menu spreads over
   * the row rather than past its edge onto what is beside it.
   */
  readonly align = input<GlzMenuAlign>('end');

  protected readonly positions = computed(() => POSITIONS[this.align()]);
  /** What the button is for, before the choice, for screen readers: "Model: ". */
  protected readonly prefix = computed(() => `${this.label()}: `);
  protected readonly icons = { open: ChevronDown, check: Check };
  private readonly current = computed(() =>
    this.options().find((option) => option.value === this.value()),
  );
  protected readonly shownLabel = computed(() => {
    const current = this.current();
    return current === undefined ? this.placeholder() : (current.short ?? current.label);
  });
  protected readonly shownIcon = computed(() => this.current()?.icon ?? null);
  protected readonly shown = computed<readonly ShownOption[]>(() =>
    this.options().map((option) => ({
      value: option.value,
      label: option.label,
      hint: option.hint ?? '',
      hasHint: (option.hint ?? '') !== '',
      icon: option.icon ?? null,
      checked: option.value === this.value(),
    })),
  );

  protected pick(value: string): void {
    this.value.set(value);
  }
}
