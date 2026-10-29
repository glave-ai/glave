import { TestBed } from '@angular/core/testing';
import {
  type ActivatedRouteSnapshot,
  Router,
  type RouterStateSnapshot,
  UrlTree,
} from '@angular/router';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { SERVER } from '../testing/fixtures';
import { adminGuard } from './admin.guard';

describe('adminGuard', () => {
  async function run(said: object): Promise<unknown> {
    const fake = new FakeSession().answer('server/discover', { ...SERVER, ...said });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    return TestBed.runInInjectionContext(() =>
      adminGuard({} as ActivatedRouteSnapshot, {} as RouterStateSnapshot),
    );
  }

  function sentTo(result: unknown): string {
    expect(result).toBeInstanceOf(UrlTree);
    return TestBed.inject(Router).serializeUrl(result as UrlTree);
  }

  it('lets an organization’s admins and owners in', async () => {
    const organization = { exists: { organization: true, workers: false } };
    expect(await run({ ...organization, role: 'admin' })).toBe(true);
    TestBed.resetTestingModule();
    expect(await run({ ...organization, role: 'owner' })).toBe(true);
  });

  it('sends a member to their Chats', async () => {
    const said = { exists: { organization: true, workers: false }, role: 'member' };
    expect(sentTo(await run(said))).toBe('/');
  });

  it('sends a person on their own server, with no organization shown, to their Chats', async () => {
    expect(sentTo(await run({}))).toBe('/');
  });
});
