import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  type ElementRef,
  Injector,
  LOCALE_ID,
  afterNextRender,
  computed,
  inject,
  linkedSignal,
  signal,
  viewChild,
} from '@angular/core';
import { RouterLink } from '@angular/router';
import type { ScheduleId } from '@glave/client';

import { scheduleLine } from '../../lib/repeating';
import { clock } from '../../services/clock';
import { ServerService } from '../../services/server.service';
import { TasksService } from '../../services/tasks.service';
import { READER_ZONE } from '../../services/zone.service';
import { ScheduleActionsComponent } from '../schedule-actions/schedule-actions.component';

/**
 * The Repeating list on the Tasks page, where schedules run: each schedule by its name, which
 * opens its chat, when it repeats in the server's words with its next and last run on its own
 * clocks, and Pause or Resume and Remove, each row saying in the server's words what it refused.
 * Shown once it has a schedule; after the last is removed it says nothing repeats, the keyboard
 * on its heading.
 */
@Component({
  selector: 'app-repeating-list',
  imports: [RouterLink, ScheduleActionsComponent],
  templateUrl: './repeating-list.component.html',
  styleUrl: './repeating-list.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class RepeatingListComponent {
  protected readonly copy = {
    heading: $localize`:@@repeatingList.heading:Repeating`,
    none: $localize`:@@repeatingList.none:Nothing repeats now.`,
  } as const;
  private readonly tasks = inject(TasksService);
  private readonly server = inject(ServerService);
  private readonly offered = this.server.schedules;
  private readonly locale = inject(LOCALE_ID);
  private readonly zone = inject(READER_ZONE);
  private readonly injector = inject(Injector);
  private readonly heading = viewChild<ElementRef<HTMLElement>>('heading');
  /** What the server refused each schedule's last change with, by schedule. */
  private readonly errors = signal<ReadonlyMap<ScheduleId, string>>(new Map());
  /** Now, moving on while schedules show, so "tomorrow" becomes "today". */
  private readonly now = clock(() => this.tasks.schedules().length > 0);

  protected readonly rows = computed(() => {
    const reader = { locale: this.locale, timeZone: this.zone };
    const [now, errors] = [this.now(), this.errors()];
    return this.tasks.schedules().map((schedule) => {
      const thread = schedule.thread ?? null;
      return {
        schedule,
        id: schedule.schedule,
        name: schedule.name,
        link: thread === null ? null : ['/t', thread],
        line: scheduleLine(schedule, reader, now),
        paused: schedule.paused,
        error: errors.get(schedule.schedule) ?? null,
      };
    });
  });
  /** Whether the list shows: where schedules run, once it has had one while the page shows. */
  protected readonly shown = linkedSignal<boolean, boolean>({
    source: () => this.offered() && this.rows().length > 0,
    computation: (has, previous) => has || (previous?.value ?? false),
  });
  protected readonly empty = computed(() => this.rows().length === 0);

  constructor() {
    void this.server.load();
    const unwatch = this.tasks.watch();
    inject(DestroyRef).onDestroy(unwatch);
  }

  /** Keeps what the server refused `schedule`'s change with, or forgets it (`null`). */
  protected failed(schedule: ScheduleId, error: string | null): void {
    this.errors.update((all) => {
      const next = new Map(all);
      if (error === null) {
        next.delete(schedule);
      } else {
        next.set(schedule, error);
      }
      return next;
    });
  }

  /** A row went: the keyboard goes to the list's heading, which stays. */
  protected removed(): void {
    afterNextRender(() => this.heading()?.nativeElement.focus(), { injector: this.injector });
  }
}
