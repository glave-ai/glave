import type { Signal, WritableSignal } from '@angular/core';
import type { Notification, Setting } from '@glave/client';

import { newestUpdate, withSummary } from '../lib/lists';
import type { ProjectWithThreads } from '../lib/nav';
import { FIRST_RETRY_MS, LAST_RETRY_MS, cutOff, pause } from '../lib/watching';
import type { SessionService } from './session.service';

/** How often the lists' times move on while the tab is shown, in milliseconds. */
export const CLOCK_MS = 60_000;

/**
 * What following the lists needs of the projects service, which loads it once the app asks to
 * follow them, so the first page carries none of it.
 */
export interface ListsHost {
  readonly session: SessionService;
  readonly document: Document;
  readonly lists: WritableSignal<readonly ProjectWithThreads[]>;
  /** What the lists' times count from, moved on with each change and each minute. */
  readonly read: WritableSignal<number>;
  /** Why the last read failed, if it did. */
  readonly error: Signal<string | null>;
  /** How many of each project's threads the lists hold. */
  readonly limit: number;
  /** Reads the lists again, or joins the read under way. */
  refresh(): Promise<void>;
  /** Reads the lists again after the read under way, which began before what calls for it. */
  reread(): Promise<void>;
  /** The read under way, if any. */
  reading(): Promise<void> | null;
  /** Takes the server's word that the person's `setting` changed. */
  heard(setting: Setting): void;
}

/** Following the lists, once started. */
export interface Following {
  /** Follows them again, the lists just read, as when the tab signs in again. */
  again(): void;
  /** Lets go of the watch, as when the tab signs out. */
  letGo(): void;
}

/**
 * Follows the lists while the tab is shown, until `stop` aborts: watched from the newest update
 * they hold. A hidden tab lets go of its watch, as a thread's does, and reads the lists again and
 * watches them once shown; their times move on each minute it is shown.
 */
export function followLists(host: ListsHost, stop: AbortSignal): Following {
  const { document } = host;
  const hidden = (): boolean => document.visibilityState === 'hidden';
  // The projects a summary named that the lists lacked, read again for once each.
  const lacked = new Set<string>();
  let watching: AbortController | null = null;
  const letGo = (): void => {
    watching?.abort();
    watching = null;
  };
  const watch = (read: boolean): void => {
    if (watching !== null || stop.aborted || hidden()) {
      return;
    }
    const controller = new AbortController();
    watching = controller;
    void watchLists(host, controller.signal, read, lacked).finally(() => {
      if (watching === controller) {
        watching = null;
      }
    });
  };
  if (stop.aborted) {
    return { again: letGo, letGo };
  }
  const shown = (): void => {
    if (hidden()) {
      letGo();
    } else {
      watch(false);
    }
  };
  const clock = setInterval(() => {
    if (!hidden()) {
      host.read.set(Date.now());
    }
  }, CLOCK_MS);
  document.addEventListener('visibilitychange', shown);
  stop.addEventListener(
    'abort',
    () => {
      document.removeEventListener('visibilitychange', shown);
      clearInterval(clock);
      letGo();
    },
    { once: true },
  );
  return {
    again: () => {
      watch(true);
    },
    letGo,
  };
}

/**
 * Watches the lists from the newest update they hold, once read (`read` says they just were):
 * each thread's summary as the server says it changes. With nothing listed, from when they were
 * read. Cut off, it reads them again and comes back from there, waiting longer each time; it
 * ends once `signal` aborts or the tab is signed out.
 */
async function watchLists(
  host: ListsHost,
  signal: AbortSignal,
  read: boolean,
  lacked: Set<string>,
): Promise<void> {
  const stopped = (): boolean => signal.aborted;
  let wait = FIRST_RETRY_MS;
  let fresh = read;
  while (!stopped() && host.session.signedIn()) {
    const readAt = new Date().toISOString();
    if (fresh) {
      await host.reading();
    }
    // A first read that failed is tried again, as any other.
    if (!fresh || host.error() !== null) {
      await host.refresh();
    }
    fresh = false;
    const began = Date.now();
    const after = newestUpdate(host.lists()) ?? readAt;
    try {
      const watch = host.session.stream('thread/list/watch', { after }, signal);
      for (let next = await watch.next(); !next.done; next = await watch.next()) {
        take(host, next.value, lacked);
        wait = FIRST_RETRY_MS;
      }
    } catch (error: unknown) {
      if (stopped() || !cutOff(error)) {
        return;
      }
    }
    // A watch that lasted was not failing: the next waits only a little.
    if (Date.now() - began > 2 * wait) {
      wait = FIRST_RETRY_MS;
    }
    await pause(wait, signal);
    wait = Math.min(wait * 2, LAST_RETRY_MS);
  }
}

/**
 * Takes what the person's own setting the server says changed to whoever shows it; and a
 * thread's summary as the server says it is now, into the lists, and again once a read
 * under way has read them, since that read began before it. A new thread in a project the lists
 * lack, as one made elsewhere, reads them again, once for each such project.
 */
function take(host: ListsHost, note: Notification, lacked: Set<string>): void {
  if (note.method === 'settings/changed') {
    host.heard(note.params.setting);
    return;
  }
  if (note.method !== 'thread/changed') {
    return;
  }
  const { summary } = note.params;
  const apply = (): boolean => {
    const lists = withSummary(host.lists(), summary, host.limit);
    if (lists !== null) {
      host.lists.set(lists);
      host.read.set(Date.now());
    }
    return lists !== null;
  };
  const listed = apply();
  const unknown = !listed && summary.standing === 'active' && !lacked.has(summary.project);
  if (listed) {
    // Listed now, it may be lacked again later, as once removed and given back.
    lacked.delete(summary.project);
  } else if (unknown) {
    lacked.add(summary.project);
  }
  void (unknown ? host.reread() : host.reading())?.then(apply);
}
