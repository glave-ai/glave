import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzSpinnerComponent } from './spinner.component';

describe('GlzSpinnerComponent', () => {
  it('says what is loading', async () => {
    const fixture = TestBed.createComponent(GlzSpinnerComponent);
    fixture.componentRef.setInput('label', 'Loading threads');
    await fixture.whenStable();
    const host: HTMLElement = fixture.nativeElement;
    expect(host.getAttribute('role')).toBe('progressbar');
    expect(host.getAttribute('aria-label')).toBe('Loading threads');
    await expectAccessible(host);
  });
});
