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
    // Closed, Earlier says something in it waits.
    expect(root.querySelector('[data-testid="showcase-earlier-end"] [role="img"]')?.ariaLabel).toBe(
      'needs you',
    );
    expect(root.querySelector<HTMLElement>('[data-testid="showcase-panel-panel"]')?.inert).toBe(
      true,
    );
  });

  it('opens a list’s options from its ⋯, and picks one', async () => {
    const fixture = TestBed.createComponent(ShowcaseLayoutComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    // A menu that opens before its trigger's first tick closes again (the CDK's focus check).
    await new Promise((resolve) => setTimeout(resolve));
    root.querySelector<HTMLButtonElement>('[data-testid="showcase-earlier-more-trigger"]')?.click();
    await fixture.whenStable();
    const actions = Array.from(
      document.querySelectorAll<HTMLElement>('[data-testid="showcase-earlier-more-action"]'),
    );
    expect(actions.map((action) => action.textContent.trim())).toEqual(['Rename', 'Archive']);
    actions[1]?.click();
    await fixture.whenStable();
    expect(document.querySelector('[role="menu"]')).toBeNull();
  });
});
