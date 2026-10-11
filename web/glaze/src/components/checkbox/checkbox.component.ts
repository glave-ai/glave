import { ChangeDetectionStrategy, Component, booleanAttribute, input } from '@angular/core';

import { GlzCheckableDirective } from '../../directives/checkable.directive';

/**
 * A checkbox with its label: a native checkbox (keyboard, screen readers and forms all as the
 * browser has them) drawn in Glaze's style. Binds with reactive forms, `ngModel`, signal
 * forms or `[(checked)]`.
 *
 * ```html
 * <glz-checkbox [(checked)]="remember">Remember this choice</glz-checkbox>
 * <glz-checkbox [formControl]="terms">I agree</glz-checkbox>
 * ```
 */
@Component({
  selector: 'glz-checkbox',
  templateUrl: './checkbox.component.html',
  styleUrl: './checkbox.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class GlzCheckboxComponent extends GlzCheckableDirective {
  /** Shown as partly checked, as for a "select all" over a mixed selection. */
  readonly indeterminate = input(false, { transform: booleanAttribute });
}
