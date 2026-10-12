import { Service, inject, signal } from '@angular/core';
import { ApiError, ErrorCode, type MemoryId, type Remembered } from '@glave/client';

import { SessionService } from './session.service';

/** What the memory page knows: the memories, or that memory is off for this server. */
export type MemoryState = 'loading' | 'ready' | 'off';

/** The person's and this space's memories, and the person's controls over them. */
@Service()
export class MemoryService {
  private readonly session = inject(SessionService);
  private readonly list = signal<readonly Remembered[]>([]);
  private readonly status = signal<MemoryState>('loading');
  private readonly failed = signal<string | null>(null);

  readonly memories = this.list.asReadonly();
  readonly state = this.status.asReadonly();
  readonly error = this.failed.asReadonly();

  async refresh(): Promise<void> {
    await this.run(async () => {
      const { memories } = await this.session.call('memory/list', {});
      this.list.set(memories);
      this.status.set('ready');
    });
  }

  async edit(memory: MemoryId, text: string): Promise<void> {
    await this.run(async () => {
      await this.session.call('memory/edit', { memory, text });
      await this.refresh();
    });
  }

  async forget(memory: MemoryId): Promise<void> {
    await this.run(async () => {
      await this.session.call('memory/forget', { memory });
      await this.refresh();
    });
  }

  private async run(work: () => Promise<void>): Promise<void> {
    try {
      await work();
      this.failed.set(null);
    } catch (error: unknown) {
      if (error instanceof ApiError && error.code === ErrorCode.MEMORY_OFF) {
        this.status.set('off');
        return;
      }
      this.failed.set(error instanceof Error ? error.message : String(error));
    }
  }
}
