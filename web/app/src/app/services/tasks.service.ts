import { DOCUMENT } from '@angular/common';
import { DestroyRef, Service, computed, inject, signal } from '@angular/core';
import type { ScheduleId, ScheduleInfo, TaskInfo, ThreadId } from '@glave/client';

import { SessionService } from './session.service';

/** How often a list someone is looking at is read again, in milliseconds. */
export const TASKS_REFRESH_MS = 5_000;

/**
 * The tasks of the server's own project: running here first, then elsewhere, then finished; and
 * the schedules that start more, each with a chat of its own, paused, resumed and removed here.
 * The one source for the Tasks page, the thread's tools panel and the side nav's Tasks row;
 * while any of them shows, they are read again every few seconds, so cost and time stay
 * current. A schedule changed here is kept as the server answered the change, and a read that
 * a change may have overtaken leaves the schedules as they are.
 */
@Service()
export class TasksService {
  private readonly session = inject(SessionService);
  private readonly document = inject(DOCUMENT);
  private watchers = 0;
  private timer: ReturnType<typeof setInterval> | null = null;
  /** How many reads were asked for: only the latest one's answer is kept. */
  private reads = 0;
  /** How many changes to schedules were asked for and answered, so a read knows it was overtaken. */
  private changes = 0;
  /** How many changes to schedules are on their way. */
  private changing = 0;
  private readonly list = signal<readonly TaskInfo[]>([]);
  private readonly kept = signal<readonly ScheduleInfo[]>([]);
  private readonly failed = signal<string | null>(null);
  private readonly read = signal(false);

  readonly tasks = this.list.asReadonly();
  /** The person's schedules, as the server lists them. */
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
    this.reads += 1;
    const [read, changes] = [this.reads, this.changes];
    const before = this.changing;
    try {
      const [{ tasks }, { schedules }] = await Promise.all([
        this.session.call('tasks/list', { limit: 50 }),
        this.session.call('schedule/list', {}),
      ]);
      if (read !== this.reads) {
        return;
      }
      this.list.set(tasks);
      // A change asked for or answered meanwhile may be newer than what this read says.
      if (before === 0 && this.changing === 0 && changes === this.changes) {
        this.kept.set(schedules);
      }
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

  /**
   * Pauses `schedule` (`paused`) or resumes it, and keeps it as the server answers; throws what
   * the server refused it with.
   */
  async pause(schedule: ScheduleId, paused: boolean): Promise<ScheduleInfo> {
    const { schedule: now } = await this.change(() =>
      this.session.call('schedule/pause', { schedule, paused }),
    );
    this.kept.update((all) => all.map((each) => (each.schedule === now.schedule ? now : each)));
    return now;
  }

  /** Removes `schedule`, keeping its chat; throws what the server refused it with. */
  async remove(schedule: ScheduleId): Promise<void> {
    await this.change(() => this.session.call('schedule/delete', { schedule }));
    this.kept.update((all) => all.filter((each) => each.schedule !== schedule));
  }

  /** Asks the server for `change`, which a read asked for meanwhile may not have seen. */
  private async change<T>(change: () => Promise<T>): Promise<T> {
    this.changing += 1;
    try {
      return await change();
    } finally {
      this.changing -= 1;
      this.changes += 1;
    }
  }

  private stopTimer(): void {
    if (this.timer !== null) {
      clearInterval(this.timer);
      this.timer = null;
    }
  }
}
