import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { AccentShadeDirective } from './accent-shade.directive';

@Component({
  imports: [AccentShadeDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<span appAccentShade="#0f766e"></span>`,
})
class HostComponent {}

describe('AccentShadeDirective', () => {
  it('sets the shade for what is inside', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const shaded = (fixture.nativeElement as HTMLElement).querySelector('span');
    expect(shaded?.style.getPropertyValue('--shade')).toBe('#0f766e');
  });
});
