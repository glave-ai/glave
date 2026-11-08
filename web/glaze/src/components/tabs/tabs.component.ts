import { FocusKeyManager, type FocusableOption } from '@angular/cdk/a11y';
import { Directionality } from '@angular/cdk/bidi';
import { NgTemplateOutlet } from '@angular/common';
import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  type ElementRef,
  Injector,
  type TemplateRef,
  booleanAttribute,
  computed,
  contentChildren,
  inject,
  input,
  model,
  viewChildren,
} from '@angular/core';

import { injectId } from '../../tokens/ids';
import { GlzTabComponent } from '../tab/tab.component';
import { testIds } from '../../lib/test-ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { tab: 'tab', panel: 'panel' } as const;

/** A tab as the tab list shows it. */
interface ShownTab {
  readonly index: number;
  readonly label: string;
  readonly tabId: string;
  readonly panelId: string;
  readonly selected: boolean;
  readonly tabIndex: number;
  readonly controls: string | null;
  readonly content: TemplateRef<unknown>;
}

/**
 * Tabs: a row of `<glz-tab>`s and the panel of the one selected. The ARIA tabs pattern:
 * arrow keys move between tabs and select them, Home and End jump to the ends, and Tab moves
 * into the panel. `selected` is the selected tab's index, bound two ways; `fill` makes them fill
 * what holds them.
 *
 * ```html
 * <glz-tabs label="Thread details" [(selected)]="tab">
 *   <glz-tab label="Conversation">…</glz-tab>
 *   <glz-tab label="Events">…</glz-tab>
 * </glz-tabs>
 * ```
 */
@Component({
  selector: 'glz-tabs',
  imports: [NgTemplateOutlet],
  templateUrl: './tabs.component.html',
  styleUrl: './tabs.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.data-testid]': 'testId()', '[class.fill]': 'fill()' },
})
export class GlzTabsComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** The selected tab's index. */
  readonly selected = model(0);
  /** Names the tab list for screen readers. */
  readonly label = input.required<string>();
  /**
   * Whether the tabs fill what holds them, their panel taking the room the list leaves, with no
   * inset of its own, as a side panel's do: what is in it lays itself out in that room.
   */
  readonly fill = input(false, { transform: booleanAttribute });

  private readonly id = injectId('glz-tabs');
  private readonly tabs = contentChildren(GlzTabComponent);
  private readonly buttons = viewChildren<ElementRef<HTMLButtonElement>>('tab');

  protected readonly shown = computed<readonly ShownTab[]>(() =>
    this.tabs().map((tab, index) => {
      const selected = index === this.selected();
      const panelId = `${this.id}-panel-${String(index)}`;
      return {
        index,
        label: tab.label(),
        tabId: `${this.id}-tab-${String(index)}`,
        panelId,
        selected,
        tabIndex: selected ? 0 : -1,
        controls: selected ? panelId : null,
        content: tab.content(),
      };
    }),
  );
  protected readonly current = computed(() => this.shown().find((tab) => tab.selected));

  /** The CDK's roving focus over the tab buttons: arrows (as the text reads), Home, End. */
  private readonly focusables = computed<readonly FocusableOption[]>(() =>
    this.buttons().map((button) => ({
      focus: () => {
        button.nativeElement.focus();
      },
    })),
  );
  private readonly keys = new FocusKeyManager(this.focusables, inject(Injector))
    .withHorizontalOrientation(inject(Directionality).value)
    .withHomeAndEnd()
    .withWrap();

  constructor() {
    const changes = this.keys.change.subscribe((index) => {
      this.selected.set(index);
    });
    inject(DestroyRef).onDestroy(() => {
      changes.unsubscribe();
      this.keys.destroy();
    });
  }

  protected select(index: number): void {
    this.selected.set(index);
  }

  protected keydown(event: KeyboardEvent): void {
    this.keys.updateActiveItem(this.selected());
    this.keys.onKeydown(event);
  }
}
