import type { ThreadSummary } from '@glave/client';

import type { ProjectWithThreads } from './nav';

/**
 * `lists` with `summary` in its place, as the server says its thread is now: in its project's
 * list, newest first among the newest `limit`, or gone from it once it is not active (archived
 * or deleted). `null` when none of the lists is its project's.
 */
export function withSummary(
  lists: readonly ProjectWithThreads[],
  summary: ThreadSummary,
  limit: number,
): readonly ProjectWithThreads[] | null {
  const at = lists.findIndex(({ project }) => project.id === summary.project);
  if (at < 0) {
    return null;
  }
  return lists.map((entry, index) => {
    if (index !== at) {
      return entry;
    }
    const others = entry.threads.filter((thread) => thread.id !== summary.id);
    const threads = summary.standing === 'active' ? [summary, ...others] : others;
    threads.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
    return { ...entry, threads: threads.slice(0, limit) };
  });
}

/** The latest `updated_at` of the threads `lists` hold, if any: where a watch of them picks up. */
export function newestUpdate(lists: readonly ProjectWithThreads[]): string | undefined {
  let newest: string | undefined;
  for (const { threads } of lists) {
    for (const { updated_at: at } of threads) {
      if (newest === undefined || at > newest) {
        newest = at;
      }
    }
  }
  return newest;
}
