import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { ShowcaseLayoutComponent } from './showcase-layout.component';

describe('ShowcaseLayoutComponent', () => {
  it('shows tabs, the side nav, lists and a panel', async () => {
    const fixture = TestBed.createComponent(ShowcaseLayoutComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelectorAll('[role="tab"]')).toHaveLength(3);
    expect(root.querySelector('glz-sidenav-layout')).not.toBeNull();
    expect(root.querySelector('[data-testid="showcase-panel-handle"]')).not.toBeNull();
    await expectAccessible(root);
    root.querySelector<HTMLButtonElement>('[data-testid="showcase-earlier-toggle"]')?.click();
    root.querySelector<HTMLButtonElement>('[data-testid="showcase-details-toggle"]')?.click();
    await fixture.whenStable();
    expect(
      root.querySelector('[data-testid="showcase-earlier-toggle"]')?.getAttribute('aria-expanded'),
    ).toBe('false');
    expect(root.querySelector<HTMLElement>('[data-testid="showcase-panel-panel"]')?.inert).toBe(
      true,
    );
  });
});
