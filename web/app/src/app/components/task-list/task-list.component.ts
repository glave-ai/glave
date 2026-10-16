import { ChangeDetectionStrategy, Component, DestroyRef, computed, inject } from '@angular/core';
import { RouterLink } from '@angular/router';
import type { TaskInfo } from '@glave/client';
import {
  GlzBadgeComponent,
  GlzButtonComponent,
  GlzCardComponent,
  GlzEmptyStateComponent,
  type GlzTone,
  formatDuration,
  formatMoney,
  formatTokens,
} from '@glave/glaze';
import { ListChecks } from 'lucide';

import { TasksService } from '../../services/tasks.service';

const COPY = {
  stop: 'Stop',
  emptyHeading: 'No tasks',
  emptyText: 'Run something in the background and it shows up here.',
  failed: 'Could not read the tasks:',
  noCost: 'cost not reported',
} as const;

const STATUS: Readonly<
  Record<TaskInfo['status'], { readonly text: string; readonly tone: GlzTone }>
> = {
  running: { text: 'running', tone: 'neutral' },
  elsewhere: { text: 'running elsewhere', tone: 'neutral' },
  finished: { text: 'finished', tone: 'success' },
};

/**
 * The background tasks, live: each one's title, status, model, tokens, cost and time, with
 * Stop while it runs. Read again every few seconds while it shows; the Tasks page and the
 * thread's tools panel both show this one list.
 */
@Component({
  selector: 'app-task-list',
  imports: [
    RouterLink,
    GlzBadgeComponent,
    GlzButtonComponent,
    GlzCardComponent,
    GlzEmptyStateComponent,
  ],
  templateUrl: './task-list.component.html',
  styleUrl: './task-list.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TaskListComponent {
  protected readonly copy = COPY;
  protected readonly icon = ListChecks;
  protected readonly service = inject(TasksService);

  protected readonly rows = computed(() =>
    this.service.tasks().map((task) => ({
      thread: task.thread,
      link: ['/t', task.thread],
      title: task.title,
      status: task.outcome ?? STATUS[task.status].text,
      tone: STATUS[task.status].tone,
      running: task.status === 'running',
      details: [
        task.model,
        `${formatTokens(task.tokens.input + task.tokens.output)} tokens`,
        task.cost === null || task.cost === undefined ? COPY.noCost : formatMoney(task.cost),
        formatDuration(task.elapsed_ms),
      ].join(' · '),
      stopLabel: `${COPY.stop} ${task.title}`,
    })),
  );

  constructor() {
    const unwatch = this.service.watch();
    inject(DestroyRef).onDestroy(unwatch);
  }

  protected stop(thread: string): void {
    void this.service.stop(thread);
  }
}
