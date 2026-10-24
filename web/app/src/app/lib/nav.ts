import type { Project, Status, ThreadSummary } from '@glave/client';
import type { GlzIconData } from '@glave/glaze';
import Hand from 'lucide/dist/esm/icons/hand.mjs';
import TriangleAlert from 'lucide/dist/esm/icons/triangle-alert.mjs';

import { threadTitle } from './threads';
import { type Reader, type Since, type Times, readerTimes } from './when';

/** The side nav's own lists, whose state is kept beside each project's (its project's id). */
export const PROJECTS_LIST = 'projects';
export const CHATS_LIST = 'chats';

/** A project with its threads, as the projects service reads them. */
export interface ProjectWithThreads {
  readonly project: Project;
  readonly threads: readonly ThreadSummary[];
  /** What the server says when the project's folder isn't there anymore. */
  readonly gone?: string;
}

/**
 * What says a closed list has something in it that needs the person: a small, neutral mark, and
 * what screen readers hear.
 */
export function needsYou(): { readonly icon: GlzIconData; readonly label: string } {
  return { icon: Hand, label: $localize`:@@nav.needsYou:needs you` };
}

/**
 * The side nav's words for its threads, however they are grouped: what names them for screen
 * readers, and what a search that finds none says.
 */
export function threadsWords(): { readonly threads: string; readonly noMatch: string } {
  return {
    threads: $localize`:@@nav.threads:Threads`,
    noMatch: $localize`:@@nav.noMatch:No threads match`,
  };
}

/**
 * What a thread's line in the side nav says under its title: that it needs the person, grouped
 * by project; or where it is, grouped by what it is doing.
 */
interface NavLine {
  readonly text: string;
  /** Whether it says the thread needs the person, which reads stronger than where it is. */
  readonly needsYou: boolean;
}

/** A thread's line in the side nav, with the threads it started under it. */
export interface NavThread {
  readonly id: string;
  readonly link: readonly string[];
  readonly title: string;
  /** Whether the server says it runs. */
  readonly running: boolean;
  /** The line under its title, if it has one. */
  readonly line: NavLine | null;
  /** When it was last active, which its line shows while it does not run. */
  readonly since: Since;
  readonly current: boolean;
  readonly children: readonly NavThread[];
  readonly hasChildren: boolean;
}

/** A mark at a heading's end, an icon with what screen readers hear. */
export interface HeadingMark {
  readonly icon: GlzIconData;
  readonly label: string;
  /** Whether it warns, in the warning tone: the project's folder is gone. */
  readonly warning: boolean;
}

/** A heading in the side nav, and how the list under it shows. */
interface NavHeading {
  readonly expanded: boolean;
  /** While closed with something in it waiting for the person, the mark that says so. */
  readonly mark: HeadingMark | null;
}

/** A project in the side nav, with its threads. */
export interface NavProject extends NavHeading {
  /** The project's project, where a new thread from its + starts. */
  readonly id: string;
  readonly name: string;
  readonly threads: readonly NavThread[];
  /** What the server says when its folder isn't there anymore, which its mark says too. */
  readonly gone: string | null;
}

/** What the side nav lists by project: each project with its threads, then the person's chats. */
export interface NavSections {
  readonly projects: NavHeading & { readonly list: readonly NavProject[] };
  /** Every personal project's threads together. */
  readonly chats: NavHeading & { readonly list: readonly NavThread[] };
}

/** The side nav's threads by what the server says each is doing: its `status`. */
type StatusSections = Readonly<Record<Status, readonly NavThread[]>>;

/**
 * What the side nav's threads show: the search, the thread shown now, and now, in milliseconds
 * since the epoch, for its reader (their own language and time zone unless set): their times
 * count from it.
 */
interface ThreadsView {
  readonly query: string;
  readonly current: string | null;
  readonly now: number;
  readonly reader?: Reader;
}

/**
 * What the side nav shows by project: its threads, and the lists the person closed
 * (`PROJECTS_LIST`, `CHATS_LIST` or a project's id).
 */
export interface NavView extends ThreadsView {
  readonly collapsed: ReadonlySet<string>;
}

