import { DOCUMENT } from '@angular/common';
import { Service, inject, signal } from '@angular/core';
import type { ScheduleId, ScheduleInfo } from '@glave/client';

import { SessionService } from './session.service';
import { TASKS_REFRESH_MS } from './tasks.service';

/**
 * The person's schedules, as the server lists them for the Repeating list, read while it shows;
 * and paused, resumed or removed from the app, each kept as the server answered it, so a read a
 * change may have overtaken leaves them as they are, and of two reads the latest wins. Loaded
 * only with what shows a schedule, never with the app's first page.
 */
@Service()
export class SchedulesService {
  private readonly session = inject(SessionService);
  private readonly document = inject(DOCUMENT);
  /** How many reads were asked for: only the latest one's answer is kept. */
  private reads = 0;
  /** How many changes were answered, so a read knows when one overtook it. */
  private changes = 0;
  /** How many changes are on their way. */
  private changing = 0;
  private readonly kept = signal<readonly ScheduleInfo[] | null>(null);

  /** The person's schedules, as the server lists them; `null` until read. */
  readonly schedules = this.kept.asReadonly();

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

  /** Reads the schedules, kept unless a later read or a change overtook this one. */
  async refresh(): Promise<void> {
    this.reads += 1;
    const [read, changes, before] = [this.reads, this.changes, this.changing];
    try {
      const { schedules } = await this.session.call('schedule/list', {});
      const overtaken = before > 0 || this.changing > 0 || changes !== this.changes;
      if (read === this.reads && !overtaken) {
        this.kept.set(schedules);
      }
    } catch {
      // What was read before stays, until the next read.
    }
  }

  /** Pauses `schedule` (`paused`) or resumes it; throws what the server refused it with. */
  async pause(schedule: ScheduleId, paused: boolean): Promise<ScheduleInfo> {
    const { schedule: now } = await this.change(() =>
      this.session.call('schedule/pause', { schedule, paused }),
    );
    this.kept.update(
      (all) => all?.map((each) => (each.schedule === now.schedule ? now : each)) ?? null,
    );
    return now;
  }

  /** Removes `schedule`, keeping its chat; throws what the server refused it with. */
  async remove(schedule: ScheduleId): Promise<void> {
    await this.change(() => this.session.call('schedule/delete', { schedule }));
    this.kept.update((all) => all?.filter((each) => each.schedule !== schedule) ?? null);
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
}
