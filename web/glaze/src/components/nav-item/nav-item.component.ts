import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  computed,
  input,
} from '@angular/core';

import type { GlzIconData } from '../../lib/icons';
import { GlzIconComponent } from '../icon/icon.component';

/**
 * One link or button in a `glz-nav-list`: an optional icon, or anything else in its place
 * (`[glzNavItemStart]`, such as an avatar), its text, and anything at the end
 * (`[glzNavItemEnd]`, such as a badge or a status). `current` marks the page shown now.
 *
 * ```html
 * <a glzNavItem routerLink="/t/2" [current]="open()">Fix the build <glz-badge glzNavItemEnd>1</glz-badge></a>
 * <button glzNavItem><glz-avatar glzNavItemStart name="Ada" /> Ada</button>
 * ```
 */
@Component({
  selector: 'a[glzNavItem], button[glzNavItem]',
  imports: [GlzIconComponent],
  templateUrl: './nav-item.component.html',
  styleUrl: './nav-item.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.aria-current]': 'ariaCurrent()' },
})
export class GlzNavItemComponent {
  readonly icon = input<GlzIconData | undefined>(undefined);
  readonly current = input(false, { transform: booleanAttribute });

  protected readonly ariaCurrent = computed(() => (this.current() ? 'page' : null));
}
