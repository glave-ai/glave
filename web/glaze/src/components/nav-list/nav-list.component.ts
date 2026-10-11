import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { injectId } from '../../lib/ids';

/**
 * A list of links in a side nav, under an optional heading that names the group for screen
 * readers. Its items are `glzNavItem` links or buttons.
 *
 * ```html
 * <glz-nav-list heading="Threads">
 *   <a glzNavItem routerLink="/t/1" [current]="true">Plan the launch</a>
 *   <a glzNavItem routerLink="/t/2">Fix the build <glz-badge glzNavItemEnd tone="warning">1</glz-badge></a>
 * </glz-nav-list>
 * ```
 */
@Component({
  selector: 'glz-nav-list',
  templateUrl: './nav-list.component.html',
  styleUrl: './nav-list.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    role: 'group',
    '[attr.aria-labelledby]': 'labelledBy()',
  },
})
export class GlzNavListComponent {
  readonly heading = input('');

  protected readonly headingId = injectId('glz-nav-list');
  protected readonly hasHeading = computed(() => this.heading() !== '');
  protected readonly labelledBy = computed(() => (this.hasHeading() ? this.headingId : null));
}
