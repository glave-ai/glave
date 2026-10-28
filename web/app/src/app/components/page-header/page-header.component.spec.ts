import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';
import { type FakeMedia, fakeMedia } from '@glave/glaze/testing/media';

import { BackService } from '../../services/back.service';
import { NavService } from '../../services/nav.service';
import { PageHeaderComponent } from './page-header.component';

@Component({
  imports: [PageHeaderComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<app-page-header [back]="back()" />`,
})
class HostComponent {
  readonly back = signal(false);
}

describe('PageHeaderComponent', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  let narrow: FakeMedia;

  beforeEach(() => {
    localStorage.clear();
    narrow = fakeMedia();
    TestBed.configureTestingModule({
      providers: [provideRouter([])],
    });
  });

  async function render(back: boolean): Promise<HTMLElement> {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.back.set(back);
    await fixture.whenStable();
    return fixture.nativeElement as HTMLElement;
  }

  it('is empty on wide screens on the Chats: no menu button and no Back', async () => {
    const root = await render(false);
    expect(root.querySelector('header glz-top-bar')).not.toBeNull();
    expect(root.querySelector('[data-testid="page-menu"]')).toBeNull();
    expect(root.querySelector('[data-testid="page-back"]')).toBeNull();
    await expectAccessible(root);
  });

  it('opens the side nav’s drawer on narrow screens', async () => {
    narrow.set(true);
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
    narrow.set(true);
    const root = await render(true);
    const buttons = [...root.querySelectorAll('header button')];
    expect(buttons.map((button) => button.getAttribute('data-testid'))).toEqual([
      'page-menu',
      'page-back',
    ]);
    expect(buttons[1]?.textContent.trim()).toBe('Back');
    await expectAccessible(root);
  });

  it('goes back where the person came from, else to their Chats', async () => {
    const back = vi.spyOn(TestBed.inject(BackService), 'back').mockResolvedValue(undefined);
    const root = await render(true);
    root.querySelector<HTMLButtonElement>('[data-testid="page-back"]')?.click();
    expect(back).toHaveBeenCalledWith('/');
  });
});
