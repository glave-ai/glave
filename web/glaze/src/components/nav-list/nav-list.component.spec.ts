import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzNavItemComponent } from '../nav-item/nav-item.component';
import { GlzNavListComponent } from './nav-list.component';

@Component({
  imports: [GlzNavListComponent, GlzNavItemComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <nav aria-label="Threads">
      <glz-nav-list heading="Today" data-testid="today">
        <a glzNavItem href="/t/1" current>Plan the launch</a>
      </glz-nav-list>
      <glz-nav-list data-testid="plain"><a glzNavItem href="/t/2">Other</a></glz-nav-list>
    </nav>
  `,
})
class HostComponent {}

describe('GlzNavListComponent', () => {
  it('names its group by its heading, when it has one', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const today = root.querySelector('[data-testid="today"]');
    const heading = today?.querySelector('.heading');
    expect(heading?.textContent).toBe('Today');
    expect(today?.getAttribute('aria-labelledby')).toBe(heading?.id);
    expect(root.querySelector('[data-testid="plain"]')?.hasAttribute('aria-labelledby')).toBe(
      false,
    );
    await expectAccessible(root);
  });
});
