import { CdkMenuItem } from '@angular/cdk/menu';
import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import type { GlzIconData } from '../../lib/icons';
import { GlzIconComponent } from '../icon/icon.component';

/**
 * An action in a `<glz-menu>`. `triggered` fires when it is chosen, by click, Enter or Space;
 * the menu then closes. `tone="danger"` for an action that destroys something.
 *
 * ```html
 * <button glzMenuItem [icon]="trashIcon" tone="danger" (triggered)="remove()">Delete</button>
 * ```
 */
@Component({
  selector: 'button[glzMenuItem]',
  imports: [GlzIconComponent],
  hostDirectives: [
    {
      directive: CdkMenuItem,
      inputs: ['cdkMenuItemDisabled: disabled'],
      outputs: ['cdkMenuItemTriggered: triggered'],
    },
  ],
  templateUrl: './menu-item.component.html',
  styleUrl: './menu-item.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[class]': 'toneClass()' },
})
export class GlzMenuItemComponent {
  readonly icon = input<GlzIconData | undefined>(undefined);
  readonly tone = input<'neutral' | 'danger'>('neutral');

  protected readonly toneClass = computed(() => `glz-tone-${this.tone()}`);
}
