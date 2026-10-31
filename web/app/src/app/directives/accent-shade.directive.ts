import { Directive, input } from '@angular/core';

/** Sets the shade of the accent what it is on, and what is inside it, shows (`--shade`). */
@Directive({
  selector: '[appAccentShade]',
  host: { '[style.--shade]': 'appAccentShade()' },
})
export class AccentShadeDirective {
  readonly appAccentShade = input.required<string>();
}
