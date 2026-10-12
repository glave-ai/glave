import { TestBed } from '@angular/core/testing';

import { GlzBreadcrumbComponent } from './breadcrumb.component';

describe('GlzBreadcrumbComponent', () => {
  it('is a list item, the current page when marked', async () => {
    const fixture = TestBed.createComponent(GlzBreadcrumbComponent);
    await fixture.whenStable();
    const host: HTMLElement = fixture.nativeElement;
    expect(host.getAttribute('role')).toBe('listitem');
    expect(host.hasAttribute('aria-current')).toBe(false);
    fixture.componentRef.setInput('current', true);
    await fixture.whenStable();
    expect(host.getAttribute('aria-current')).toBe('page');
  });
});
