import { TestBed } from '@angular/core/testing';
import {
  ApiError,
  ErrorCode,
  type Notification,
  type Project,
  type ThreadSummary,
} from '@glave/client';

import { FakeSession, FakeTurn, provideFakeSession } from '../testing/fake-session';
import { record, thread } from '../testing/fixtures';
import { CLOCK_MS } from './lists-feed';
import { ProjectsService } from './projects.service';
import { SessionService } from './session.service';

/** What the server sends when `summary`'s thread changes. */
function changed(summary: ThreadSummary): Notification {
  return { jsonrpc: '2.0', method: 'thread/changed', params: { thread: summary.id, summary } };
}

/** Lets what is under way run. */
function settled(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

/** Follows the lists, once what follows them has loaded and done what it could. */
async function following(projects: ProjectsService): Promise<void> {
  projects.follow();
  await vi.dynamicImportSettled();
  await settled();
}

describe('ProjectsService', () => {
  it('lists projects before the personal project, each with its threads', async () => {
    const fake = new FakeSession()
      .answer('project/list', {
        projects: [record('me', 'Personal', 'personal'), record('p1', 'glave')],
      })
      .answer('thread/list', (params) => ({
        threads: (params as { project: string }).project === 'p1' ? [thread('t1', 'Plan')] : [],
      }));
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    await projects.refresh();
    expect(projects.projects().map((entry) => entry.project.name)).toEqual(['glave', 'Personal']);
    expect(projects.home()).toBe('me');
    expect(projects.thread('t1')?.title).toBe('Plan');
    expect(projects.thread('nope')).toBeUndefined();
    expect(projects.project('p1')?.name).toBe('glave');
    expect(projects.project('nope')).toBeUndefined();
    expect(projects.loaded()).toBe(true);
  });

  it('starts new chats in the personal project (Chats), not the first project', async () => {
    const fake = new FakeSession()
      .answer('project/list', {
        projects: [
          record('p1', 'Playground'),
          record('p2', 'glave'),
          record('me', 'Personal', 'personal'),
        ],
      })
      .answer('thread/list', { threads: [] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    await projects.refresh();
    expect(projects.projects()[0]?.project.name).toBe('Playground');
    expect(projects.home()).toBe('me');
  });

  it('has no home of its own with only projects, so the server picks', async () => {
    const fake = new FakeSession()
      .answer('project/list', { projects: [record('p1', 'Playground')] })
      .answer('thread/list', { threads: [] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    await projects.refresh();
    expect(projects.home()).toBeUndefined();
  });

  it('counts the lists’ times from when it last read them', async () => {
    vi.useFakeTimers({ toFake: ['Date'] });
    try {
      vi.setSystemTime(Date.parse('2026-10-11T10:00:00.000Z'));
      const fake = new FakeSession().answer('project/list', { projects: [] });
      TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
      const projects = TestBed.inject(ProjectsService);
      vi.setSystemTime(Date.parse('2026-10-11T10:05:00.000Z'));
      expect(projects.readAt()).toBe(Date.parse('2026-10-11T10:00:00.000Z'));
      await projects.refresh();
      expect(projects.readAt()).toBe(Date.parse('2026-10-11T10:05:00.000Z'));
    } finally {
      vi.useRealTimers();
    }
  });

  it('says what went wrong', async () => {
    const fake = new FakeSession().answer(
      'project/list',
      new ApiError({ code: ErrorCode.INTERNAL, message: 'store down' }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    await projects.refresh();
    expect(projects.error()).toBe('store down');
  });

  it('joins a read under way instead of starting another', async () => {
    const fake = new FakeSession().answer('project/list', { projects: [] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    await Promise.all([projects.refresh(), projects.refresh()]);
    expect(fake.paramsOf('project/list')).toHaveLength(1);
    await projects.refresh();
    expect(fake.paramsOf('project/list')).toHaveLength(2);
  });

  it('makes a project, then lists it, after a read that began before it was made', async () => {
    const listed: Project[] = [record('me', 'Personal', 'personal'), record('p1', 'glave')];
    const fake = new FakeSession()
      .answer('project/list', () => ({ projects: [...listed] }))
      .answer('thread/list', { threads: [] })
      .answer('project/create', (params) => {
        const made = record('p2', `${(params as { name: string }).name} 2`);
        listed.push(made);
        return { project: made };
      });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    const before = projects.refresh();
    const made = await projects.create({ name: 'glave', folder: { kind: 'empty' } });
    await before;
    expect(made.name).toBe('glave 2');
    expect(fake.paramsOf('project/create')).toEqual([{ name: 'glave', folder: { kind: 'empty' } }]);
    expect(fake.paramsOf('project/list')).toHaveLength(2);
    expect(projects.projects().map((entry) => entry.project.name)).toEqual([
      'glave',
      'glave 2',
      'Personal',
    ]);
  });

  it('throws what the server refused, and lists nothing new', async () => {
    const fake = new FakeSession()
      .answer('project/list', { projects: [] })
      .answer(
        'project/create',
        new ApiError({ code: ErrorCode.HOST, message: 'could not clone it' }),
      );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    await expect(
      projects.create({ name: 'x', folder: { kind: 'repository', url: 'https://x/y' } }),
    ).rejects.toThrow('could not clone it');
    expect(fake.paramsOf('project/list')).toHaveLength(0);
  });

  it('renames a project, then lists it by its new name', async () => {
    let name = 'glave';
    const fake = new FakeSession()
      .answer('project/list', () => ({ projects: [record('p1', name)] }))
      .answer('thread/list', { threads: [] })
      .answer('project/rename', (params) => {
        name = (params as { name: string }).name;
        return { project: record('p1', name) };
      });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    await projects.refresh();
    await projects.rename('p1', 'glave app');
    expect(fake.paramsOf('project/rename')).toEqual([{ project: 'p1', name: 'glave app' }]);
    expect(projects.projects().map((entry) => entry.project.name)).toEqual(['glave app']);
  });

  it('throws a rename the server refused', async () => {
    const fake = new FakeSession().answer('project/list', { projects: [] }).answer(
      'project/rename',
      new ApiError({
        code: ErrorCode.INVALID_PARAMS,
        message: 'there is a project called "glave" already; choose another name',
      }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    await expect(projects.rename('p1', 'x')).rejects.toThrow('choose another name');
    expect(fake.paramsOf('project/list')).toHaveLength(0);
  });

  it('knows which projects lost their folder, in the server’s words, until one is found', async () => {
    const said = "This project's folder isn't there anymore.";
    let gone = true;
    const fake = new FakeSession()
      .answer('project/list', () => ({
        projects: [record('p1', 'playground'), record('p2', 'glave')],
        gone: gone ? [{ project: 'p1', message: said }] : [],
      }))
      .answer('thread/list', { threads: [] })
      .answer('project/folder/set', () => {
        gone = false;
        return { project: record('p1', 'playground') };
      });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    await projects.refresh();
    expect(projects.gone('p1')).toBe(said);
    expect(projects.gone('p2')).toBeNull();
    await projects.findFolder('p1', '/Users/ana/code/playground');
    expect(fake.paramsOf('project/folder/set')).toEqual([
      { project: 'p1', path: '/Users/ana/code/playground' },
    ]);
    expect(projects.gone('p1')).toBeNull();
  });

  it('removes a project, then lists it no more', async () => {
    let listed = [record('p1', 'playground'), record('me', 'Personal', 'personal')];
    const fake = new FakeSession()
      .answer('project/list', () => ({ projects: listed }))
      .answer('thread/list', { threads: [] })
      .answer('project/remove', () => {
        listed = listed.filter((each) => each.id !== 'p1');
        return {};
      });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    await projects.refresh();
    await projects.remove('p1');
    expect(fake.paramsOf('project/remove')).toEqual([{ project: 'p1' }]);
    expect(projects.projects().map((entry) => entry.project.name)).toEqual(['Personal']);
  });

  it('follows the lists from the newest update they hold, as the server says each thread changes', async () => {
    const watch = new FakeTurn();
    const fake = new FakeSession()
      .answer('project/list', { projects: [record('project-1', 'glave')] })
      .answer('thread/list', { threads: [thread('t1', 'Plan')] })
      .listWatch(watch);
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    await following(projects);
    expect(fake.paramsOf('thread/list/watch')).toEqual([{ after: '2026-10-11T10:05:00.000Z' }]);
    const at = '2026-10-11T10:06:00.000Z';
    watch.send(changed(thread('t1', 'Plan', { status: 'running', updated_at: at })));
    await settled();
    expect(projects.thread('t1')?.status).toBe('running');
    watch.send(changed(thread('t2', 'Fix the build', { updated_at: at })));
    watch.send(changed(thread('t1', 'Plan', { status: 'waiting', updated_at: at })));
    await settled();
    expect(projects.thread('t1')?.status).toBe('waiting');
    expect(projects.projects()[0]?.threads.map((each) => each.id)).toEqual(['t1', 't2']);
    watch.send(changed(thread('t2', 'Fix the build', { standing: 'deleted', updated_at: at })));
    await settled();
    expect(projects.thread('t2')).toBeUndefined();
    // Nothing was read again: the watch said it all.
    expect(fake.paramsOf('project/list')).toHaveLength(1);
    expect(fake.paramsOf('thread/list')).toHaveLength(1);
  });

  it('reads the lists again when a thread turns up in a project they lack', async () => {
    const watch = new FakeTurn();
    let listed = [record('project-1', 'glave')];
    const fake = new FakeSession()
      .answer('project/list', () => ({ projects: listed }))
      .answer('thread/list', { threads: [] })
      .listWatch(watch);
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    await following(projects);
    listed = [...listed, record('project-2', 'site')];
    watch.send(changed(thread('t9', 'Launch', { project: 'project-2' })));
    await settled();
    expect(fake.paramsOf('project/list')).toHaveLength(2);
    // Heard while the lists were read again, so taken in once they were.
    expect(projects.thread('t9')?.title).toBe('Launch');
  });

  it('reads the lists again and comes back from the newest update once its watch is cut off', async () => {
    vi.useFakeTimers();
    try {
      const [cut, again] = [new FakeTurn(), new FakeTurn()];
      const fake = new FakeSession()
        .answer('project/list', { projects: [record('project-1', 'glave')] })
        .answer('thread/list', { threads: [thread('t1', 'Plan')] })
        .listWatch(cut)
        .listWatch(again);
      TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
      const projects = TestBed.inject(ProjectsService);
      projects.follow();
      await vi.dynamicImportSettled();
      await vi.advanceTimersByTimeAsync(0);
      cut.end({ ended: 'unwatched' });
      await vi.advanceTimersByTimeAsync(999);
      expect(fake.paramsOf('project/list')).toHaveLength(1);
      await vi.advanceTimersByTimeAsync(1);
      expect(fake.paramsOf('project/list')).toHaveLength(2);
      expect(fake.paramsOf('thread/list/watch')).toHaveLength(2);
      // Signed out, it stops.
      again.fail(new ApiError({ code: ErrorCode.NO_SIGN_IN, message: 'signed out' }));
      await vi.advanceTimersByTimeAsync(60_000);
      expect(fake.paramsOf('thread/list/watch')).toHaveLength(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it('lets go of its watch while the tab is hidden, and reads the lists again when shown', async () => {
    const fake = new FakeSession()
      .answer('project/list', { projects: [] })
      .answer('thread/list', { threads: [] });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    const shown = (state: DocumentVisibilityState): void => {
      Object.defineProperty(document, 'visibilityState', { value: state, configurable: true });
      document.dispatchEvent(new Event('visibilitychange'));
    };
    try {
      // Read for the first page, but not watched while hidden.
      shown('hidden');
      await following(projects);
      expect(fake.paramsOf('project/list')).toHaveLength(1);
      expect(fake.paramsOf('thread/list/watch')).toHaveLength(0);
      shown('visible');
      await settled();
      shown('hidden');
      await settled();
      shown('visible');
      await settled();
      expect(fake.paramsOf('project/list')).toHaveLength(3);
      // With nothing listed, from when the lists were read.
      const from = { after: expect.stringMatching(/Z$/u) as unknown };
      expect(fake.paramsOf('thread/list/watch')).toEqual([from, from]);
    } finally {
      Reflect.deleteProperty(document, 'visibilityState');
    }
  });

  it('lets go of its watch once the tab signs out', async () => {
    const watch = new FakeTurn();
    const fake = new FakeSession()
      .answer('project/list', { projects: [record('project-1', 'glave')] })
      .answer('thread/list', { threads: [thread('t1', 'Plan')] })
      .listWatch(watch);
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    await following(projects);
    await TestBed.inject(SessionService).signOut();
    TestBed.tick();
    await settled();
    // What the watch let go of would say is never taken.
    watch.send(changed(thread('t1', 'Plan', { status: 'running' })));
    await settled();
    expect(projects.thread('t1')?.status).toBe('idle');
    // Signed out, following again watches nothing.
    projects.follow();
    await settled();
    expect(fake.paramsOf('thread/list/watch')).toHaveLength(1);
  });

  it('moves its times on each minute while it follows the lists', async () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(Date.parse('2026-10-11T10:00:00.000Z'));
      const fake = new FakeSession().answer('project/list', { projects: [] });
      TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
      const projects = TestBed.inject(ProjectsService);
      projects.follow();
      await vi.dynamicImportSettled();
      await vi.advanceTimersByTimeAsync(CLOCK_MS);
      expect(projects.readAt()).toBe(Date.parse('2026-10-11T10:01:00.000Z'));
    } finally {
      vi.useRealTimers();
    }
  });
});
