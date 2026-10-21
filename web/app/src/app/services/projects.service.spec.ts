import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type Project } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { record, thread } from '../testing/fixtures';
import { REFRESH_MS, ProjectsService } from './projects.service';

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
    expect(projects.projectOf('t1')?.name).toBe('glave');
    expect(projects.projectOf('nope')).toBeUndefined();
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
    const fake = new FakeSession()
      .answer('project/list', { projects: [] })
      .answer(
        'project/rename',
        new ApiError({ code: ErrorCode.NOT_ALLOWED, message: 'only its owner renames it' }),
      );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const projects = TestBed.inject(ProjectsService);
    await expect(projects.rename('p1', 'x')).rejects.toThrow('only its owner renames it');
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

  it('polls only while the tab is shown', () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    const hidden = vi.spyOn(document, 'hidden', 'get').mockReturnValue(true);
    try {
      const fake = new FakeSession().answer('project/list', { projects: [] });
      TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
      TestBed.inject(ProjectsService);
      vi.advanceTimersByTime(REFRESH_MS * 3);
      expect(fake.paramsOf('project/list')).toHaveLength(0);
      hidden.mockReturnValue(false);
      vi.advanceTimersByTime(REFRESH_MS);
      expect(fake.paramsOf('project/list')).toHaveLength(1);
    } finally {
      hidden.mockRestore();
      vi.useRealTimers();
    }
  });
});
