import { ActiveDescendantKeyManager, type Highlightable } from '@angular/cdk/a11y';
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  type ElementRef,
  Injector,
  afterRenderEffect,
  booleanAttribute,
  computed,
  inject,
  input,
  model,
  signal,
  viewChild,
} from '@angular/core';
import { Check, ChevronDown, Pin, PinOff } from 'lucide';

import { type GlzPickerGroup, pickerGroups } from '../../lib/picker';
import { positionPopover } from '../../lib/popover';
import { testIds } from '../../lib/test-ids';
import { injectId } from '../../tokens/ids';
import type { GlzMenuOption } from '../menu-select/menu-select.component';
import { GlzIconComponent } from '../icon/icon.component';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = {
  trigger: 'trigger',
  search: 'search',
  listbox: 'listbox',
  option: 'option',
  pin: 'pin',
  empty: 'empty',
} as const;

/** An option as the list shows it. */
interface ShownOption {
  readonly value: string;
  readonly label: string;
  readonly hint: string;
  readonly hasHint: boolean;
  readonly hope: boolean;
  readonly id: string;
  readonly selected: boolean;
  readonly active: boolean;
  readonly pinned: boolean;
}

interface ShownGroup {
  readonly key: string;
  readonly heading: string;
  readonly headingId: string;
  readonly options: readonly ShownOption[];
}

/**
 * A choice among many options, searched as you type: a quiet button that shows the current one
 * and opens a popover with a search box over a scrolling list, the ARIA combobox pattern with a
 * listbox popup. The keyboard model is the CDK's (ActiveDescendantKeyManager): arrow keys,
 * Page Up and Page Down move through the matches, Enter picks, Escape closes, focus goes back
 * to the button. Options can be pinned to the top (`pinned`, bound two ways; keeping them is
 * the app's job): by the pin on each option, or by the Pin button under the list, which pins
 * the highlighted one. An option with `hope` wears Hope's sun: set it only for Hope.
 *
 * ```html
 * <glz-picker label="Model" [options]="models" [(value)]="model" [(pinned)]="pinned" />
 * ```
 */
