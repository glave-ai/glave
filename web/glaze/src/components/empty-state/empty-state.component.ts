import { ChangeDetectionStrategy, Component, input } from '@angular/core';

import type { GlzIconData } from '../../lib/icons';
import { GlzIconComponent } from '../icon/icon.component';

/**
 * What to show where a list or view has nothing yet: an icon, a heading, a line on what
 * goes here, and the action that starts it.
 *
 * ```html
 * <glz-empty-state [icon]="messagesIcon" heading="No threads yet">
 *   Start one and it shows up here.
 *   <button glzEmptyStateAction glzButton variant="primary">New chat</button>
 * </glz-empty-state>
 * ```
 */
@Component({
  selector: 'glz-empty-state',
  imports: [GlzIconComponent],
  templateUrl: './empty-state.component.html',
  styleUrl: './empty-state.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GlzEmptyStateComponent {
  readonly heading = input.required<string>();
  readonly icon = input<GlzIconData | undefined>(undefined);
}
