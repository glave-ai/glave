import { Service, computed, inject, signal } from '@angular/core';
import type { TaskInfo, ThreadId } from '@glave/client';

import { SessionService } from './session.service';

/** The tasks of the server's own space: running here first, then elsewhere, then finished. */
@Service()
export class TasksService {
  private readonly session = inject(SessionService);
  private readonly list = signal<readonly TaskInfo[]>([]);
  private readonly failed = signal<string | null>(null);
  private readonly read = signal(false);

  readonly tasks = this.list.asReadonly();
  readonly error = this.failed.asReadonly();
  readonly loaded = this.read.asReadonly();
  readonly running = computed(() => this.list().filter((task) => task.status === 'running').length);

  async refresh(): Promise<void> {
    try {
      const { tasks } = await this.session.call('tasks/list', { limit: 50 });
      this.list.set(tasks);
      this.failed.set(null);
    } catch (error: unknown) {
      this.failed.set(error instanceof Error ? error.message : String(error));
    } finally {
      this.read.set(true);
    }
  }

  /** Stops the task working in `thread`, then reads the list again. */
  async stop(thread: ThreadId): Promise<void> {
    try {
      await this.session.call('tasks/stop', { thread });
    } catch (error: unknown) {
      this.failed.set(error instanceof Error ? error.message : String(error));
    }
    await this.refresh();
  }
}