@Component({
  selector: 'glz-picker',
  imports: [GlzIconComponent],
  templateUrl: './picker.component.html',
  styleUrl: './picker.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.data-testid]': 'testId()' },
})
export class GlzPickerComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** The chosen option's value, or `null` for none. */
  readonly value = model<string | null>(null);
  /** The values pinned to the top, in the order they were pinned. */
  readonly pinned = model<readonly string[]>([]);
  readonly options = input<readonly GlzMenuOption[]>([]);
  /** What is being chosen, such as "Model": names the button, the popover and the list. */
  readonly label = input.required<string>();
  /** Shown on the button while nothing is chosen. */
  readonly placeholder = input('Choose');
  /** Names the search box, and is its placeholder. */
  readonly searchLabel = input('Search');
  /** Shown in the list when nothing matches. */
  readonly emptyText = input('No matches');
  /** The heading over the pinned options. */
  readonly pinnedHeading = input('Pinned');
  /** The heading over the rest. */
  readonly allHeading = input('All');
  /** The Pin button's words, before the highlighted option's name. */
  readonly pinText = input('Pin');
  readonly unpinText = input('Unpin');
  readonly disabled = input(false, { transform: booleanAttribute });

  protected readonly popoverId = injectId('glz-picker');
  protected readonly listboxId = `${this.popoverId}-list`;
  protected readonly icons = { open: ChevronDown, check: Check, pin: Pin, unpin: PinOff };
  protected readonly query = signal('');
  protected readonly open = signal(false);
  private readonly activeIndex = signal(-1);
  private readonly trigger = viewChild.required<ElementRef<HTMLButtonElement>>('trigger');
  private readonly popover = viewChild.required<ElementRef<HTMLElement>>('popover');
  private readonly search = viewChild.required<ElementRef<HTMLInputElement>>('search');

  protected readonly prefix = computed(() => `${this.label()}: `);
  private readonly current = computed(() =>
    this.options().find((option) => option.value === this.value()),
  );
  protected readonly shownLabel = computed(() => this.current()?.label ?? this.placeholder());
  protected readonly hope = computed(() => this.current()?.hope === true);
  private readonly groups = computed<readonly GlzPickerGroup[]>(() =>
    pickerGroups(this.options(), this.pinned(), this.query()),
  );
  /** The matches in the order the list shows them, pinned first. */
  private readonly flat = computed(() => this.groups().flatMap((group) => group.options));
  protected readonly shownGroups = computed<readonly ShownGroup[]>(() => {
    let index = 0;
    return this.groups().map((group) => ({
      key: group.kind,
      heading: group.kind === 'pinned' ? this.pinnedHeading() : this.allHeading(),
      headingId: `${this.popoverId}-${group.kind}`,
      options: group.options.map((option) => {
        const at = index;
        index += 1;
        return {
          value: option.value,
          label: option.label,
          hint: option.hint ?? '',
          hasHint: (option.hint ?? '') !== '',
          hope: option.hope === true,
          id: `${this.popoverId}-${String(at)}`,
          selected: option.value === this.value(),
          active: at === this.activeIndex(),
          pinned: group.kind === 'pinned',
        };
      }),
    }));
  });
  protected readonly empty = computed(() => this.flat().length === 0);
  protected readonly expanded = computed(() => (this.open() ? 'true' : 'false'));
  private readonly active = computed(() => this.flat()[this.activeIndex()] ?? null);
  protected readonly activeId = computed(() => {
    const active = this.active();
    return active === null ? null : `${this.popoverId}-${String(this.activeIndex())}`;
  });
  protected readonly cannotPin = computed(() => this.active() === null);
  protected readonly pinButton = computed(() => {
    const active = this.active();
    if (active === null) {
      return this.pinText();
    }
    const verb = this.pinned().includes(active.value) ? this.unpinText() : this.pinText();
    return `${verb} ${active.label}`;
  });

  private readonly highlightables = computed<readonly Highlightable[]>(() =>
    this.flat().map(() => ({
      setActiveStyles: () => undefined,
      setInactiveStyles: () => undefined,
    })),
  );
  private readonly keys = new ActiveDescendantKeyManager(this.highlightables, inject(Injector))
    .withVerticalOrientation()
    .withPageUpDown();

  constructor() {
    const changes = this.keys.change.subscribe((index) => {
      this.activeIndex.set(index);
    });
    inject(DestroyRef).onDestroy(() => {
      changes.unsubscribe();
      this.keys.destroy();
    });
    // The highlighted option stays in view as the keys move through a long list.
    afterRenderEffect(() => {
      const id = this.activeId();
      if (id !== null && this.open()) {
        this.popover()
          .nativeElement.querySelector(`[id="${id}"]`)
          ?.scrollIntoView({ block: 'nearest' });
      }
    });
  }

  /** Opens the list with the search box focused and the current option highlighted. */
  protected show(): void {
    const popover = this.popover().nativeElement;
    this.query.set('');
    this.open.set(true);
    popover.showPopover();
    positionPopover(this.trigger().nativeElement, popover, 'above', `--${this.popoverId}`);
    const at = this.flat().findIndex((option) => option.value === this.value());
    this.activate(at === -1 ? 0 : at);
    this.search().nativeElement.focus();
  }

  /** Follows the popover's own closing: Escape or a click outside. */
  protected toggled(event: Event): void {
    if ('newState' in event && event.newState === 'closed') {
      this.open.set(false);
    }
  }

  protected typing(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.query.set(event.target.value);
      this.activate(0);
    }
  }

  protected keydown(event: KeyboardEvent): void {
    if (event.key === 'Enter') {
      const active = this.active();
      event.preventDefault();
      if (active !== null) {
        this.choose(active.value);
      }
      return;
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      this.close();
      return;
    }
    this.keys.onKeydown(event);
  }

  /** Keeps focus in the search box as the pointer picks or pins. */
  protected keepFocus(event: Event): void {
    event.preventDefault();
  }

  protected choose(value: string): void {
    this.value.set(value);
    this.close();
  }

  /** Pins or unpins `value`; without one, the highlighted option. */
  protected togglePin(event: Event, value?: string): void {
    event.stopPropagation();
    const target = value ?? this.active()?.value;
    if (target === undefined) {
      return;
    }
    this.pinned.update((pinned) =>
      pinned.includes(target) ? pinned.filter((each) => each !== target) : [...pinned, target],
    );
    const at = this.flat().findIndex((option) => option.value === target);
    this.activate(at);
  }

  private close(): void {
    this.popover().nativeElement.hidePopover();
    this.open.set(false);
    this.trigger().nativeElement.focus();
  }

  private activate(index: number): void {
    if (index < 0 || this.flat().length === 0) {
      this.keys.setActiveItem(-1);
      this.activeIndex.set(-1);
      return;
    }
    this.keys.setActiveItem(Math.min(index, this.flat().length - 1));
  }
}
