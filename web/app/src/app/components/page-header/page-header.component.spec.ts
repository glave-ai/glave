import { BreakpointObserver, type BreakpointState } from '@angular/cdk/layout';
import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';
import { BehaviorSubject } from 'rxjs';

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
  let narrow: BehaviorSubject<BreakpointState>;

  beforeEach(() => {
    localStorage.clear();
    narrow = new BehaviorSubject<BreakpointState>({ matches: false, breakpoints: {} });
    TestBed.configureTestingModule({
      providers: [{ provide: BreakpointObserver, useValue: { observe: () => narrow } }],
    });
  });

  it('holds the page’s heading and actions, with no menu button on wide screens', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('header glz-top-bar h1')?.textContent).toBe('Tasks');
    expect(root.querySelector('.end [data-testid="action"]')).not.toBeNull();
    expect(root.querySelector('[data-testid="page-menu"]')).toBeNull();
    await expectAccessible(root);
  });

  it('opens the side nav’s drawer on narrow screens', async () => {
    narrow.next({ matches: true, breakpoints: {} });
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const menu = root.querySelector<HTMLButtonElement>('[data-testid="page-menu"]');
    expect(menu?.getAttribute('aria-expanded')).toBe('true');
    expect(menu?.getAttribute('aria-keyshortcuts')).toMatch(/\+B$/u);
    await expectAccessible(root);
    menu?.click();
    await fixture.whenStable();
    expect(TestBed.inject(NavService).open()).toBe(false);
    expect(menu?.getAttribute('aria-expanded')).toBe('false');
  });
});
