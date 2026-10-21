import { record, thread } from '../testing/fixtures';
import { newestUpdate, withSummary } from './lists';
import type { ProjectWithThreads } from './nav';

const LISTS: readonly ProjectWithThreads[] = [
  {
    project: record('project-1', 'glave'),
    threads: [
      thread('t2', 'Fix the build', { updated_at: '2026-10-11T10:07:00.000Z' }),
      thread('t1', 'Plan', { updated_at: '2026-10-11T10:05:00.000Z' }),
    ],
  },
  { project: record('me', 'Personal', 'personal'), threads: [] },
];

function ids(lists: readonly ProjectWithThreads[] | null): readonly string[] {
  return lists?.[0]?.threads.map((each) => each.id) ?? [];
}

describe('lists', () => {
  it('puts a thread’s summary in its place, newest first, keeping the newest it holds', () => {
    const moved = withSummary(
      LISTS,
      thread('t1', 'Plan', { status: 'running', updated_at: '2026-10-11T10:09:00.000Z' }),
      30,
    );
    expect(ids(moved)).toEqual(['t1', 't2']);
    expect(moved?.[0]?.threads[0]?.status).toBe('running');
    expect(moved?.[1]).toBe(LISTS[1]);
    const added = withSummary(
      LISTS,
      thread('t3', 'New', { updated_at: '2026-10-11T10:08:00.000Z' }),
      2,
    );
    expect(ids(added)).toEqual(['t3', 't2']);
  });

  it('drops a thread no longer active, and knows no project the lists lack', () => {
    expect(
      ids(withSummary(LISTS, thread('t2', 'Fix the build', { standing: 'archived' }), 30)),
    ).toEqual(['t1']);
    expect(ids(withSummary(LISTS, thread('t1', 'Plan', { standing: 'deleted' }), 30))).toEqual([
      't2',
    ]);
    expect(withSummary(LISTS, thread('t9', 'Elsewhere', { project: 'project-9' }), 30)).toBeNull();
  });

  it('knows the newest update the lists hold, if they hold any', () => {
    expect(newestUpdate(LISTS)).toBe('2026-10-11T10:07:00.000Z');
    expect(newestUpdate([])).toBeUndefined();
  });
});
