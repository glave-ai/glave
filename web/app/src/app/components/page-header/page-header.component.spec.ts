import { BreakpointObserver, type BreakpointState } from '@angular/cdk/layout';
import { Location } from '@angular/common';
import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';
import { BehaviorSubject } from 'rxjs';

import { NavService } from '../../services/nav.service';
import { NAVIGATION, PageHeaderComponent } from './page-header.component';

@Component({
  imports: [PageHeaderComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<app-page-header [back]="back()" />`,
})
class HostComponent {
  readonly back = signal(false);
}

describe('PageHeaderComponent', () => {
  let narrow: BehaviorSubject<BreakpointState>;

  function setUp(navigation: Pick<Navigation, 'canGoBack'> | null): void {
    localStorage.clear();
    narrow = new BehaviorSubject<BreakpointState>({ matches: false, breakpoints: {} });
    TestBed.configureTestingModule({
      providers: [
        provideRouter([]),
        { provide: BreakpointObserver, useValue: { observe: () => narrow } },
        { provide: NAVIGATION, useValue: navigation },
      ],
    });
  }

  async function render(back: boolean): Promise<HTMLElement> {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.back.set(back);
    await fixture.whenStable();
    return fixture.nativeElement as HTMLElement;
  }

  it('is empty on wide screens on the Chats: no menu button and no Back', async () => {
    setUp(null);
    const root = await render(false);
    expect(root.querySelector('header glz-top-bar')).not.toBeNull();
    expect(root.querySelector('[data-testid="page-menu"]')).toBeNull();
    expect(root.querySelector('[data-testid="page-back"]')).toBeNull();
    await expectAccessible(root);
  });

  it('opens the side nav’s drawer on narrow screens', async () => {
    setUp(null);
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

  it('has Back after the menu button on a page that is not the Chats', async () => {
    setUp(null);
    narrow.next({ matches: true, breakpoints: {} });
    const root = await render(true);
    const buttons = [...root.querySelectorAll('header button')];
    expect(buttons.map((button) => button.getAttribute('data-testid'))).toEqual([
      'page-menu',
      'page-back',
    ]);
    expect(buttons[1]?.textContent.trim()).toBe('Back');
    await expectAccessible(root);
  });

  it('goes back to the page before when it is glave’s', async () => {
    setUp({ canGoBack: true });
    const back = vi.spyOn(TestBed.inject(Location), 'back').mockReturnValue(undefined);
    const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
    const root = await render(true);
    root.querySelector<HTMLButtonElement>('[data-testid="page-back"]')?.click();
    expect(back).toHaveBeenCalledOnce();
    expect(navigate).not.toHaveBeenCalled();
  });

  it('goes to the Chats when the person arrived at the page directly', async () => {
    setUp({ canGoBack: false });
    const back = vi.spyOn(TestBed.inject(Location), 'back').mockReturnValue(undefined);
    const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
    const root = await render(true);
    root.querySelector<HTMLButtonElement>('[data-testid="page-back"]')?.click();
    expect(navigate).toHaveBeenCalledWith('/');
    expect(back).not.toHaveBeenCalled();
  });

  it('goes to the Chats in a browser that keeps no such history', async () => {
    setUp(null);
    const navigate = vi.spyOn(TestBed.inject(Router), 'navigateByUrl').mockResolvedValue(true);
    const root = await render(true);
    root.querySelector<HTMLButtonElement>('[data-testid="page-back"]')?.click();
    expect(navigate).toHaveBeenCalledWith('/');
  });
});
