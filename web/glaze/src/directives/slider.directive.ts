import { Directive, input } from '@angular/core';

import { GlzFieldControlDirective } from './field-control.directive';

/**
 * A slider: a native range input (arrow keys, Page Up/Down, Home/End and screen readers as
 * the browser has them) in Glaze's accent, wired to a `<glz-form-field>`. Give `valueText`
 * when the number alone does not say what it means.
 *
 * ```html
 * <glz-form-field label="Budget">
 *   <input type="range" glzSlider min="0" max="100" [formControl]="budget" [valueText]="budgetText()" />
 * </glz-form-field>
 * ```
 */
@Directive({
  selector: 'input[type="range"][glzSlider]',
  hostDirectives: [GlzFieldControlDirective],
  host: {
    class: 'glz-slider',
    '[attr.aria-valuetext]': 'valueText()',
  },
})
export class GlzSliderDirective {
  /** What the value means, read by screen readers instead of the bare number. */
  readonly valueText = input<string | null>(null);
}
