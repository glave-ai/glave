import { space, thread } from '../testing/fixtures';
import { type NavView, matchesQuery, navGroups } from './nav';

const SPACES = [
  {
    space: space('p1', 'glave'),
    threads: [
      thread('t1', 'Plan the launch', { status: 'running' }),
      thread('t2', 'Fix the build'),
    ],
  },
  { space: space('p2', 'Garden'), threads: [] },
  {
    space: space('me', 'Personal', 'personal'),
    threads: [thread('t3', 'Café ideas', { updated_at: '2026-10-11T09:00:00.000Z' })],
  },
  {
    space: space('me2', 'Shared with me', 'personal'),
    threads: [thread('t4', null, { updated_at: '2026-10-11T11:00:00.000Z' })],
  },
];

const VIEW: NavView = { query: '', current: 't2', collapsed: new Set(['p2']) };

describe('nav', () => {
  it('matches titles ignoring case and accents', () => {
    expect(matchesQuery('Café ideas', 'cafe')).toBe(true);
    expect(matchesQuery('Café ideas', '  ')).toBe(true);
    expect(matchesQuery('Café ideas', 'tea')).toBe(false);
  });

  it('groups each project, then the loose chats newest first', () => {
    const groups = navGroups(SPACES, VIEW);
    expect(groups.map((group) => [group.id, group.name, group.space, group.expanded])).toEqual([
      ['p1', 'glave', 'p1', true],
      ['p2', 'Garden', 'p2', false],
      ['chats', 'Chats', 'me', true],
    ]);
    expect(groups[0]?.threads.map((each) => [each.title, each.current, each.mark?.text])).toEqual([
      ['Plan the launch', false, 'running'],
      ['Fix the build', true, undefined],
    ]);
    expect(groups[0]?.threads[0]?.link).toEqual(['/t', 't1']);
    expect(groups[2]?.threads.map((each) => each.title)).toEqual(['New chat', 'Café ideas']);
  });

  it('keeps only the matching threads and their groups, open, while searching', () => {
    const groups = navGroups(SPACES, { ...VIEW, query: 'CAFE', collapsed: new Set(['chats']) });
    expect(groups.map((group) => [group.id, group.expanded])).toEqual([['chats', true]]);
    expect(groups[0]?.threads.map((each) => each.title)).toEqual(['Café ideas']);
    expect(navGroups(SPACES, { ...VIEW, query: 'nothing' })).toEqual([]);
  });

  it('has no loose group without a personal space', () => {
    expect(navGroups(SPACES.slice(0, 2), VIEW).map((group) => group.id)).toEqual(['p1', 'p2']);
  });
});
