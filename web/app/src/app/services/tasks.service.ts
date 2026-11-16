import { DOCUMENT } from '@angular/common';
import { DestroyRef, Service, computed, inject, signal } from '@angular/core';
import type { ScheduleId, ScheduleInfo, TaskInfo, ThreadId } from '@glave/client';

import { SessionService } from './session.service';

/** How often a list someone is looking at is read again, in milliseconds. */
export const TASKS_REFRESH_MS = 5_000;

/**
 * The tasks of the server's own project: running here first, then elsewhere, then finished; and
 * the schedules that start more, each with a chat of its own. The one source for the Tasks
 * page, a schedule's chat, the thread's tools panel and the side nav's Tasks row; while any of
 * them shows, they are read again every few seconds, so cost and time stay current. As the
 * first of them shows, the server is given this browser's time zone, which the person's
 * schedules repeat in.
 */
@Service()
export class TasksService {
  private readonly session = inject(SessionService);
  private readonly document = inject(DOCUMENT);
  private watchers = 0;
  private timer: ReturnType<typeof setInterval> | null = null;
  /** Whether the server was given this browser's time zone. */
  private zoned = false;
  private readonly list = signal<readonly TaskInfo[]>([]);
  private readonly kept = signal<readonly ScheduleInfo[]>([]);
  private readonly failed = signal<string | null>(null);
  private readonly read = signal(false);

  readonly tasks = this.list.asReadonly();
  /** The person's schedules, oldest first. */
  readonly schedules = this.kept.asReadonly();
  readonly error = this.failed.asReadonly();
  readonly loaded = this.read.asReadonly();
  readonly running = computed(() => this.list().filter((task) => task.status === 'running').length);
  /** How many tasks are still going, here or on another server. */
  readonly going = computed(() => this.list().filter((task) => task.status !== 'finished').length);
  /** How many schedules have a run still to come. */
  private readonly upcoming = computed(
    () => this.kept().filter((schedule) => (schedule.next ?? null) !== null).length,
  );
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
      void this.giveZone();
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
      this.kept.set(schedules);
      this.failed.set(null);
    } catch (error: unknown) {
      this.failed.set(error instanceof Error ? error.message : String(error));
    } finally {
      this.read.set(true);
    }
  }

  /** Stops the task working in `thread`, then reads the list again. */
  async stop(thread: ThreadId): Promise<void> {
    await this.change(() => this.session.call('tasks/stop', { thread }));
  }

  /** Pauses `schedule` (`paused`) or resumes it, then reads the lists again. */
  async pause(schedule: ScheduleId, paused: boolean): Promise<void> {
    await this.change(() => this.session.call('schedule/pause', { schedule, paused }));
  }

  /** Removes `schedule`, keeping its chat, then reads the lists again. */
  async remove(schedule: ScheduleId): Promise<void> {
    await this.change(() => this.session.call('schedule/delete', { schedule }));
  }

  /** Asks the server for `change`, then reads the lists again; what it refused is said. */
  private async change(change: () => Promise<unknown>): Promise<void> {
    try {
      await change();
    } catch (error: unknown) {
      this.failed.set(error instanceof Error ? error.message : String(error));
    }
    await this.refresh();
  }

  /** Gives the server this browser's time zone, once; unread, schedules keep the last given. */
  private async giveZone(): Promise<void> {
    if (this.zoned) {
      return;
    }
    this.zoned = true;
    const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    try {
      await this.session.call('zone/set', { zone });
    } catch {
      // The person's schedules repeat in the zone the server last had.
    }
  }

  private stopTimer(): void {
    if (this.timer !== null) {
      clearInterval(this.timer);
      this.timer = null;
    }
  }
}
