import { space, thread } from '../testing/fixtures';
import { type NavView, matchesQuery, navSections, waitingThreads } from './nav';

const SPACES = [
  {
    space: space('p1', 'glave'),
    threads: [
      thread('t1', 'Plan the launch', { status: 'running', space: 'p1' }),
      thread('t2', 'Fix the build', { status: 'waiting', space: 'p1' }),
    ],
  },
  { space: space('p2', 'Garden'), threads: [] },
  {
    space: space('me', 'Personal', 'personal'),
    threads: [
      thread('t3', 'Café ideas', {
        space: 'me',
        status: 'waiting',
        updated_at: '2026-10-11T09:00:00.000Z',
      }),
    ],
  },
  {
    space: space('me2', 'Shared with me', 'personal'),
    threads: [thread('t4', null, { space: 'me2', updated_at: '2026-10-11T11:00:00.000Z' })],
  },
];

const VIEW: NavView = { query: '', current: 't2', collapsed: new Set(['p2']) };

describe('nav', () => {
  it('matches titles ignoring case and accents', () => {
    expect(matchesQuery('Café ideas', 'cafe')).toBe(true);
    expect(matchesQuery('Café ideas', '  ')).toBe(true);
    expect(matchesQuery('Café ideas', 'tea')).toBe(false);
  });

  it('lists the chats newest first, then each project, closed as the person left it', () => {
    const { chats, projects } = navSections(SPACES, VIEW);
    expect(chats.map((each) => each.title)).toEqual(['New chat', 'Café ideas']);
    expect(projects.map((project) => [project.id, project.name, project.expanded])).toEqual([
      ['p1', 'glave', true],
      ['p2', 'Garden', false],
    ]);
    expect(
      projects[0]?.threads.map((each) => [each.title, each.current, each.mark?.variant]),
    ).toEqual([
      ['Plan the launch', false, 'plain'],
      ['Fix the build', true, 'dot'],
    ]);
    expect(projects[0]?.threads[0]?.link).toEqual(['/t', 't1']);
  });

  it('keeps only the matching threads and their projects, open, while searching', () => {
    const cafe = navSections(SPACES, { ...VIEW, query: 'CAFE' });
    expect(cafe.chats.map((each) => each.title)).toEqual(['Café ideas']);
    expect(cafe.projects).toEqual([]);
    const build = navSections(SPACES, { ...VIEW, query: 'build', collapsed: new Set(['p1']) });
    expect(build.chats).toEqual([]);
    expect(build.projects.map((project) => [project.id, project.expanded])).toEqual([['p1', true]]);
  });

  it('lists every thread that needs the person, newest first, with its place', () => {
    expect(waitingThreads(SPACES)).toEqual([
      { id: 't2', link: ['/t', 't2'], title: 'Fix the build', place: 'glave' },
      { id: 't3', link: ['/t', 't3'], title: 'Café ideas', place: 'Chats' },
    ]);
    expect(waitingThreads([])).toEqual([]);
  });
});
