import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { space, thread } from '../testing/fixtures';
import { REFRESH_MS, SpacesService } from './spaces.service';

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

  it('joins a read under way instead of starting another', async () => {
    const fake = new FakeSession().answer('space/list', { spaces: [] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const spaces = TestBed.inject(SpacesService);
    await Promise.all([spaces.refresh(), spaces.refresh()]);
    expect(fake.paramsOf('space/list')).toHaveLength(1);
    await spaces.refresh();
    expect(fake.paramsOf('space/list')).toHaveLength(2);
  });

  it('polls only while the tab is shown', () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    const hidden = vi.spyOn(document, 'hidden', 'get').mockReturnValue(true);
    try {
      const fake = new FakeSession().answer('space/list', { spaces: [] });
      TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
      TestBed.inject(SpacesService);
      vi.advanceTimersByTime(REFRESH_MS * 3);
      expect(fake.paramsOf('space/list')).toHaveLength(0);
      hidden.mockReturnValue(false);
      vi.advanceTimersByTime(REFRESH_MS);
      expect(fake.paramsOf('space/list')).toHaveLength(1);
    } finally {
      hidden.mockRestore();
      vi.useRealTimers();
    }
  });
});
