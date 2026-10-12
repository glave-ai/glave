import { BreakpointObserver, type BreakpointState } from '@angular/cdk/layout';
import { ChangeDetectionStrategy, Component, viewChild } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { BehaviorSubject, type Observable } from 'rxjs';

import { expectAccessible } from '../../testing/axe';
import { GlzSidenavLayoutComponent } from './sidenav-layout.component';

@Component({
  imports: [GlzSidenavLayoutComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-sidenav-layout testId="sidenav" label="Spaces">
      <nav glzSidenav aria-label="Spaces"><a href="/a">A space</a></nav>
      <main>Content</main>
    </glz-sidenav-layout>
  `,
})
class HostComponent {
  readonly layout = viewChild.required(GlzSidenavLayoutComponent);
}

class FakeBreakpoints {
  readonly matches = new BehaviorSubject<BreakpointState>({ matches: false, breakpoints: {} });
  observe(): Observable<BreakpointState> {
    return this.matches;
  }
}

describe('GlzSidenavLayoutComponent', () => {
  let breakpoints: FakeBreakpoints;

  beforeEach(() => {
    breakpoints = new FakeBreakpoints();
    TestBed.configureTestingModule({
      providers: [{ provide: BreakpointObserver, useValue: breakpoints }],
    });
  });

  function sidenav(root: HTMLElement): HTMLElement {
    const found = root.querySelector<HTMLElement>('[data-testid="sidenav-nav"]');
    if (found === null) {
      throw new Error('no side nav');
    }
    return found;
  }

  it('shows the nav beside the content on wide screens, and collapses it', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const layout = fixture.componentInstance.layout();
    expect(layout.open()).toBe(true);
    expect(sidenav(root).inert).toBe(false);
    expect(sidenav(root).getAttribute('role')).toBeNull();
    await expectAccessible(root);
    layout.toggle();
    await fixture.whenStable();
    expect(sidenav(root).inert).toBe(true);
  });

  it('resizes and collapses by its edge, a window splitter', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const layout = fixture.componentInstance.layout();
    const handle = root.querySelector<HTMLElement>('[data-testid="sidenav-handle"]');
    if (handle === null) {
      throw new Error('no handle');
    }
    const press = (key: string): void => {
      handle.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }));
    };
    expect(handle.getAttribute('role')).toBe('separator');
    expect(handle.getAttribute('aria-controls')).toBe(sidenav(root).id);
    expect(handle.getAttribute('aria-label')).toBe('Resize the navigation');
    press('Home');
    expect(layout.width()).toBe(200);
    press('ArrowRight');
    expect(layout.width()).toBe(216);
    press('ArrowLeft');
    press('ArrowLeft');
    expect(layout.width()).toBe(200);
    await fixture.whenStable();
    expect(root.querySelector('glz-sidenav-layout')?.getAttribute('style')).toContain(
      '--glz-sidenav-width: 200px',
    );
    expect(handle.getAttribute('aria-valuenow')).toBe('200');
    press('Enter');
    await fixture.whenStable();
    expect(layout.open()).toBe(false);
    expect(sidenav(root).inert).toBe(true);
    expect(handle.getAttribute('aria-valuenow')).toBe('0');
    await expectAccessible(root);
    press('Enter');
    expect(layout.open()).toBe(true);
  });

  it('keeps the wide state while the screen is narrow, and brings it back', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const layout = fixture.componentInstance.layout();
    layout.close();
    breakpoints.matches.next({ matches: true, breakpoints: {} });
    await fixture.whenStable();
    layout.toggle();
    await fixture.whenStable();
    expect(layout.open()).toBe(true);
    breakpoints.matches.next({ matches: false, breakpoints: {} });
    await fixture.whenStable();
    expect(layout.open()).toBe(false);
    expect(
      (fixture.nativeElement as HTMLElement).querySelector('[data-testid="sidenav-handle"]'),
    ).not.toBeNull();
  });

  it('becomes a modal drawer on narrow screens, closed by Escape or the scrim', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    breakpoints.matches.next({ matches: true, breakpoints: {} });
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const layout = fixture.componentInstance.layout();
    expect(layout.open()).toBe(false);
    layout.toggle();
    await fixture.whenStable();
    expect(sidenav(root).getAttribute('role')).toBe('dialog');
    expect(sidenav(root).getAttribute('aria-modal')).toBe('true');
    expect(document.activeElement).toBe(sidenav(root));
    expect(root.querySelector<HTMLElement>('.content')?.inert).toBe(true);
    sidenav(root).dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await fixture.whenStable();
    expect(layout.open()).toBe(false);
    layout.toggle();
    await fixture.whenStable();
    root.querySelector<HTMLElement>('[data-testid="sidenav-scrim"]')?.click();
    await fixture.whenStable();
    expect(layout.open()).toBe(false);
    layout.toggle();
    layout.navigated();
    expect(layout.open()).toBe(false);
  });
});
