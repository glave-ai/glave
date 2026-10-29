import { TestBed } from '@angular/core/testing';
import type { ActivatedRouteSnapshot, CanActivateFn, RouterStateSnapshot } from '@angular/router';

import type { Authenticator } from '../lib/passkeys';
import { SessionService } from '../services/session.service';
import { FAKE_SERVER, FakeSession, provideFakeSession } from './fake-session';
import { MADE_PASSKEY } from './fixtures';

/** A device that makes a passkey whenever it is asked, and signs in with none. */
export const MAKING_DEVICE: Authenticator = {
  create: () => Promise.resolve(MADE_PASSKEY as unknown as Credential),
  get: () => Promise.resolve(null),
};

/** Lets a component's promises and the page settle. */
export async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve));
  }
  await fixture.whenStable();
}

/** What `guard` answers for a tab opened at `link`, signed out, the app started. */
export async function guardAt(guard: CanActivateFn, link: string): Promise<unknown> {
  window.history.replaceState(null, '', link);
  const fake = new FakeSession().answer('server/discover', { id: FAKE_SERVER });
  TestBed.configureTestingModule({ providers: provideFakeSession(fake, null) });
  await TestBed.inject(SessionService).start();
  return TestBed.runInInjectionContext(() =>
    guard({} as ActivatedRouteSnapshot, {} as RouterStateSnapshot),
  );
}
