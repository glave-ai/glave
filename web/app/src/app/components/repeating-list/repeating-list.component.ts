import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  LOCALE_ID,
  computed,
  inject,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';
import type { ScheduleId } from '@glave/client';
import { GlzButtonComponent, GlzCardComponent, GlzConfirmDialogComponent } from '@glave/glaze';

import { repeatLine } from '../../lib/repeating';
import { TasksService } from '../../services/tasks.service';

/**
 * The Repeating list on the Tasks page: each schedule by its name, which opens its chat, when it
 * repeats in the server's words with its next and last run, and Pause or Resume and Remove.
 * Remove asks once, and keeps the chat with its past runs. With no schedules it shows nothing.
 */
@Component({
  selector: 'app-repeating-list',
  imports: [RouterLink, GlzButtonComponent, GlzCardComponent, GlzConfirmDialogComponent],
  templateUrl: './repeating-list.component.html',
  styleUrl: './repeating-list.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class RepeatingListComponent {
  protected readonly copy = {
    heading: $localize`:@@repeatingList.heading:Repeating`,
    pause: $localize`:@@repeatingList.pause:Pause`,
    resume: $localize`:@@repeatingList.resume:Resume`,
    remove: $localize`:@@repeatingList.remove:Remove`,
    removeHeading: $localize`:@@repeatingList.removeHeading:Stop repeating this?`,
    removeText: $localize`:@@repeatingList.removeText:No more runs start. Its chat stays, with its past runs.`,
  } as const;
  private readonly service = inject(TasksService);
  private readonly locale = inject(LOCALE_ID);
  /** The schedule Remove was pressed for, while the person decides. */
  private readonly removing = signal<ScheduleId | null>(null);
  protected readonly confirming = signal(false);

  protected readonly rows = computed(() =>
    this.service.schedules().map((schedule) => {
      const name = schedule.name;
      const thread = schedule.thread ?? null;
      return {
        schedule: schedule.schedule,
        name,
        link: thread === null ? null : ['/t', thread],
        line: repeatLine(schedule, this.locale),
        paused: schedule.paused,
        pauseLabel: $localize`:@@repeatingList.pauseNamed:Pause ${name}:NAME:`,
        resumeLabel: $localize`:@@repeatingList.resumeNamed:Resume ${name}:NAME:`,
        removeLabel: $localize`:@@repeatingList.removeNamed:Remove ${name}:NAME:`,
      };
    }),
  );
  protected readonly any = computed(() => this.rows().length > 0);

  constructor() {
    const unwatch = this.service.watch();
    inject(DestroyRef).onDestroy(unwatch);
  }

  protected pause(schedule: ScheduleId): void {
    void this.service.pause(schedule, true);
  }

  protected resume(schedule: ScheduleId): void {
    void this.service.pause(schedule, false);
  }

  /** Asks before removing `schedule`. */
  protected ask(schedule: ScheduleId): void {
    this.removing.set(schedule);
    this.confirming.set(true);
  }

  /** Removes the schedule asked about, when the person said yes. */
  protected decided(yes: boolean): void {
    const schedule = this.removing();
    this.removing.set(null);
    if (yes && schedule !== null) {
      void this.service.remove(schedule);
    }
  }
}
