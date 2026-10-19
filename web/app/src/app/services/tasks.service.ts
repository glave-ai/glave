import { DOCUMENT } from '@angular/common';
import { DestroyRef, Service, computed, inject, signal } from '@angular/core';
import type { TaskInfo, ThreadId } from '@glave/client';

import { SessionService } from './session.service';

/** How often a list someone is looking at is read again, in milliseconds. */
export const TASKS_REFRESH_MS = 5_000;

/**
 * The tasks of the server's own project: running here first, then elsewhere, then finished; and
 * whether a schedule will start more. The one source for the Tasks page, the thread's tools
 * panel and the side nav's Tasks row; while any of them shows, they are read again every few
 * seconds, so cost and time stay current.
 */
@Service()
export class TasksService {
  private readonly session = inject(SessionService);
  private readonly document = inject(DOCUMENT);
  private watchers = 0;
  private timer: ReturnType<typeof setInterval> | null = null;
  private readonly list = signal<readonly TaskInfo[]>([]);
  private readonly failed = signal<string | null>(null);
  private readonly read = signal(false);
  /** How many schedules have a run still to come. */
  private readonly upcoming = signal(0);

  readonly tasks = this.list.asReadonly();
  readonly error = this.failed.asReadonly();
  readonly loaded = this.read.asReadonly();
  readonly running = computed(() => this.list().filter((task) => task.status === 'running').length);
  /** How many tasks are still going, here or on another server. */
  readonly going = computed(() => this.list().filter((task) => task.status !== 'finished').length);
  /** Whether tasks mean something now: one is going, or a schedule will start one. */
  readonly live = computed(() => this.going() > 0 || this.upcoming() > 0);

  constructor() {
    inject(DestroyRef).onDestroy(() => {
      this.stopTimer();
    });
  }

  /**
   * Reads the tasks now and again every few seconds (skipping a hidden tab) until every view
   * that asked has called the function this returns.
   */
  watch(): () => void {
    this.watchers += 1;
    if (this.timer === null) {
      void this.refresh();
      this.timer = setInterval(() => {
        if (!this.document.hidden) {
          void this.refresh();
        }
      }, TASKS_REFRESH_MS);
    }
    let watching = true;
    return () => {
      if (watching) {
        watching = false;
        this.watchers -= 1;
        if (this.watchers === 0) {
          this.stopTimer();
        }
      }
    };
  }

  async refresh(): Promise<void> {
    try {
      const [{ tasks }, { schedules }] = await Promise.all([
        this.session.call('tasks/list', { limit: 50 }),
        this.session.call('schedule/list', {}),
      ]);
      this.list.set(tasks);
      this.upcoming.set(schedules.filter((schedule) => (schedule.next ?? null) !== null).length);
      this.failed.set(null);
    } catch (error: unknown) {
      this.failed.set(error instanceof Error ? error.message : String(error));
    } finally {
      this.read.set(true);
    }
  }

  /** Stops the task working in `thread`, then reads the list again. */
  async stop(thread: ThreadId): Promise<void> {
    try {
      await this.session.call('tasks/stop', { thread });
    } catch (error: unknown) {
      this.failed.set(error instanceof Error ? error.message : String(error));
    }
    await this.refresh();
  }

  private stopTimer(): void {
    if (this.timer !== null) {
      clearInterval(this.timer);
      this.timer = null;
    }
  }
}
