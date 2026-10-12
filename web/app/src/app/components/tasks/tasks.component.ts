import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { GlzButtonComponent } from '@glave/glaze';

import { TasksService } from '../../services/tasks.service';
import { PageHeaderComponent } from '../page-header/page-header.component';
import { TaskListComponent } from '../task-list/task-list.component';

const COPY = {
  heading: 'Tasks',
  intro: 'Work running with no one attached: started in the background, or by a schedule.',
  refresh: 'Refresh',
} as const;

/** The Tasks page: the background tasks, as the thread's tools panel shows them too. */
@Component({
  selector: 'app-tasks',
  imports: [GlzButtonComponent, PageHeaderComponent, TaskListComponent],
  templateUrl: './tasks.component.html',
  styleUrl: './tasks.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TasksComponent {
  protected readonly copy = COPY;
  private readonly service = inject(TasksService);

  protected refresh(): void {
    void this.service.refresh();
  }
}
