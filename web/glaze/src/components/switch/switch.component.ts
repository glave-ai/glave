import { ChangeDetectionStrategy, Component } from '@angular/core';

import { GlzCheckableDirective } from '../../directives/checkable.directive';

/**
 * An on/off switch with its label, for a setting that takes effect at once. A native checkbox
 * with the `switch` role, so Space toggles it and screen readers say "on" or "off". Binds with
 * reactive forms, `ngModel`, signal forms or `[(checked)]`.
 *
 * ```html
 * <glz-switch [(checked)]="runInCloud">Run in the cloud</glz-switch>
 * ```
 */
@Component({
  selector: 'glz-switch',
  templateUrl: './switch.component.html',
  styleUrl: './switch.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GlzSwitchComponent extends GlzCheckableDirective {}
