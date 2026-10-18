import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { expectAccessible } from '@glave/glaze/testing';

import { SessionService } from '../../services/session.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { PersonComponent } from './person.component';

describe('PersonComponent', () => {
  it('shows who is signed in, and opens to Settings and Sign out', async () => {
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: '**', children: [] }]),
        ...provideFakeSession(new FakeSession()),
      ],
    });
    const fixture = TestBed.createComponent(PersonComponent);
    let navigated = 0;
    fixture.componentInstance.navigated.subscribe(() => (navigated += 1));
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const person = root.querySelector<HTMLButtonElement>('[data-testid="person"]');
    const menu = root.querySelector<HTMLElement>('[data-testid="person-menu"]');
    expect(person?.querySelector('.text')?.textContent.trim()).toBe('You');
    expect(person?.popoverTargetElement).toBe(menu);
    expect(person?.getAttribute('aria-expanded')).toBe('false');
    expect(root.querySelector('[data-testid="nav-settings"]')?.getAttribute('href')).toBe(
      '/settings',
    );
    await expectAccessible(root);
    root.querySelector<HTMLElement>('[data-testid="nav-settings"]')?.click();
    await fixture.whenStable();
    expect(navigated).toBe(1);
    const session = TestBed.inject(SessionService);
    expect(session.signedIn()).toBe(true);
    root.querySelector<HTMLButtonElement>('[data-testid="person-sign-out"]')?.click();
    await new Promise((resolve) => setTimeout(resolve));
    expect(session.signedIn()).toBe(false);
  });
});
