import type { Space, ThreadSummary } from '@glave/client';

import { type ThreadMark, threadMark, threadTitle } from './threads';

/** What the side nav calls the person's own chats (their personal spaces' threads). */
const CHATS = 'Chats';

/** The side nav's own lists, whose state is kept beside each project's (its space's id). */
export const PROJECTS_LIST = 'projects';
export const CHATS_LIST = 'chats';

/** How a list in the side nav is sorted: by its latest activity (the default), or by name. */
export type NavSort = 'recent' | 'name';

/** A space with its threads, as the spaces service reads them. */
export interface SpaceWithThreads {
  readonly space: Space;
  readonly threads: readonly ThreadSummary[];
}

/** A thread's line in the side nav, with the threads it started under it. */
export interface NavThread {
  readonly id: string;
  readonly link: readonly string[];
  readonly title: string;
  readonly mark: ThreadMark | null;
  readonly current: boolean;
  readonly children: readonly NavThread[];
  readonly hasChildren: boolean;
}

/** A heading in the side nav, and how the list under it shows. */
interface NavHeading {
  readonly expanded: boolean;
  readonly sort: NavSort;
  /** While closed with something in it waiting for the person, the mark that says so. */
  readonly mark: ThreadMark | null;
}

/** A project in the side nav, with its threads. */
export interface NavProject extends NavHeading {
  /** The project's space, where a new thread from its + starts. */
  readonly id: string;
  readonly name: string;
  readonly threads: readonly NavThread[];
}

/** What the side nav lists: each project with its threads, then the person's own chats. */
export interface NavSections {
  readonly projects: NavHeading & { readonly list: readonly NavProject[] };
  /** Every personal space's threads together. */
  readonly chats: NavHeading & { readonly list: readonly NavThread[] };
}

/**
 * What the side nav shows: the search, the thread shown now, and the lists the person closed
 * and those they sorted by name (`PROJECTS_LIST`, `CHATS_LIST` or a project's id).
 */
export interface NavView {
  readonly query: string;
  readonly current: string | null;
  readonly collapsed: ReadonlySet<string>;
  readonly byName: ReadonlySet<string>;
}

/** A thread that needs the person, as the Needs you list shows it: its title and its place. */
interface WaitingThread {
  readonly id: string;
  readonly link: readonly string[];
  readonly title: string;
  /** Its project's name, or Chats. */
  readonly place: string;
}

/** An item and what its list is sorted on: whether it waits, its latest activity, its name. */
interface Ranked<T> {
  readonly item: T;
  readonly waiting: boolean;
  readonly latest: string;
  readonly name: string;
}

/** The mark of a closed list with something in it waiting for the person. */
const WAITING_MARK = threadMark({ status: 'waiting' });

/** Names in the order people read them: "Plan 2" before "Plan 10", ignoring case. */
const NAME_ORDER: Intl.CollatorOptions = { numeric: true, sensitivity: 'base' };

/** Whether `title` has `query` in it, ignoring case and accents; an empty query matches all. */
export function matchesQuery(title: string, query: string): boolean {
  const wanted = fold(query.trim());
  return wanted === '' || fold(title).includes(wanted);
}

/**
 * The side nav's lists: each project with its threads, then the person's own chats (every
 * personal space's threads together). Each list is sorted as the person chose, by its latest
 * activity unless by name, and whatever waits for the person comes first: a project with its
 * threads, a thread with the threads it started, which sit under it. A project's activity is
 * its latest thread's (its making, before it has one). A search keeps only the threads whose
 * title matches, and the projects that have one, all shown open; without one, every project
 * shows, each list closed as the person left it.
 */
export function navSections(spaces: readonly SpaceWithThreads[], view: NavView): NavSections {
  const projects = spaces
    .filter(({ space }) => space.kind === 'project')
    .map(({ space, threads }) => rankedProject(space, threads, view))
    .filter(({ item }) => !searching(view) || item.threads.length > 0);
  const chats = threadTree(personalThreads(spaces), view, view.byName.has(CHATS_LIST));
  return {
    projects: {
      ...heading(
        PROJECTS_LIST,
        projects.some(({ waiting }) => waiting),
        view,
      ),
      list: ranked(projects, view.byName.has(PROJECTS_LIST)).map(({ item }) => item),
    },
    chats: { ...heading(CHATS_LIST, chats.waiting, view), list: chats.list },
  };
}

