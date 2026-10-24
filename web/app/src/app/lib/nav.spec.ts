import type { Project, ThreadSummary } from '@glave/client';
import { TriangleAlert } from 'lucide';

import { record, thread } from '../testing/fixtures';
import {
  CHATS_LIST,
  type NavSections,
  type NavThread,
  type NavView,
  PROJECTS_LIST,
  needsYou,
  type ProjectWithThreads,
  hasProjects,
  matchesQuery,
  navSections,
  statusSections,
} from './nav';

/** A time on the fixtures' day, at 10:`minute`. */
function at(minute: number): string {
  return `2026-10-11T10:${String(minute).padStart(2, '0')}:00.000Z`;
}

function made(id: string, name: string, minute: number): Project {
  return { ...record(id, name), created_at: at(minute) };
}

function line(
  id: string,
  title: string | null,
  minute: number,
  more: Partial<ThreadSummary> = {},
): ThreadSummary {
  return thread(id, title, { updated_at: at(minute), ...more });
}

/** The side nav at 10:10 on the fixtures' day, for a reader in English in UTC. */
const VIEW: NavView = {
  query: '',
  current: null,
  collapsed: new Set(),
  now: Date.parse(at(10)),
  reader: { locale: 'en-US', timeZone: 'UTC' },
};

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

const PROJECTS: readonly ProjectWithThreads[] = [
  {
    project: made('p1', 'glave', 0),
    threads: [line('t1', 'Plan the launch', 5), line('t2', 'Fix the build', 7)],
  },
  { project: made('p2', 'Garden', 0), threads: [line('t3', 'Plant the beans', 9)] },
  { project: made('p3', 'Notes', 8), threads: [] },
  { project: record('me', 'Personal', 'personal'), threads: [line('t4', 'Café ideas', 0)] },
  { project: record('me2', 'Shared with me', 'personal'), threads: [line('t6', null, 8)] },
];

