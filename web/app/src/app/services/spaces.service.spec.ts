import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { space, thread } from '../testing/fixtures';
import { SpacesService } from './spaces.service';

describe('SpacesService', () => {
  it('lists projects before the personal space, each with its threads', async () => {
    const fake = new FakeSession()
      .answer('space/list', {
        spaces: [space('me', 'Personal', 'personal'), space('p1', 'glave')],
      })
      .answer('thread/list', (params) => ({
        threads: (params as { space: string }).space === 'p1' ? [thread('t1', 'Plan')] : [],
      }));
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const spaces = TestBed.inject(SpacesService);
    await spaces.refresh();
    expect(spaces.spaces().map((entry) => entry.space.name)).toEqual(['glave', 'Personal']);
    expect(spaces.home()).toBe('p1');
    expect(spaces.thread('t1')?.title).toBe('Plan');
    expect(spaces.thread('nope')).toBeUndefined();
    expect(spaces.loaded()).toBe(true);
  });

  it('says what went wrong', async () => {
    const fake = new FakeSession().answer(
      'space/list',
      new ApiError({ code: ErrorCode.INTERNAL, message: 'store down' }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const spaces = TestBed.inject(SpacesService);
    await spaces.refresh();
    expect(spaces.error()).toBe('store down');
  });
});
