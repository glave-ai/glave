import { ChangeDetectionStrategy, Component, input } from '@angular/core';

/**
 * A small spinning ring: something is under way and will not take long. `label` says what,
 * for screen readers.
 *
 * ```html
 * <glz-spinner label="Loading threads" />
 * ```
 */
@Component({
  selector: 'glz-spinner',
  templateUrl: './spinner.component.html',
  styleUrl: './spinner.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    role: 'progressbar',
    '[attr.aria-label]': 'label()',
  },
})
export class GlzSpinnerComponent {
  readonly label = input('Loading');
}
