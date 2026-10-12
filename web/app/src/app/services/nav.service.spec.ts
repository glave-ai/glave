import { TestBed } from '@angular/core/testing';

import { NavService } from './nav.service';

describe('NavService', () => {
  it('shows the side nav at first, and toggles it', () => {
    const nav = TestBed.inject(NavService);
    expect(nav.open()).toBe(true);
    nav.toggle();
    expect(nav.open()).toBe(false);
    nav.toggle();
    expect(nav.open()).toBe(true);
  });
});
