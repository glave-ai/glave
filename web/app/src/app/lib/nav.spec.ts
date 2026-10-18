import type { Space, ThreadSummary } from '@glave/client';

import { space, thread } from '../testing/fixtures';
import {
  CHATS_LIST,
  type NavSections,
  type NavThread,
  type NavView,
  PROJECTS_LIST,
  type SpaceWithThreads,
  matchesQuery,
  navSections,
  waitingThreads,
} from './nav';

/** A time on the fixtures' day, at 10:`minute`. */
function at(minute: number): string {
  return `2026-10-11T10:${String(minute).padStart(2, '0')}:00.000Z`;
}

function made(id: string, name: string, minute: number): Space {
  return { ...space(id, name), created_at: at(minute) };
}

function line(
  id: string,
  title: string | null,
  minute: number,
  more: Partial<ThreadSummary> = {},
): ThreadSummary {
  return thread(id, title, { updated_at: at(minute), ...more });
}

const VIEW: NavView = { query: '', current: null, collapsed: new Set(), byName: new Set() };

/** The titles of `threads`, each with those it started after it, indented. */
function titles(threads: readonly NavThread[], indent = ''): readonly string[] {
  return threads.flatMap((each) => [
    `${indent}${each.title}`,
    ...titles(each.children, `${indent}  `),
  ]);
}

function projectNames(sections: NavSections): readonly string[] {
  return sections.projects.list.map((project) => project.name);
}

const SPACES: readonly SpaceWithThreads[] = [
  {
    space: made('p1', 'glave', 0),
    threads: [line('t1', 'Plan the launch', 5), line('t2', 'Fix the build', 7)],
  },
  { space: made('p2', 'Garden', 0), threads: [line('t3', 'Plant the beans', 9)] },
  { space: made('p3', 'Notes', 8), threads: [] },
  { space: space('me', 'Personal', 'personal'), threads: [line('t4', 'Café ideas', 0)] },
  { space: space('me2', 'Shared with me', 'personal'), threads: [line('t6', null, 8)] },
];

