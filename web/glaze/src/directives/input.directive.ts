import { Directive } from '@angular/core';

import { GlzFieldControlDirective } from './field-control.directive';

/**
 * Glaze's look and form-field wiring for a native `input`, `textarea` or `select`. The element
 * stays native, so the browser's keyboard, autofill and screen-reader behaviour all apply,
 * and Angular's own value accessors bind it to reactive forms, `ngModel` or signal forms. In a
 * `<glz-form-field>` the field's label names it and its hint or error describes it.
 *
 * Text, email, number, password, search, date and time inputs all work the same way.
 *
 * ```html
 * <glz-form-field label="Due">
 *   <input glzInput type="date" [formControl]="due" />
 * </glz-form-field>
 * ```
 */
@Directive({
  selector: 'input[glzInput], textarea[glzInput], select[glzInput]',
  hostDirectives: [GlzFieldControlDirective],
  host: { class: 'glz-input' },
})
export class GlzInputDirective {}
