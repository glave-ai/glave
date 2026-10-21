import type { WritableSignal } from '@angular/core';
import type { Notification } from '@glave/client';

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
  /** How many of each project's threads the lists hold. */
  readonly limit: number;
  /** Reads the lists again, or joins the read under way. */
  refresh(): Promise<void>;
  /** The read under way, if any. */
  reading(): Promise<void> | null;
}

/**
 * Follows the lists while the tab is shown, until `stop` aborts: watched from the newest update
 * they hold. A hidden tab lets go of its watch, as a thread's does, and reads the lists again and
 * watches them once shown; their times move on each minute it is shown. Answers what starts
 * following them again, the lists just read, as when the tab signs in again.
 */
export function followLists(host: ListsHost, stop: AbortSignal): () => void {
  const { document } = host;
  const hidden = (): boolean => document.visibilityState === 'hidden';
  let watching: AbortController | null = null;
  const watch = (read: boolean): void => {
    if (watching !== null || stop.aborted || hidden()) {
      return;
    }
    const controller = new AbortController();
    watching = controller;
    void follow(host, controller.signal, read).finally(() => {
      if (watching === controller) {
        watching = null;
      }
    });
  };
  const shown = (): void => {
    if (hidden()) {
      watching?.abort();
      watching = null;
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
      watching?.abort();
    },
    { once: true },
  );
  return () => {
    watch(true);
  };
}

/**
 * Watches the lists from the newest update they hold, once read (`read` says they just were):
 * each thread's summary as the server says it changes. Cut off, it reads them again and comes
 * back from there, waiting longer each time; it ends once `signal` aborts or the tab is signed
 * out.
 */
async function follow(host: ListsHost, signal: AbortSignal, read: boolean): Promise<void> {
  const stopped = (): boolean => signal.aborted;
  let [wait, fresh] = [FIRST_RETRY_MS, read];
  while (!stopped() && host.session.signedIn()) {
    await (fresh ? host.reading() : host.refresh());
    fresh = false;
    const began = Date.now();
    const after = newestUpdate(host.lists());
    try {
      const params = after === undefined ? {} : { after };
      const watch = host.session.stream('thread/list/watch', params, signal);
      for (let next = await watch.next(); !next.done; next = await watch.next()) {
        take(host, next.value);
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
 * Takes a thread's summary as the server says it is now, into the lists, and again once a read
 * under way has read them, since that read began before it. A new thread in a project the lists
 * lack, as one made elsewhere, reads them again.
 */
function take(host: ListsHost, note: Notification): void {
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
  const reading = !listed && summary.standing === 'active' ? host.refresh() : host.reading();
  void reading?.then(apply);
}
