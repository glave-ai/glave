import { Directive, input } from '@angular/core';

/** Fills a swatch with the color it stands for. */
@Directive({
  selector: '[glzSwatchColor]',
  host: { '[style.background-color]': 'glzSwatchColor()' },
})
export class GlzSwatchColorDirective {
  /** The CSS color to show. */
  readonly glzSwatchColor = input.required<string>();
}
