import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { SessionService } from '../../services/session.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { PersonComponent } from './person.component';

describe('PersonComponent', () => {
  it('shows who is signed in, the theme choice, and signs out', async () => {
    TestBed.configureTestingModule({ providers: provideFakeSession(new FakeSession()) });
    const fixture = TestBed.createComponent(PersonComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('.name')?.textContent).toBe('You');
    expect(root.querySelector('[data-testid="theme-picker"]')).not.toBeNull();
    await expectAccessible(root);
    const session = TestBed.inject(SessionService);
    expect(session.signedIn()).toBe(true);
    root.querySelector<HTMLButtonElement>('[data-testid="person-sign-out"]')?.click();
    expect(session.signedIn()).toBe(false);
  });
});
