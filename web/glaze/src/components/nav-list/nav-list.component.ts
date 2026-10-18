import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  contentChildren,
  inject,
  input,
  model,
} from '@angular/core';
import { ChevronDown } from 'lucide';

import { testIds } from '../../lib/test-ids';
import { injectId } from '../../tokens/ids';
import { GlzIconComponent } from '../icon/icon.component';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { toggle: 'toggle', caret: 'caret', end: 'end', items: 'items' } as const;

/**
 * A list of links in a side nav, under an optional heading that names the group for screen
 * readers: small and muted, its items' text at the same left edge. Its items are `glzNavItem`
 * links or buttons. A `collapsible` list's heading is a disclosure button that shows or hides
 * its items (`expanded`, bound two ways), with a caret just after its name that turns while
 * the list is closed. Actions for the group, such as "New thread here", go beside the heading
 * (`[glzNavListActions]`); they and the caret show while the pointer or focus is on the
 * heading, and always on a touch screen, the caret faintly. A mark goes at the heading's end
 * (`[glzNavListEnd]`), such as a dot for what waits in a closed list; it describes the toggle.
 *
 * A list inside another list is a group, such as a project: its heading is quieter than the
 * outer list's, and the groups sit further apart than the items in each.
 *
 * ```html
 * <glz-nav-list heading="Threads">
 *   <a glzNavItem routerLink="/t/1" [current]="true">Plan the launch</a>
 *   <a glzNavItem routerLink="/t/2">Fix the build <glz-badge glzNavItemEnd tone="warning">1</glz-badge></a>
 * </glz-nav-list>
 * <glz-nav-list heading="Projects" collapsible [(expanded)]="projects">
 *   <button glzNavListActions glzIconButton size="sm" label="New project">…</button>
 *   <glz-nav-list heading="glave" collapsible [(expanded)]="open">
 *     <button glzNavListActions glzIconButton size="sm" label="New thread in glave">…</button>
 *     <glz-badge glzNavListEnd tone="accent" variant="dot">needs you</glz-badge>
 *     …
 *   </glz-nav-list>
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
    '[class.glz-nav-list--group]': 'nested',
    '[class.glz-nav-list--groups]': 'hasGroups()',
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

  protected readonly icons = { caret: ChevronDown };
  protected readonly headingId = injectId('glz-nav-list');
  protected readonly itemsId = injectId('glz-nav-list-items');
  protected readonly endId = injectId('glz-nav-list-end');
  /** Whether this list is a group inside another, such as a project under Projects. */
  protected readonly nested =
    inject(GlzNavListComponent, { optional: true, skipSelf: true }) !== null;
  private readonly groups = contentChildren(GlzNavListComponent, { descendants: true });
  protected readonly hasGroups = computed(() => this.groups().length > 0);
  protected readonly hasHeading = computed(() => this.heading() !== '');
  /** Whether it has a header: a heading, or a collapsible list's toggle. */
  protected readonly hasHeader = computed(() => this.collapsible() || this.hasHeading());
  protected readonly labelledBy = computed(() => (this.hasHeading() ? this.headingId : null));
  protected readonly collapsed = computed(() => this.collapsible() && !this.expanded());

  protected toggle(): void {
    this.expanded.update((expanded) => !expanded);
  }
}
