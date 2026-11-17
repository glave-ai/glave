import { DOCUMENT } from '@angular/common';
import { Service, inject, signal } from '@angular/core';
import type { ScheduleId, ScheduleInfo } from '@glave/client';

import { SessionService } from './session.service';
import { TASKS_REFRESH_MS } from './tasks.service';

/**
 * The person's schedules, as the server lists them for the Repeating list, read while it shows;
 * and paused, resumed or removed from the app, each kept as the server answered it, then read
 * again. Of two reads the latest wins, so one asked before a change never undoes it. Loaded only
 * with what shows a schedule, never with the app's first page.
 */
@Service()
export class SchedulesService {
  private readonly session = inject(SessionService);
  private readonly document = inject(DOCUMENT);
  /** How many reads were asked for: only the latest one's answer is kept. */
  private reads = 0;
  private readonly kept = signal<readonly ScheduleInfo[] | null>(null);
  private readonly done = signal(false);

  /** The person's schedules, as the server lists them; `null` until read. */
  readonly schedules = this.kept.asReadonly();
  /** Whether they were read once, or could not be. */
  readonly read = this.done.asReadonly();

  /**
   * Reads the schedules now and again every few seconds, skipping a hidden tab, until the
   * function this returns is called.
   */
  watch(): () => void {
    void this.refresh();
    const timer = setInterval(() => {
      if (!this.document.hidden) {
        void this.refresh();
      }
    }, TASKS_REFRESH_MS);
    return () => {
      clearInterval(timer);
    };
  }

  /** Reads the schedules, kept unless a later read was asked for meanwhile. */
  async refresh(): Promise<void> {
    this.reads += 1;
    const read = this.reads;
    try {
      const { schedules } = await this.session.call('schedule/list', {});
      if (read === this.reads) {
        this.kept.set(schedules);
      }
    } catch {
      // What was read before stays, until the next read.
    } finally {
      this.done.set(true);
    }
  }

  /** Pauses `schedule` (`paused`) or resumes it; throws what the server refused it with. */
  async pause(schedule: ScheduleId, paused: boolean): Promise<ScheduleInfo> {
    const answer = await this.session.call('schedule/pause', { schedule, paused });
    const now = answer.schedule;
    this.changed((all) => all.map((each) => (each.schedule === now.schedule ? now : each)));
    return now;
  }

  /** Removes `schedule`, keeping its chat; throws what the server refused it with. */
  async remove(schedule: ScheduleId): Promise<void> {
    await this.session.call('schedule/delete', { schedule });
    this.changed((all) => all.filter((each) => each.schedule !== schedule));
  }

  /** Keeps a change as the server answered it, then reads the list again, past any read asked before. */
  private changed(change: (all: readonly ScheduleInfo[]) => readonly ScheduleInfo[]): void {
    this.kept.update((all) => (all === null ? null : change(all)));
    void this.refresh();
  }
}
