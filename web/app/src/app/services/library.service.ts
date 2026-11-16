import { Service, inject, signal } from '@angular/core';
import type { AssetId, LibraryItem } from '@glave/client';

import { SessionService } from './session.service';

/**
 * The person's Library (`library/list`): what glave made for them, every project's, newest
 * first, as the server says it; and taking an item out of it (`library/remove`).
 */
@Service()
export class LibraryService {
  private readonly session = inject(SessionService);
  private readonly list = signal<readonly LibraryItem[]>([]);
  private readonly read = signal(false);
  private readonly failed = signal<string | null>(null);

  readonly items = this.list.asReadonly();
  /** Whether the Library has been read once, so an empty one says so only then. */
  readonly loaded = this.read.asReadonly();
  /** Why it could not be read or changed, in the server's words. */
  readonly error = this.failed.asReadonly();

  async refresh(): Promise<void> {
    await this.run(async () => {
      const { items } = await this.session.call('library/list', {});
      this.list.set(items);
      this.read.set(true);
    });
  }

  /** Takes `asset` out of the Library; the chat it came from keeps its copy. */
  async remove(asset: AssetId): Promise<void> {
    await this.run(async () => {
      await this.session.call('library/remove', { asset });
      this.list.update((items) => items.filter((item) => item.asset !== asset));
    });
  }

  private async run(work: () => Promise<void>): Promise<void> {
    try {
      await work();
      this.failed.set(null);
    } catch (error: unknown) {
      this.failed.set(error instanceof Error ? error.message : String(error));
    }
  }
}
