import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { ShowcaseComponent } from './showcase.component';

describe('ShowcaseComponent', () => {
  it('lists every section and shows them all', async () => {
    const fixture = TestBed.createComponent(ShowcaseComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const links = Array.from(
      root.querySelectorAll<HTMLAnchorElement>('[data-testid="showcase-contents-link"]'),
    ).map((link) => link.getAttribute('href'));
    expect(links).toEqual(['#actions', '#forms', '#layout', '#overlays', '#ai']);
    for (const href of links) {
      expect(root.querySelector(`section${href ?? ''}`)).not.toBeNull();
    }
    await expectAccessible(root);
  });
});
