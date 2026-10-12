import type { Space, SpaceId, ThreadSummary } from '@glave/client';

import { type ThreadMark, threadMark, threadTitle } from './threads';

/** The id and name of the group that holds the person's loose chats (their personal spaces). */
const LOOSE_GROUP = 'chats';
const LOOSE_NAME = 'Chats';

/** A space with its threads, as the spaces service reads them. */
export interface SpaceWithThreads {
  readonly space: Space;
  readonly threads: readonly ThreadSummary[];
}

/** A thread's line in the side nav. */
interface NavThread {
  readonly id: string;
  readonly link: readonly string[];
  readonly title: string;
  readonly mark: ThreadMark | null;
  readonly current: boolean;
}

/** A group of threads in the side nav: one project, or the loose chats. */
interface NavGroup {
  readonly id: string;
  readonly name: string;
  /** The space a new thread from this group starts in. */
  readonly space: SpaceId;
  readonly threads: readonly NavThread[];
  readonly expanded: boolean;
}

/** What the side nav shows: the search, the thread shown now and the groups closed. */
export interface NavView {
  readonly query: string;
  readonly current: string | null;
  readonly collapsed: ReadonlySet<string>;
}

/** Whether `title` has `query` in it, ignoring case and accents; an empty query matches all. */
export function matchesQuery(title: string, query: string): boolean {
  const wanted = fold(query.trim());
  return wanted === '' || fold(title).includes(wanted);
}

/**
 * The side nav's groups: each project with its threads, then the loose chats (every personal
 * space's threads together). A search keeps only the threads whose title matches, and the
 * groups that have one, all shown open; without one, every group shows, closed as the person
 * left it.
 */
export function navGroups(spaces: readonly SpaceWithThreads[], view: NavView): readonly NavGroup[] {
  const searching = view.query.trim() !== '';
  const line = (thread: ThreadSummary): NavThread => ({
    id: thread.id,
    link: ['/t', thread.id],
    title: threadTitle(thread),
    mark: threadMark(thread),
    current: thread.id === view.current,
  });
  const projects = spaces
    .filter(({ space }) => space.kind === 'project')
    .map(({ space, threads }) => ({ id: space.id, name: space.name, space: space.id, threads }));
  const personal = spaces.filter(({ space }) => space.kind === 'personal');
  const first = personal.at(0);
  const loose =
    first === undefined
      ? []
      : [
          {
            id: LOOSE_GROUP,
            name: LOOSE_NAME,
            space: first.space.id,
            threads: [...personal.flatMap(({ threads }) => threads)].sort((a, b) =>
              b.updated_at.localeCompare(a.updated_at),
            ),
          },
        ];
  return [...projects, ...loose]
    .map((group) => ({
      id: group.id,
      name: group.name,
      space: group.space,
      threads: group.threads.map(line).filter((thread) => matchesQuery(thread.title, view.query)),
      expanded: searching || !view.collapsed.has(group.id),
    }))
    .filter((group) => !searching || group.threads.length > 0);
}

function fold(text: string): string {
  return text
    .normalize('NFD')
    .replace(/\p{Diacritic}/gu, '')
    .toLowerCase();
}
