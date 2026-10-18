import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type Space } from '@glave/client';

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
    expect(spaces.home()).toBe('me');
    expect(spaces.thread('t1')?.title).toBe('Plan');
    expect(spaces.thread('nope')).toBeUndefined();
    expect(spaces.spaceOf('t1')?.name).toBe('glave');
    expect(spaces.spaceOf('nope')).toBeUndefined();
    expect(spaces.loaded()).toBe(true);
  });

  it('starts new chats in the personal space (Chats), not the first project', async () => {
    const fake = new FakeSession()
      .answer('space/list', {
        spaces: [
          space('p1', 'Playground'),
          space('p2', 'glave'),
          space('me', 'Personal', 'personal'),
        ],
      })
      .answer('thread/list', { threads: [] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const spaces = TestBed.inject(SpacesService);
    await spaces.refresh();
    expect(spaces.spaces()[0]?.space.name).toBe('Playground');
    expect(spaces.home()).toBe('me');
  });

  it('has no home of its own with only projects, so the server picks', async () => {
    const fake = new FakeSession()
      .answer('space/list', { spaces: [space('p1', 'Playground')] })
      .answer('thread/list', { threads: [] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const spaces = TestBed.inject(SpacesService);
    await spaces.refresh();
    expect(spaces.home()).toBeUndefined();
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

  it('makes a project, then lists it, after a read that began before it was made', async () => {
    const listed: Space[] = [space('me', 'Personal', 'personal'), space('p1', 'glave')];
    const fake = new FakeSession()
      .answer('space/list', () => ({ spaces: [...listed] }))
      .answer('thread/list', { threads: [] })
      .answer('space/create', (params) => {
        const made = space('p2', `${(params as { name: string }).name} 2`);
        listed.push(made);
        return { space: made };
      });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const spaces = TestBed.inject(SpacesService);
    const before = spaces.refresh();
    const made = await spaces.create({ name: 'glave', folder: { kind: 'empty' } });
    await before;
    expect(made.name).toBe('glave 2');
    expect(fake.paramsOf('space/create')).toEqual([{ name: 'glave', folder: { kind: 'empty' } }]);
    expect(fake.paramsOf('space/list')).toHaveLength(2);
    expect(spaces.spaces().map((entry) => entry.space.name)).toEqual([
      'glave',
      'glave 2',
      'Personal',
    ]);
  });

  it('throws what the server refused, and lists nothing new', async () => {
    const fake = new FakeSession()
      .answer('space/list', { spaces: [] })
      .answer(
        'space/create',
        new ApiError({ code: ErrorCode.HOST, message: 'could not clone it' }),
      );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const spaces = TestBed.inject(SpacesService);
    await expect(
      spaces.create({ name: 'x', folder: { kind: 'repository', url: 'https://x/y' } }),
    ).rejects.toThrow('could not clone it');
    expect(fake.paramsOf('space/list')).toHaveLength(0);
  });

  it('renames a project, then lists it by its new name', async () => {
    let name = 'glave';
    const fake = new FakeSession()
      .answer('space/list', () => ({ spaces: [space('p1', name)] }))
      .answer('thread/list', { threads: [] })
      .answer('space/rename', (params) => {
        name = (params as { name: string }).name;
        return { space: space('p1', name) };
      });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const spaces = TestBed.inject(SpacesService);
    await spaces.refresh();
    await spaces.rename('p1', 'glave app');
    expect(fake.paramsOf('space/rename')).toEqual([{ space: 'p1', name: 'glave app' }]);
    expect(spaces.spaces().map((entry) => entry.space.name)).toEqual(['glave app']);
  });

  it('throws a rename the server refused', async () => {
    const fake = new FakeSession()
      .answer('space/list', { spaces: [] })
      .answer(
        'space/rename',
        new ApiError({ code: ErrorCode.NOT_ALLOWED, message: 'only its owner renames it' }),
      );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const spaces = TestBed.inject(SpacesService);
    await expect(spaces.rename('p1', 'x')).rejects.toThrow('only its owner renames it');
    expect(fake.paramsOf('space/list')).toHaveLength(0);
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
