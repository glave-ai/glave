import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  LOCALE_ID,
  computed,
  inject,
} from '@angular/core';
import { RouterLink } from '@angular/router';
import type { Outcome, TaskInfo } from '@glave/client';
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

/**
 * The background tasks, live: each one's title, status, model, tokens, cost and time, with
 * Stop while it runs. Read again every few seconds while it shows, on the Tasks page.
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
  protected readonly copy = {
    stop: $localize`:@@taskList.stop:Stop`,
    emptyHeading: $localize`:@@taskList.emptyHeading:No tasks`,
    emptyText: $localize`:@@taskList.emptyText:Run something in the background and it shows up here.`,
    noCost: $localize`:@@taskList.noCost:cost not reported`,
  } as const;
  protected readonly icon = ListChecks;
  protected readonly service = inject(TasksService);
  private readonly locale = inject(LOCALE_ID);
  private readonly statuses: Readonly<
    Record<TaskInfo['status'], { readonly text: string; readonly tone: GlzTone }>
  > = {
    running: { text: $localize`:@@taskList.running:running`, tone: 'neutral' },
    elsewhere: { text: $localize`:@@taskList.elsewhere:running elsewhere`, tone: 'neutral' },
    finished: { text: $localize`:@@taskList.finished:finished`, tone: 'success' },
  };
  /** How a finished task's turn ended, in a word. */
  private readonly outcomes: Readonly<Record<Outcome, string>> = {
    completed: $localize`:@@taskList.completed:completed`,
    cancelled: $localize`:@@taskList.cancelled:cancelled`,
    failed: $localize`:@@taskList.outcomeFailed:failed`,
    interrupted: $localize`:@@taskList.interrupted:interrupted`,
    limit_reached: $localize`:@@taskList.limitReached:stopped at its limit`,
  };
  /** Why the tasks could not be read, as the server says. */
  protected readonly problem = computed(() => {
    const error = this.service.error();
    return error === null
      ? null
      : $localize`:@@taskList.failed:Could not read the tasks: ${error}:ERROR:`;
  });

  protected readonly rows = computed(() =>
    this.service.tasks().map((task) => {
      const tokens = formatTokens(task.tokens.input + task.tokens.output, this.locale);
      const cost = task.cost ?? null;
      return {
        thread: task.thread,
        link: ['/t', task.thread],
        title: task.title,
        status: task.outcome ? this.outcomes[task.outcome] : this.statuses[task.status].text,
        tone: this.statuses[task.status].tone,
        running: task.status === 'running',
        details: [
          task.model,
          $localize`:@@taskList.tokens:${tokens}:TOKENS: tokens`,
          cost === null ? this.copy.noCost : formatMoney(cost, this.locale),
          formatDuration(task.elapsed_ms, this.locale),
        ].join(' · '),
        stopLabel: $localize`:@@taskList.stopTask:Stop ${task.title}:TASK:`,
      };
    }),
  );

  constructor() {
    const unwatch = this.service.watch();
    inject(DestroyRef).onDestroy(unwatch);
  }

  protected stop(thread: string): void {
    void this.service.stop(thread);
  }
}
