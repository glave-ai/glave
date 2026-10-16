import type { Space, ThreadSummary } from '@glave/client';

import { type ThreadMark, threadMark, threadTitle } from './threads';

/** What the side nav calls the person's own chats (their personal spaces' threads). */
const CHATS = 'Chats';

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

/** A project in the side nav, with its threads. */
interface NavProject {
  /** The project's space, where a new thread from its + starts. */
  readonly id: string;
  readonly name: string;
  readonly threads: readonly NavThread[];
  readonly expanded: boolean;
}

/** What the side nav lists: the person's own chats, then each project with its threads. */
export interface NavSections {
  /** Every personal space's threads together, newest first. */
  readonly chats: readonly NavThread[];
  readonly projects: readonly NavProject[];
}

/** What the side nav shows: the search, the thread shown now and the projects closed. */
export interface NavView {
  readonly query: string;
  readonly current: string | null;
  readonly collapsed: ReadonlySet<string>;
}

/** A thread that needs the person, as the Needs you list shows it: its title and its place. */
interface WaitingThread {
  readonly id: string;
  readonly link: readonly string[];
  readonly title: string;
  /** Its project's name, or Chats. */
  readonly place: string;
}

/** Whether `title` has `query` in it, ignoring case and accents; an empty query matches all. */
export function matchesQuery(title: string, query: string): boolean {
  const wanted = fold(query.trim());
  return wanted === '' || fold(title).includes(wanted);
}

/**
 * The side nav's lists: the person's own chats (every personal space's threads together,
 * newest first), then each project with its threads. A search keeps only the threads whose
 * title matches, and the projects that have one, all shown open; without one, every project
 * shows, closed as the person left it.
 */
export function navSections(spaces: readonly SpaceWithThreads[], view: NavView): NavSections {
  const searching = view.query.trim() !== '';
  const lines = (threads: readonly ThreadSummary[]): readonly NavThread[] =>
    threads
      .map((thread) => ({
        id: thread.id,
        link: ['/t', thread.id],
        title: threadTitle(thread),
        mark: threadMark(thread),
        current: thread.id === view.current,
      }))
      .filter((thread) => matchesQuery(thread.title, view.query));
  const projects = spaces
    .filter(({ space }) => space.kind === 'project')
    .map(({ space, threads }) => ({
      id: space.id,
      name: space.name,
      threads: lines(threads),
      expanded: searching || !view.collapsed.has(space.id),
    }))
    .filter((project) => !searching || project.threads.length > 0);
  return { chats: lines(newestFirst(personalThreads(spaces))), projects };
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
