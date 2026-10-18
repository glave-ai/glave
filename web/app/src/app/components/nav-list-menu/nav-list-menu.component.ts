import { ChangeDetectionStrategy, Component, input, model, output } from '@angular/core';
import { GlzMenuButtonComponent, type GlzOption } from '@glave/glaze';

import type { NavSort } from '../../lib/nav';

const COPY = { sortBy: 'Sort by' } as const;

/** How a list can be sorted, as its menu offers it. */
const SORTS: readonly GlzOption[] = [
  { value: 'recent', label: 'Recent activity' },
  { value: 'name', label: 'Name' },
];

/**
 * A list's ⋯ in the side nav: what can be done with it (a project's Rename), then how it is
 * sorted, by recent activity or by name. Its own file, so the side nav can load it, and the
 * CDK's menu with it, once the page is idle rather than in the app's first load.
 */
@Component({
  selector: 'app-nav-list-menu',
  imports: [GlzMenuButtonComponent],
  templateUrl: './nav-list-menu.component.html',
  styleUrl: './nav-list-menu.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class NavListMenuComponent {
  /** The test hook of its menu button; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  /** What the menu is for, such as "Options for glave". */
  readonly label = input.required<string>();
  /** What can be done with the list besides sorting it. */
  readonly actions = input<readonly GlzOption[]>([]);
  /** How the list is sorted. */
  readonly sort = model<NavSort>('recent');
  /** Emitted with an action's value when the person picks it. */
  readonly action = output<string>();

  protected readonly copy = COPY;
  protected readonly sorts = SORTS;

  protected sortBy(value: string | null): void {
    this.sort.set(value === 'name' ? 'name' : 'recent');
  }

  protected act(value: string): void {
    this.action.emit(value);
  }
}
