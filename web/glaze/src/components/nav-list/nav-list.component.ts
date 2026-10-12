import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
  model,
} from '@angular/core';
import { ChevronDown, ChevronRight } from 'lucide';

import { testIds } from '../../lib/test-ids';
import { injectId } from '../../tokens/ids';
import { GlzIconComponent } from '../icon/icon.component';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { toggle: 'toggle', items: 'items' } as const;

/**
 * A list of links in a side nav, under an optional heading that names the group for screen
 * readers. Its items are `glzNavItem` links or buttons. A `collapsible` list's heading is a
 * disclosure button that shows or hides its items (`expanded`, bound two ways); actions for
 * the group, such as "New thread here", go at the heading's end (`[glzNavListActions]`).
 *
 * ```html
 * <glz-nav-list heading="Threads">
 *   <a glzNavItem routerLink="/t/1" [current]="true">Plan the launch</a>
 *   <a glzNavItem routerLink="/t/2">Fix the build <glz-badge glzNavItemEnd tone="warning">1</glz-badge></a>
 * </glz-nav-list>
 * <glz-nav-list heading="glave" collapsible [(expanded)]="open">
 *   <button glzNavListActions glzIconButton size="sm" label="New thread in glave">…</button>
 *   …
 * </glz-nav-list>
 * ```
 */
@Component({
  selector: 'glz-nav-list',
  imports: [GlzIconComponent],
  templateUrl: './nav-list.component.html',
  styleUrl: './nav-list.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    role: 'group',
    '[attr.aria-labelledby]': 'labelledBy()',
    '[attr.data-testid]': 'testId()',
  },
})
export class GlzNavListComponent {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  readonly heading = input('');
  /** Whether the heading shows and hides the items. */
  readonly collapsible = input(false, { transform: booleanAttribute });
  /** Whether a collapsible list shows its items. */
  readonly expanded = model(true);

  protected readonly headingId = injectId('glz-nav-list');
  protected readonly itemsId = injectId('glz-nav-list-items');
  protected readonly hasHeading = computed(() => this.heading() !== '');
  protected readonly labelledBy = computed(() => (this.hasHeading() ? this.headingId : null));
  protected readonly collapsed = computed(() => this.collapsible() && !this.expanded());
  protected readonly chevron = computed(() => (this.expanded() ? ChevronDown : ChevronRight));

  protected toggle(): void {
    this.expanded.update((expanded) => !expanded);
  }
}