/** An item and when it was last active, which its list is ordered by. */
interface Dated<T> {
  readonly item: T;
  readonly waiting: boolean;
  readonly latest: string;
}

/** The fewest characters a search needs before it looks at threads' days too. */
const DAY_QUERY = 3;

/**
 * Whether a thread with `title`, whose days are written `days` (`Times.day`), is found by
 * `query`, ignoring case and accents: its title has the query in it, or, for a query of a few
 * characters that is not a number alone, each of the query's words begins a word of one way of
 * writing one of its days ("oct 12", "12 oct", "yesterday", "10/12"). An empty query finds all.
 */
export function matchesQuery(title: string, days: readonly string[], query: string): boolean {
  const wanted = fold(query.trim());
  if (wanted === '' || fold(title).includes(wanted)) {
    return true;
  }
  if (wanted.length < DAY_QUERY || /^[\d\s]+$/u.test(wanted)) {
    return false;
  }
  const words = wordsOf(wanted);
  return days.some((day) => {
    const written = wordsOf(fold(day));
    return words.every((word) => written.some((each) => each.startsWith(word)));
  });
}

/** Whether the person has a project of their own beside their Chats. */
export function hasProjects(lists: readonly ProjectWithThreads[]): boolean {
  return lists.some(({ project }) => project.kind === 'project');
}

/**
 * The side nav's lists by project: each project with its threads, then the person's own chats
 * (every personal project's threads together). Each list is newest first, in one order the
 * person can learn, whatever waits or runs: a project by its latest thread (its making, before
 * it has one), a thread with the threads it started, which sit under it. A thread that needs the
 * person says so under its title. A search keeps only the threads it finds, by title or by day,
 * and the projects that have one, all shown open; without one, every project shows, each list
 * closed as the person left it.
 */
export function navSections(lists: readonly ProjectWithThreads[], view: NavView): NavSections {
  const times = readerTimes(view.now, view.reader);
  const found = (thread: ThreadSummary): boolean => isFound(thread, view, times);
  const projects = lists
    .filter(({ project }) => project.kind === 'project')
    .map(({ project, threads, gone }) =>
      datedProject(project, threads, view, times, found, gone ?? null),
    )
    .filter(({ item }) => !searching(view) || item.threads.length > 0);
  const chats = threadTree(personalThreads(lists), view, times, found);
  return {
    projects: {
      ...heading(
        PROJECTS_LIST,
        projects.some(({ waiting }) => waiting),
        view,
      ),
      list: newestFirst(projects).map(({ item }) => item),
    },
    chats: { ...heading(CHATS_LIST, chats.waiting, view), list: chats.list },
  };
}

/**
 * The side nav's threads by what the server says each is doing: those that need the person (an
 * approval, a question, or their project's trust), those running, then the rest, each newest
 * first, and each thread once, with its project's name, or Chats, under its title. A thread
 * another started is listed on its own, where what it is doing puts it. A search keeps only the
 * threads it finds.
 */
export function statusSections(
  lists: readonly ProjectWithThreads[],
  view: ThreadsView,
): StatusSections {
  const times = readerTimes(view.now, view.reader);
  // What the side nav calls the person's own chats (their personal projects' threads).
  const chats = $localize`:@@nav.chats:Chats`;
  const dated = lists.flatMap(({ project, threads }) => {
    const line = { text: project.kind === 'project' ? project.name : chats, needsYou: false };
    return threads
      .filter((thread) => isFound(thread, view, times))
      .map((thread) => ({
        status: thread.status,
        latest: thread.updated_at,
        item: { ...navThread(thread, view, times, []), line },
      }));
  });
  const doing = (status: Status): readonly NavThread[] =>
    newestFirst(dated.filter((each) => each.status === status)).map(({ item }) => item);
  return { waiting: doing('waiting'), running: doing('running'), idle: doing('idle') };
}

/** Whether the search finds `thread`, by its title or by the day it began or was last active. */
function isFound(thread: ThreadSummary, view: ThreadsView, times: Times): boolean {
  return (
    !searching(view) ||
    matchesQuery(
      threadTitle(thread),
      [...times.day(thread.created_at), ...times.day(thread.updated_at)],
      view.query,
    )
  );
}

