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
import type { ScheduleId, ScheduleInfo } from '@glave/client';

import { scheduleParts } from '../../lib/repeating';
import { clock } from '../../services/clock';
import { ProjectsService } from '../../services/projects.service';
import { SchedulesService } from '../../services/schedules.service';
import { READER_ZONE } from '../../services/zone.service';
import { ScheduleActionsComponent } from '../schedule-actions/schedule-actions.component';

/** `schedules` in one order whatever told them: oldest first. */
function inOrder(schedules: readonly ScheduleInfo[]): readonly ScheduleInfo[] {
  return [...schedules].sort(
    (a, b) => a.created_at.localeCompare(b.created_at) || a.schedule.localeCompare(b.schedule),
  );
}

/**
 * The Repeating list on the Tasks page: each schedule by its name, which opens its chat, when it
 * repeats in the server's words with its next and last run on its own clocks (a line each on a
 * phone), and Pause or Resume and Remove, each row saying in the server's words what it
 * refused. It opens with the schedules the side nav's chats carry, so it takes its room at
 * once, until the server's list is read. Shown once it has a schedule; after the last is
 * removed it says nothing repeats, the keyboard on its heading.
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
  private readonly service = inject(SchedulesService);
  private readonly projects = inject(ProjectsService);
  private readonly locale = inject(LOCALE_ID);
  private readonly zone = inject(READER_ZONE);
  private readonly injector = inject(Injector);
  private readonly heading = viewChild<ElementRef<HTMLElement>>('heading');
  /** What the server refused each schedule's last change with, by schedule. */
  private readonly errors = signal<ReadonlyMap<ScheduleId, string>>(new Map());
  /** The schedules the side nav's chats carry, which the list shows until its own is read. */
  private readonly carried = computed(() =>
    this.projects
      .projects()
      .flatMap(({ threads }) => threads.flatMap((thread) => thread.schedule ?? [])),
  );
  private readonly schedules = computed(() => inOrder(this.service.schedules() ?? this.carried()));
  /** Now, moving on while schedules show, so "tomorrow" becomes "today". */
  private readonly now = clock(() => this.schedules().length > 0);

  protected readonly rows = computed(() => {
    const reader = { locale: this.locale, timeZone: this.zone };
    const [now, errors] = [this.now(), this.errors()];
    return this.schedules().map((schedule) => {
      const thread = schedule.thread ?? null;
      return {
        schedule,
        id: schedule.schedule,
        name: schedule.name,
        link: thread === null ? null : ['/t', thread],
        parts: scheduleParts(schedule, reader, now).map((text, at) => ({ text, after: at > 0 })),
        paused: schedule.paused,
        error: errors.get(schedule.schedule) ?? null,
      };
    });
  });
  /** Whether the list shows: once it has had a schedule while the page shows. */
  protected readonly shown = linkedSignal<boolean, boolean>({
    source: () => this.rows().length > 0,
    computation: (has, previous) => has || (previous?.value ?? false),
  });
  protected readonly empty = computed(() => this.rows().length === 0);

  constructor() {
    const unwatch = this.service.watch();
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
