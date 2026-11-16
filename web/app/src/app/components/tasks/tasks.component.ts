import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { GlzButtonComponent } from '@glave/glaze';

import { TasksService } from '../../services/tasks.service';
import { PageHeaderComponent } from '../page-header/page-header.component';
import { RepeatingListComponent } from '../repeating-list/repeating-list.component';
import { TaskListComponent } from '../task-list/task-list.component';

/** The Tasks page: what repeats, where schedules run, and the tasks running now. */
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
  } as const;
  private readonly service = inject(TasksService);

  protected refresh(): void {
    void this.service.refresh();
  }
}
