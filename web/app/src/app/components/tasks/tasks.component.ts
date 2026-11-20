import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { GlzButtonComponent } from '@glave/glaze';

import { SchedulesService } from '../../services/schedules.service';
import { TasksService } from '../../services/tasks.service';
import { PageHeaderComponent } from '../page-header/page-header.component';
import { RepeatingListComponent } from '../repeating-list/repeating-list.component';
import { TaskListComponent } from '../task-list/task-list.component';

/**
 * The Tasks page: what repeats, the tasks running now, and those finished, these once the
 * schedules are read, so it opens at its shape.
 */
@Component({
  selector: 'app-tasks',
  imports: [GlzButtonComponent, PageHeaderComponent, RepeatingListComponent, TaskListComponent],
  templateUrl: './tasks.component.html',
  styleUrl: './tasks.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TasksComponent {
  protected readonly copy = {
    heading: $localize`:@@tasks.heading:Tasks`,
    intro: $localize`:@@tasks.intro:Work running with no one attached: started in the background, or by a schedule.`,
    refresh: $localize`:@@tasks.refresh:Refresh`,
    running: $localize`:@@tasks.running:Running now`,
    finished: $localize`:@@tasks.finished:Finished`,
  } as const;
  private readonly service = inject(TasksService);
  /** Whether the schedules are read, which the Repeating list above what runs shows. */
  protected readonly ready = inject(SchedulesService).read;
  /** Whether any task has finished, which go under their own heading. */
  protected readonly finished = computed(() =>
    this.service.tasks().some((task) => task.status === 'finished'),
  );

  protected refresh(): void {
    void this.service.refresh();
  }
}
