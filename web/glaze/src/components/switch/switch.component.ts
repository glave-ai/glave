import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { GlzCheckableDirective } from '../../directives/checkable.directive';
import { testIds } from '../../lib/test-ids';

/** The parts end-to-end tests reach, as `<testId>-<part>`. */
const PARTS = { input: 'input' } as const;

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
  host: { '[attr.data-testid]': 'testId()' },
})
export class GlzSwitchComponent extends GlzCheckableDirective {
  /** The test hook for this component; its parts get `<testId>-<part>`. */
  readonly testId = input<string | null>(null);
  protected readonly ids = computed(() => testIds(this.testId(), PARTS));
}
