import { TestBed } from '@angular/core/testing';
import {
  type ActivatedRouteSnapshot,
  Router,
  type RouterStateSnapshot,
  UrlTree,
} from '@angular/router';

import { SessionService } from '../services/session.service';
import { FAKE_SERVER, FakeSession, provideFakeSession } from '../testing/fake-session';
import { inviteGuard } from './invite.guard';

describe('inviteGuard', () => {
  async function run(link: string): Promise<unknown> {
    window.history.replaceState(null, '', link);
    const fake = new FakeSession().answer('server/discover', { id: FAKE_SERVER });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake, null) });
    await TestBed.inject(SessionService).start();
    return TestBed.runInInjectionContext(() =>
      inviteGuard({} as ActivatedRouteSnapshot, {} as RouterStateSnapshot),
    );
  }

  it('sends a tab opened with an invite to the page that opens it', async () => {
    const result = await run('/#invite=c0de');
    expect(result).toBeInstanceOf(UrlTree);
    expect(TestBed.inject(Router).serializeUrl(result as UrlTree)).toBe('/invite');
  });

  it('lets any other tab through', async () => {
    expect(await run('/')).toBe(true);
  });
});
