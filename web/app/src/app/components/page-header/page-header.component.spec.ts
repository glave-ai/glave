import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { NavService } from '../../services/nav.service';
import { PageHeaderComponent } from './page-header.component';

@Component({
  imports: [PageHeaderComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <app-page-header>
      <h1>Tasks</h1>
      <button appPageHeaderEnd type="button" data-testid="action">Refresh</button>
    </app-page-header>
  `,
})
class HostComponent {}

describe('PageHeaderComponent', () => {
  it('holds the page’s heading and actions, and toggles the side nav', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('header glz-top-bar h1')?.textContent).toBe('Tasks');
    expect(root.querySelector('.end [data-testid="action"]')).not.toBeNull();
    const menu = root.querySelector<HTMLButtonElement>('[data-testid="page-menu"]');
    expect(menu?.getAttribute('aria-expanded')).toBe('true');
    await expectAccessible(root);
    menu?.click();
    await fixture.whenStable();
    expect(TestBed.inject(NavService).open()).toBe(false);
    expect(menu?.getAttribute('aria-expanded')).toBe('false');
  });
});
