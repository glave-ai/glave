import { TestBed } from '@angular/core/testing';
import {
  type ActivatedRouteSnapshot,
  Router,
  type RouterStateSnapshot,
  UrlTree,
} from '@angular/router';
import { ApiError, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { MODELS } from '../testing/fixtures';
import { hasModelGuard } from './has-model.guard';

describe('hasModelGuard', () => {
  async function run(fake: FakeSession): Promise<unknown> {
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    return TestBed.runInInjectionContext(() =>
      hasModelGuard({} as ActivatedRouteSnapshot, {} as RouterStateSnapshot),
    );
  }

  it('lets the person in once a model is set up', async () => {
    expect(await run(new FakeSession().answer('model/list', { models: MODELS }))).toBe(true);
  });

  it('sends a first run to the page that adds a model', async () => {
    const result = await run(new FakeSession().answer('model/list', { models: [] }));
    expect(result).toBeInstanceOf(UrlTree);
    expect(TestBed.inject(Router).serializeUrl(result as UrlTree)).toBe('/connect');
  });

  it('lets the person in when the list cannot be read, rather than send them away', async () => {
    const down = new ApiError({ code: ErrorCode.INTERNAL, message: 'down' });
    expect(await run(new FakeSession().answer('model/list', down))).toBe(true);
  });
});