function searching(view: ThreadsView): boolean {
  return view.query.trim() !== '';
}

/** How the list `list` shows: open while searching, else as the person left it. */
function heading(list: string, waiting: boolean, view: NavView): NavHeading {
  const expanded = searching(view) || !view.collapsed.has(list);
  return { expanded, mark: !expanded && waiting ? { ...needsYou(), warning: false } : null };
}

function datedProject(
  project: Project,
  threads: readonly ThreadSummary[],
  view: NavView,
  times: Times,
  found: (thread: ThreadSummary) => boolean,
  gone: string | null,
): Dated<NavProject> {
  const tree = threadTree(threads, view, times, found);
  const shown = heading(project.id, tree.waiting, view);
  return {
    item: {
      ...shown,
      // A folder that went is marked open or closed: nothing runs there until it is found.
      mark: gone === null ? shown.mark : { icon: TriangleAlert, label: gone, warning: true },
      id: project.id,
      name: project.name,
      threads: tree.list,
      gone,
    },
    waiting: tree.waiting,
    latest: [project.created_at, ...threads.map((thread) => thread.updated_at)].reduce(later),
  };
}

/** A list of threads as the side nav shows it, and whether any of them waits. */
interface ThreadTree {
  readonly list: readonly NavThread[];
  readonly waiting: boolean;
}

/**
 * The threads the search finds, each under the thread that started it when that one is shown
 * too, every level newest first, and under the title of each that waits, that it waits for the
 * person. A thread waits, and was last active, with the threads under it.
 */
function threadTree(
  threads: readonly ThreadSummary[],
  view: NavView,
  times: Times,
  found: (thread: ThreadSummary) => boolean,
): ThreadTree {
  const shown = threads.filter(found);
  const ids = new Set(shown.map((thread) => thread.id));
  const parentOf = (thread: ThreadSummary): string | null =>
    typeof thread.parent === 'string' && ids.has(thread.parent) ? thread.parent : null;
  const waits = { text: $localize`:@@nav.waitingOnYou:Waiting on you`, needsYou: true };
  const level = (parent: string | null): readonly Dated<NavThread>[] =>
    newestFirst(
      shown
        .filter((thread) => parentOf(thread) === parent)
        .map((thread) => {
          const children = level(thread.id);
          const item = navThread(
            thread,
            view,
            times,
            children.map((child) => child.item),
          );
          const waiting = thread.status === 'waiting';
          return {
            item: { ...item, line: waiting ? waits : null },
            waiting: waiting || children.some((child) => child.waiting),
            latest: [thread.updated_at, ...children.map(({ latest }) => latest)].reduce(later),
          };
        }),
    );
  const roots = level(null);
  return { list: roots.map(({ item }) => item), waiting: roots.some(({ waiting }) => waiting) };
}

/** `thread`'s line in the side nav, with `children` under it, and nothing under its title. */
function navThread(
  thread: ThreadSummary,
  view: ThreadsView,
  times: Times,
  children: readonly NavThread[],
): NavThread {
  return {
    id: thread.id,
    link: ['/t', thread.id],
    title: threadTitle(thread),
    running: thread.status === 'running',
    line: null,
    since: times.since(thread.updated_at),
    current: thread.id === view.current,
    children,
    hasChildren: children.length > 0,
  };
}

/** `items` with the latest active first. */
function newestFirst<T extends { readonly latest: string }>(items: readonly T[]): readonly T[] {
  return [...items].sort((a, b) => b.latest.localeCompare(a.latest));
}

/** The later of two of the server's times: each has one spelling, so their text sorts. */
function later(a: string, b: string): string {
  return a.localeCompare(b) >= 0 ? a : b;
}

function personalThreads(projects: readonly ProjectWithThreads[]): readonly ThreadSummary[] {
  return projects
    .filter(({ project }) => project.kind === 'personal')
    .flatMap(({ threads }) => threads);
}

function wordsOf(text: string): readonly string[] {
  return text.split(/[\s,]+/u).filter((word) => word !== '');
}

function fold(text: string): string {
  return text
    .normalize('NFD')
    .replace(/\p{Diacritic}/gu, '')
    .toLowerCase();
}
