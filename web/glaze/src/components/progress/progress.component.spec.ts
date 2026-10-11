import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzProgressComponent } from './progress.component';

describe('GlzProgressComponent', () => {
  it('reports a value, kept between 0 and 100', async () => {
    const fixture = TestBed.createComponent(GlzProgressComponent);
    fixture.componentRef.setInput('label', 'Budget used');
    fixture.componentRef.setInput('value', 140);
    await fixture.whenStable();
    const host: HTMLElement = fixture.nativeElement;
    expect(host.getAttribute('role')).toBe('progressbar');
    expect(host.getAttribute('aria-valuenow')).toBe('100');
    expect(host.style.getPropertyValue('--glz-progress-value')).toBe('100%');
    await expectAccessible(host);
  });

  it('is indeterminate without a value', async () => {
    const fixture = TestBed.createComponent(GlzProgressComponent);
    fixture.componentRef.setInput('label', 'Uploading');
    await fixture.whenStable();
    const host: HTMLElement = fixture.nativeElement;
    expect(host.hasAttribute('aria-valuenow')).toBe(false);
    expect(host.classList).toContain('indeterminate');
  });
});
