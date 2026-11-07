import { computed, inject, signal } from '@angular/core';
import type { RawEvent, ThreadId } from '@glave/client';

import { lastCommandChange } from '../lib/commands';
import { mergeEvents } from '../lib/conversation';
import { lastChanging } from '../lib/files';
import { lastPlanning } from '../lib/plans';
import { lastSourcing } from '../lib/sources';
import { SessionService } from './session.service';

/**
 * One open thread's events as a view holds them, in order, each once: its latest turns as it
 * opens, what came before them as the person goes back to it, read a page of turns at a time,
 * and what is recorded after. The server says whether anything came before; the view only asks.
 */
export class History {
  private readonly session = inject(SessionService);
  private thread: ThreadId | null = null;
  private readonly held = signal<readonly RawEvent[]>([]);
  /** The `before` that reads the turns before those held, while there are any. */
  private readonly before = signal<number | null>(null);
  private readonly back = signal(false);
  /**
   * Where the turns held began as the thread opened, when turns came before them: what they did
   * may be in what the server says of the thread, but not in the events held.
   */
  private readonly from = signal<number | null>(null);
  /** Whether the thread's latest turns were read, or there is no thread to open. */
  private readonly opened = signal(true);

  readonly events = this.held.asReadonly();
  /** Whether the thread is being opened, its latest turns read. */
  readonly opening = computed(() => !this.opened());
  /** Whether the turns before those held are being read. */
  readonly readingBack = this.back.asReadonly();
  /** The `seq` of the latest end of a call that can change files; `null` while it opens. */
  readonly changing = computed(() => this.since(lastChanging));
  /** The `seq` of the latest start or end of a command in the background it recorded, if any. */
  readonly commandChange = computed(() => lastCommandChange(this.held()) ?? this.from());
  /**
   * The `seq` of the latest end of a turn once the thread may have read a page (it called
   * `web_fetch`), if it did; `null` while the thread opens, until its latest turns are read.
   */
  readonly sourcing = computed(() => this.since(lastSourcing));
  /** The `seq` of the latest change to the thread's plan, if any; `null` while the thread opens. */
  readonly planning = computed(() => this.since(lastPlanning));

  /** Holds `thread`'s events from now on: none yet. */
  reset(thread: ThreadId): void {
    this.thread = thread;
    this.held.set([]);
    this.before.set(null);
    this.back.set(false);
    this.from.set(null);
    this.opened.set(false);
  }

  /** Reads `thread`'s latest turns, as it opens: the events read. */
  async latest(thread: ThreadId): Promise<readonly RawEvent[]> {
    try {
      const page = await this.session.call('thread/turns', { thread });
      if (this.thread !== thread) {
        return [];
      }
      this.keep(page.events);
      this.before.set(page.earlier ?? null);
      this.from.set(page.earlier ?? null);
      return page.events;
    } finally {
      if (this.thread === thread) {
        this.opened.set(true);
      }
    }
  }

  /** Reads the page of turns before those held, unless one is being read or there are none. */
  async earlier(): Promise<void> {
    const [thread, before] = [this.thread, this.before()];
    if (thread === null || before === null || this.back()) {
      return;
    }
    this.back.set(true);
    try {
      const page = await this.session.call('thread/turns', { thread, before });
      if (this.thread === thread) {
        this.keep(page.events);
        this.before.set(page.earlier ?? null);
      }
    } finally {
      if (this.thread === thread) {
        this.back.set(false);
      }
    }
  }

  /** Reads `thread`'s events after `from`, page by page, each kept as it comes: those read. */
  async after(thread: ThreadId, from?: number): Promise<readonly RawEvent[]> {
    const read: RawEvent[] = [];
    let after = from;
    for (;;) {
      const page = await this.session.call('thread/events', {
        thread,
        ...(after === undefined ? {} : { after }),
      });
      if (this.thread !== thread) {
        return read;
      }
      this.keep(page.events);
      read.push(...page.events);
      if (page.next === undefined || page.next === null) {
        return read;
      }
      after = page.next;
    }
  }

  /** Adds `event`, unless it is held already, as a turn's stream and a watch may both send it. */
  add(event: RawEvent): boolean {
    const known = this.held();
    const newest = known.at(-1)?.seq ?? 0;
    if (event.seq <= newest && known.some((each) => each.seq === event.seq)) {
      return false;
    }
    this.keep([event]);
    return true;
  }

  /**
   * What a fact of the thread is read again on, once it has opened: the `seq` of the latest of
   * `last`'s events held, else, when turns came before those it opened with, where those began.
   */
  private since(last: (events: readonly RawEvent[]) => number | null): number | null {
    return this.opened() ? (last(this.held()) ?? this.from()) : null;
  }

  private keep(events: readonly RawEvent[]): void {
    this.held.update((known) => mergeEvents(known, events));
  }
}
