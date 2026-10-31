import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { GlzSwatchColorDirective } from './swatch-color.directive';

@Component({
  imports: [GlzSwatchColorDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<span glzSwatchColor="#0f766e"></span>`,
})
class HostComponent {}

describe('GlzSwatchColorDirective', () => {
  it('fills what it is on with the color', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const swatch = (fixture.nativeElement as HTMLElement).querySelector('span');
    expect(swatch?.style.backgroundColor).toBe('rgb(15, 118, 110)');
  });
});