describe('nav', () => {
  it('finds titles ignoring case and accents', () => {
    expect(matchesQuery('Café ideas', [], 'cafe')).toBe(true);
    expect(matchesQuery('Café ideas', [], '  ')).toBe(true);
    expect(matchesQuery('Café ideas', [], 'tea')).toBe(false);
  });

  it('finds a day by the start of each of its words, in any order', () => {
    const days = ['Sunday, October 11, 2026', 'Oct 11, 2026', '10/11/2026', '2026-10-11', 'today'];
    for (const query of ['oct 11', '11 Oct', 'october', 'SUN', '10/11', '2026-10', 'today']) {
      expect(matchesQuery('Plan the launch', days, query)).toBe(true);
    }
    for (const query of ['oct 12', 'yesterday', 'Mon', 'tober']) {
      expect(matchesQuery('Plan the launch', days, query)).toBe(false);
    }
  });

  it('looks at days only for a query of a few characters that is not a number alone', () => {
    const days = ['Sunday, October 11, 2026', '2026-10-11'];
    // Two letters, or a year or a day alone, would find nearly every thread.
    for (const query of ['oc', '2026', '11', '10 11']) {
      expect(matchesQuery('Plan the launch', days, query)).toBe(false);
    }
    expect(matchesQuery('Plan 2026', days, '2026')).toBe(true);
  });

  it('lists the projects by their latest thread, each one’s threads and the chats newest first', () => {
    const sections = navSections(PROJECTS, { ...VIEW, current: 't1' });
    // Notes has no thread yet: it counts from when it was made.
    expect(projectNames(sections)).toEqual(['Garden', 'Notes', 'glave']);
    const glave = sections.projects.list[2];
    expect(titles(glave?.threads ?? [])).toEqual(['Fix the build', 'Plan the launch']);
    expect(glave?.threads[1]).toMatchObject({
      link: ['/t', 't1'],
      current: true,
      running: false,
      line: null,
    });
    expect(titles(sections.chats.list)).toEqual(['New chat', 'Café ideas']);
    expect(sections.projects).toMatchObject({ expanded: true, mark: null });
    expect(sections.chats).toMatchObject({ expanded: true, mark: null });
  });

  it('gives each thread when it was last active, counted from the view’s moment', () => {
    const { chats } = navSections(PROJECTS, VIEW);
    expect(chats.list.map(({ since }) => [since.short, since.long])).toEqual([
      ['2m', '2 minutes ago'],
      ['10m', '10 minutes ago'],
    ]);
    expect(chats.list[1]?.since.at).toBe(at(0));
  });

  it('keeps one order however the threads stand: what waits or runs is not lifted', () => {
    const projects: readonly ProjectWithThreads[] = [
      { project: made('p1', 'Alpha', 0), threads: [line('a1', 'Ship it', 9)] },
      {
        project: made('p2', 'Beta', 0),
        threads: [
          line('b1', 'Plan the launch', 5, { status: 'running' }),
          line('b2', 'Write the notes', 1, { status: 'waiting' }),
          line('b3', 'Audit the dependencies', 3),
        ],
      },
    ];
    const sections = navSections(projects, VIEW);
    expect(projectNames(sections)).toEqual(['Alpha', 'Beta']);
    expect(titles(sections.projects.list[1]?.threads ?? [])).toEqual([
      'Plan the launch',
      'Audit the dependencies',
      'Write the notes',
    ]);
    expect(
      sections.projects.list[1]?.threads.map(({ running, line: under }) => [running, under?.text]),
    ).toEqual([
      [true, undefined],
      [false, undefined],
      [false, 'Waiting on you'],
    ]);
  });

  it('puts a thread another started under it, which lifts it when it is newer', () => {
    const threads = [
      line('t1', 'Plan the launch', 5),
      line('t2', 'Fix the build', 7),
      line('t3', 'Check the parser', 9, { parent: 't1' }),
      line('t4', 'Read the logs', 2, { parent: 't3', status: 'waiting' }),
      line('t5', 'Started by one not listed', 1, { parent: 'gone' }),
    ];
    const { projects } = navSections([{ project: made('p1', 'glave', 0), threads }], VIEW);
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
    const lists: readonly ProjectWithThreads[] = [
      { project: made('p1', 'glave', 0), threads: [line('t1', 'Fix', 1, { status: 'waiting' })] },
      { project: made('p2', 'Garden', 0), threads: [line('t2', 'Plant', 2)] },
      { project: record('me', 'Personal', 'personal'), threads: [line('t3', 'Trip', 3)] },
    ];
    const collapsed = new Set(['p1', 'p2', PROJECTS_LIST, CHATS_LIST]);
    const { projects, chats } = navSections(lists, { ...VIEW, collapsed });
    expect(projects.expanded).toBe(false);
    expect(projects.mark).toEqual({ ...needsYou(), warning: false });
    expect(projects.list.map((each) => [each.name, each.expanded, each.mark?.label])).toEqual([
      ['Garden', false, undefined],
      ['glave', false, 'needs you'],
    ]);
    expect(chats).toMatchObject({ expanded: false, mark: null });
    // Open, a list shows its threads' own marks instead.
    expect(navSections(lists, VIEW).projects.list[1]?.mark).toBeNull();
  });

  it('marks a project whose folder is gone in the server’s words, open or closed', () => {
    const gone = "This project's folder isn't there anymore.";
    const projects: readonly ProjectWithThreads[] = [
      {
        project: made('p1', 'playground', 0),
        threads: [line('t1', 'Sketch', 3, { status: 'waiting' })],
        gone,
      },
      { project: made('p2', 'Garden', 0), threads: [line('t2', 'Plant', 2)] },
    ];
    for (const collapsed of [new Set<string>(), new Set(['p1'])]) {
      const [playground, garden] = navSections(projects, { ...VIEW, collapsed }).projects.list;
      expect(playground?.gone).toBe(gone);
      expect(playground?.mark).toEqual({ icon: TriangleAlert, label: gone, warning: true });
      expect(garden?.gone).toBeNull();
    }
  });

  it('keeps only the threads a search finds and their projects, open', () => {
    const cafe = navSections(PROJECTS, { ...VIEW, query: 'CAFE' });
    expect(titles(cafe.chats.list)).toEqual(['Café ideas']);
    expect(cafe.projects.list).toEqual([]);
    const collapsed = new Set(['p1', PROJECTS_LIST]);
    const build = navSections(PROJECTS, { ...VIEW, query: 'build', collapsed });
    expect(build.chats.list).toEqual([]);
    expect(build.projects.expanded).toBe(true);
    expect(build.projects.list.map((project) => [project.id, project.expanded])).toEqual([
      ['p1', true],
    ]);
    // A matching thread another started shows on its own when that one does not match.
    const child = [line('t1', 'Plan', 5), line('t2', 'Fix the build', 7, { parent: 't1' })];
    const found = navSections([{ project: made('p1', 'glave', 0), threads: child }], {
      ...VIEW,
      query: 'build',
    });
    expect(titles(found.projects.list[0]?.threads ?? [])).toEqual(['Fix the build']);
  });

  it('finds threads by the day they began or were last active', () => {
    const threads = [
      line('t1', 'Plan the launch', 5),
      thread('t2', 'Fix the build', {
        created_at: '2026-10-08T09:00:00.000Z',
        updated_at: '2026-10-08T09:30:00.000Z',
      }),
      thread('t3', 'Plant the beans', {
        created_at: '2026-10-08T09:00:00.000Z',
        updated_at: '2026-10-10T18:00:00.000Z',
      }),
    ];
    const lists = [{ project: record('me', 'Personal', 'personal'), threads }];
    const found = (query: string): readonly string[] =>
      titles(navSections(lists, { ...VIEW, query }).chats.list);
    expect(found('today')).toEqual(['Plan the launch']);
    expect(found('yesterday')).toEqual(['Plant the beans']);
    expect(found('Oct 8')).toEqual(['Plant the beans', 'Fix the build']);
    expect(found('thursday')).toEqual(['Plant the beans', 'Fix the build']);
    expect(found('Oct 9')).toEqual([]);
  });

  it('says under the title of each thread that waits, and of no other, that it waits', () => {
    const threads = [
      line('t1', 'Plan', 5),
      line('t2', 'Fix the build', 7, { status: 'waiting' }),
      line('t3', 'Read the logs', 6, { parent: 't1', status: 'waiting' }),
      line('t4', 'Audit', 4, { status: 'running' }),
    ];
    const { projects } = navSections([{ project: made('p1', 'glave', 0), threads }], VIEW);
    const [fix, plan, audit] = projects.list[0]?.threads ?? [];
    const waits = { text: 'Waiting on you', needsYou: true };
    expect([fix?.line, plan?.line, plan?.children[0]?.line, audit?.line]).toEqual([
      waits,
      null,
      waits,
      null,
    ]);
  });

  it('groups the threads by what each is doing, each once, newest first, with its place', () => {
    const lists: readonly ProjectWithThreads[] = [
      {
        project: made('p1', 'glave', 0),
        threads: [
          line('t1', 'Plan', 5, { status: 'running' }),
          line('t2', 'Fix the build', 7, { status: 'waiting' }),
          line('t3', 'Read the logs', 9, { parent: 't1', status: 'waiting' }),
          line('t4', 'Audit', 4),
        ],
      },
      {
        project: record('me', 'Personal', 'personal'),
        threads: [line('t5', 'Café ideas', 8), line('t6', 'Trip', 1, { status: 'running' })],
      },
    ];
    const sections = statusSections(lists, { ...VIEW, current: 't2' });
    const shown = (threads: readonly NavThread[]): readonly string[] =>
      threads.map(({ title, line: under }) => `${title} · ${under?.text ?? ''}`);
    // A thread another started is listed on its own, where what it does puts it.
    expect(shown(sections.waiting)).toEqual(['Read the logs · glave', 'Fix the build · glave']);
    expect(shown(sections.running)).toEqual(['Plan · glave', 'Trip · Chats']);
    expect(shown(sections.idle)).toEqual(['Café ideas · Chats', 'Audit · glave']);
    expect(sections.running[0]?.children).toEqual([]);
    expect(sections.waiting[1]).toMatchObject({
      link: ['/t', 't2'],
      current: true,
      running: false,
      line: { text: 'glave', needsYou: false },
    });
    expect(statusSections([], VIEW)).toEqual({ waiting: [], running: [], idle: [] });
  });

  it('keeps only the threads a search finds, grouped by what each is doing', () => {
    const lists: readonly ProjectWithThreads[] = [
      {
        project: made('p1', 'glave', 0),
        threads: [line('t1', 'Fix the build', 7, { status: 'waiting' }), line('t2', 'Café', 3)],
      },
    ];
    const found = statusSections(lists, { ...VIEW, query: 'cafe' });
    expect(found.waiting).toEqual([]);
    expect(found.idle.map(({ id }) => id)).toEqual(['t2']);
  });

  it('knows whether the person has a project beside their Chats', () => {
    expect(hasProjects(PROJECTS)).toBe(true);
    expect(hasProjects(PROJECTS.filter(({ project }) => project.kind === 'personal'))).toBe(false);
    expect(hasProjects([])).toBe(false);
  });
});