/** Every thread that needs the person (an approval or a question), newest first, with its place. */
export function waitingThreads(spaces: readonly SpaceWithThreads[]): readonly WaitingThread[] {
  const places = new Map(
    spaces.map(({ space }) => [space.id, space.kind === 'project' ? space.name : CHATS]),
  );
  return newestFirst(spaces.flatMap(({ threads }) => threads))
    .filter((thread) => thread.status === 'waiting')
    .map((thread) => ({
      id: thread.id,
      link: ['/t', thread.id],
      title: threadTitle(thread),
      place: places.get(thread.space) ?? CHATS,
    }));
}

function searching(view: NavView): boolean {
  return view.query.trim() !== '';
}

/** How the list `list` shows: open while searching, else as the person left it. */
function heading(list: string, waiting: boolean, view: NavView): NavHeading {
  const expanded = searching(view) || !view.collapsed.has(list);
  return {
    expanded,
    sort: view.byName.has(list) ? 'name' : 'recent',
    mark: !expanded && waiting ? WAITING_MARK : null,
  };
}

function rankedProject(
  space: Space,
  threads: readonly ThreadSummary[],
  view: NavView,
): Ranked<NavProject> {
  const tree = threadTree(threads, view, view.byName.has(space.id));
  return {
    item: {
      ...heading(space.id, tree.waiting, view),
      id: space.id,
      name: space.name,
      threads: tree.list,
    },
    waiting: tree.waiting,
    latest: [space.created_at, ...threads.map((thread) => thread.updated_at)].reduce(later),
    name: space.name,
  };
}

/** A list of threads as the side nav shows it, and whether any of them waits. */
interface ThreadTree {
  readonly list: readonly NavThread[];
  readonly waiting: boolean;
}

/**
 * The threads whose title matches the search, each under the thread that started it when that
 * one is shown too, every level sorted (`byName`, else by latest activity) with what waits
 * first. A thread waits, and was last active, with the threads under it.
 */
function threadTree(threads: readonly ThreadSummary[], view: NavView, byName: boolean): ThreadTree {
  const shown = threads.filter((thread) => matchesQuery(threadTitle(thread), view.query));
  const ids = new Set(shown.map((thread) => thread.id));
  const parentOf = (thread: ThreadSummary): string | null =>
    typeof thread.parent === 'string' && ids.has(thread.parent) ? thread.parent : null;
  const level = (parent: string | null): readonly Ranked<NavThread>[] =>
    ranked(
      shown
        .filter((thread) => parentOf(thread) === parent)
        .map((thread) => {
          const children = level(thread.id);
          const title = threadTitle(thread);
          return {
            item: {
              id: thread.id,
              link: ['/t', thread.id],
              title,
              mark: threadMark(thread),
              current: thread.id === view.current,
              children: children.map(({ item }) => item),
              hasChildren: children.length > 0,
            },
            waiting: thread.status === 'waiting' || children.some(({ waiting }) => waiting),
            latest: [thread.updated_at, ...children.map(({ latest }) => latest)].reduce(later),
            name: title,
          };
        }),
      byName,
    );
  const roots = level(null);
  return { list: roots.map(({ item }) => item), waiting: roots.some(({ waiting }) => waiting) };
}

/** `items` with what waits first, then by name (`byName`) or the latest activity first. */
function ranked<T>(items: readonly Ranked<T>[], byName: boolean): readonly Ranked<T>[] {
  return [...items].sort(
    (a, b) =>
      Number(b.waiting) - Number(a.waiting) ||
      (byName ? a.name.localeCompare(b.name, undefined, NAME_ORDER) : 0) ||
      b.latest.localeCompare(a.latest),
  );
}

/** The later of two of the server's times: each has one spelling, so their text sorts. */
function later(a: string, b: string): string {
  return a.localeCompare(b) >= 0 ? a : b;
}

function personalThreads(spaces: readonly SpaceWithThreads[]): readonly ThreadSummary[] {
  return spaces.filter(({ space }) => space.kind === 'personal').flatMap(({ threads }) => threads);
}

function newestFirst(threads: readonly ThreadSummary[]): readonly ThreadSummary[] {
  return [...threads].sort((a, b) => b.updated_at.localeCompare(a.updated_at));
}

function fold(text: string): string {
  return text
    .normalize('NFD')
    .replace(/\p{Diacritic}/gu, '')
    .toLowerCase();
}