describe('nav', () => {
  it('matches titles ignoring case and accents', () => {
    expect(matchesQuery('Café ideas', 'cafe')).toBe(true);
    expect(matchesQuery('Café ideas', '  ')).toBe(true);
    expect(matchesQuery('Café ideas', 'tea')).toBe(false);
  });

  it('lists the projects by their latest thread, each one’s threads and the chats newest first', () => {
    const sections = navSections(SPACES, { ...VIEW, current: 't1' });
    // Notes has no thread yet: it counts from when it was made.
    expect(projectNames(sections)).toEqual(['Garden', 'Notes', 'glave']);
    const glave = sections.projects.list[2];
    expect(titles(glave?.threads ?? [])).toEqual(['Fix the build', 'Plan the launch']);
    expect(glave?.threads[1]).toMatchObject({ link: ['/t', 't1'], current: true, mark: null });
    expect(titles(sections.chats.list)).toEqual(['New chat', 'Café ideas']);
    expect(sections.projects).toMatchObject({ expanded: true, sort: 'recent', mark: null });
    expect(sections.chats).toMatchObject({ expanded: true, sort: 'recent', mark: null });
  });

  it('puts what waits for the person first, whatever the sort', () => {
    const spaces: readonly SpaceWithThreads[] = [
      { space: made('p1', 'Alpha', 0), threads: [line('a1', 'Ship it', 20)] },
      {
        space: made('p2', 'Beta', 0),
        threads: [
          line('b1', 'Plan the launch', 5, { status: 'running' }),
          line('b2', 'Write the notes', 1, { status: 'waiting' }),
          line('b3', 'Audit the dependencies', 3),
        ],
      },
    ];
    const recent = navSections(spaces, VIEW);
    expect(projectNames(recent)).toEqual(['Beta', 'Alpha']);
    expect(titles(recent.projects.list[0]?.threads ?? [])).toEqual([
      'Write the notes',
      'Plan the launch',
      'Audit the dependencies',
    ]);
    expect(recent.projects.list[0]?.threads[0]?.mark?.variant).toBe('dot');
    const byName = navSections(spaces, { ...VIEW, byName: new Set(['p2']) });
    expect(byName.projects.list[0]?.sort).toBe('name');
    expect(titles(byName.projects.list[0]?.threads ?? [])).toEqual([
      'Write the notes',
      'Audit the dependencies',
      'Plan the launch',
    ]);
  });

  it('sorts a list by name as people read names, when the person asks', () => {
    const spaces: readonly SpaceWithThreads[] = [
      { space: made('p1', 'plan 10', 9), threads: [] },
      { space: made('p2', 'Plan 2', 1), threads: [] },
      { space: made('p3', 'garden', 5), threads: [] },
      {
        space: space('me', 'Personal', 'personal'),
        threads: [line('t1', 'Zebra facts', 9), line('t2', 'apples', 1)],
      },
    ];
    expect(projectNames(navSections(spaces, VIEW))).toEqual(['plan 10', 'garden', 'Plan 2']);
    const byName = navSections(spaces, { ...VIEW, byName: new Set([PROJECTS_LIST, CHATS_LIST]) });
    expect(byName.projects.sort).toBe('name');
    expect(projectNames(byName)).toEqual(['garden', 'Plan 2', 'plan 10']);
    expect(titles(byName.chats.list)).toEqual(['apples', 'Zebra facts']);
  });

  it('puts a thread another started under it, which lifts it when it is newer or waits', () => {
    const threads = [
      line('t1', 'Plan the launch', 5),
      line('t2', 'Fix the build', 7),
      line('t3', 'Check the parser', 9, { parent: 't1' }),
      line('t4', 'Read the logs', 2, { parent: 't3', status: 'waiting' }),
      line('t5', 'Started by one not listed', 1, { parent: 'gone' }),
    ];
    const { projects } = navSections([{ space: made('p1', 'glave', 0), threads }], VIEW);
    const tree = projects.list[0]?.threads ?? [];
    expect(titles(tree)).toEqual([
      'Plan the launch',
      '  Check the parser',
      '    Read the logs',
      'Fix the build',
      'Started by one not listed',
    ]);
    expect(tree.map((each) => each.hasChildren)).toEqual([true, false, false]);
  });

  it('keeps each list closed as the person left it, marked while something in it waits', () => {
    const spaces: readonly SpaceWithThreads[] = [
      { space: made('p1', 'glave', 0), threads: [line('t1', 'Fix', 1, { status: 'waiting' })] },
      { space: made('p2', 'Garden', 0), threads: [line('t2', 'Plant', 2)] },
      { space: space('me', 'Personal', 'personal'), threads: [line('t3', 'Trip', 3)] },
    ];
    const collapsed = new Set(['p1', 'p2', PROJECTS_LIST, CHATS_LIST]);
    const { projects, chats } = navSections(spaces, { ...VIEW, collapsed });
    expect(projects.expanded).toBe(false);
    expect(projects.mark).toMatchObject({ text: 'needs you', variant: 'dot' });
    expect(projects.list.map((each) => [each.name, each.expanded, each.mark?.text])).toEqual([
      ['glave', false, 'needs you'],
      ['Garden', false, undefined],
    ]);
    expect(chats).toMatchObject({ expanded: false, mark: null });
    // Open, a list shows its threads' own marks instead.
    expect(navSections(spaces, VIEW).projects.list[0]?.mark).toBeNull();
  });

  it('keeps only the matching threads and their projects, open, while searching', () => {
    const cafe = navSections(SPACES, { ...VIEW, query: 'CAFE' });
    expect(titles(cafe.chats.list)).toEqual(['Café ideas']);
    expect(cafe.projects.list).toEqual([]);
    const collapsed = new Set(['p1', PROJECTS_LIST]);
    const build = navSections(SPACES, { ...VIEW, query: 'build', collapsed });
    expect(build.chats.list).toEqual([]);
    expect(build.projects.expanded).toBe(true);
    expect(build.projects.list.map((project) => [project.id, project.expanded])).toEqual([
      ['p1', true],
    ]);
    // A matching thread another started shows on its own when that one does not match.
    const child = [line('t1', 'Plan', 5), line('t2', 'Fix the build', 7, { parent: 't1' })];
    const found = navSections([{ space: made('p1', 'glave', 0), threads: child }], {
      ...VIEW,
      query: 'build',
    });
    expect(titles(found.projects.list[0]?.threads ?? [])).toEqual(['Fix the build']);
  });

  it('lists every thread that needs the person, newest first, with its place', () => {
    const spaces: readonly SpaceWithThreads[] = [
      {
        space: space('p1', 'glave'),
        threads: [
          line('t1', 'Plan', 5, { space: 'p1' }),
          line('t2', 'Fix the build', 7, { space: 'p1', status: 'waiting' }),
        ],
      },
      {
        space: space('me', 'Personal', 'personal'),
        threads: [line('t3', 'Café ideas', 1, { space: 'me', status: 'waiting' })],
      },
    ];
    expect(waitingThreads(spaces)).toEqual([
      { id: 't2', link: ['/t', 't2'], title: 'Fix the build', place: 'glave' },
      { id: 't3', link: ['/t', 't3'], title: 'Café ideas', place: 'Chats' },
    ]);
    expect(waitingThreads([])).toEqual([]);
  });
});
