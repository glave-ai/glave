import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  input,
  computed,
} from '@angular/core';

import { GlzCheckableDirective } from '../../directives/checkable.directive';
import { testIds } from '../../lib/test-ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { input: 'input' } as const;

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
  host: { '[attr.data-testid]': 'testId()' },
})
export class GlzCheckboxComponent extends GlzCheckableDirective {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));

  /** Shown as partly checked, as for a "select all" over a mixed selection. */
  readonly indeterminate = input(false, { transform: booleanAttribute });
}
