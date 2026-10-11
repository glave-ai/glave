import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { ShowcaseLayoutComponent } from './showcase-layout.component';

describe('ShowcaseLayoutComponent', () => {
  it('shows tabs, breadcrumbs, the rail, the side nav and lists', async () => {
    const fixture = TestBed.createComponent(ShowcaseLayoutComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelectorAll('[role="tab"]')).toHaveLength(3);
    expect(root.querySelector('glz-breadcrumb[aria-current="page"]')?.textContent).toContain(
      'Plan the launch',
    );
    expect(root.querySelectorAll('glz-rail a')).toHaveLength(4);
    expect(root.querySelector('glz-sidenav-layout')).not.toBeNull();
    await expectAccessible(root);
  });
});
