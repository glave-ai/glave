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
