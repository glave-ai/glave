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
 * One destination in a `glz-rail`: an icon over a short label, on a link or a button.
 * `current` marks the one shown now (`aria-current="page"`).
 *
 * ```html
 * <a glzRailItem routerLink="/memory" [icon]="memoryIcon" label="Memory" [current]="here()"></a>
 * ```
 */
@Component({
  selector: 'a[glzRailItem], button[glzRailItem]',
  imports: [GlzIconComponent],
  templateUrl: './rail-item.component.html',
  styleUrl: './rail-item.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.aria-current]': 'ariaCurrent()' },
})
export class GlzRailItemComponent {
  readonly icon = input.required<GlzIconData>();
  readonly label = input.required<string>();
  readonly current = input(false, { transform: booleanAttribute });

  protected readonly ariaCurrent = computed(() => (this.current() ? 'page' : null));
}
