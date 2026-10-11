import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
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
  heading: 'Tasks',
  intro: 'Work running with no one attached: started in the background, or by a schedule.',
  refresh: 'Refresh',
  stop: 'Stop',
  open: 'Open the thread',
  emptyHeading: 'No tasks',
  emptyText: 'Run something in the background and it shows up here.',
  failed: 'Could not read the tasks:',
  noCost: 'cost not reported',
} as const;

const STATUS: Readonly<
  Record<TaskInfo['status'], { readonly text: string; readonly tone: GlzTone }>
> = {
  running: { text: 'running', tone: 'accent' },
  elsewhere: { text: 'running elsewhere', tone: 'neutral' },
  finished: { text: 'finished', tone: 'success' },
};

/** The tasks panel: each task's title, status, model, tokens, cost and time, and Stop. */
@Component({
  selector: 'app-tasks',
  imports: [
    RouterLink,
    GlzBadgeComponent,
    GlzButtonComponent,
    GlzCardComponent,
    GlzEmptyStateComponent,
  ],
  templateUrl: './tasks.component.html',
  styleUrl: './tasks.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TasksComponent {
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
  protected readonly empty = computed(
    () => this.service.loaded() && this.service.tasks().length === 0,
  );

  constructor() {
    void this.service.refresh();
  }

  protected refresh(): void {
    void this.service.refresh();
  }

  protected stop(thread: string): void {
    void this.service.stop(thread);
  }
}
