import { Service, inject, signal } from '@angular/core';
import { ApiError, type Grouping } from '@glave/client';

import { sentence, unreached } from '../lib/text';
import { SessionService } from './session.service';

/**
 * How the side nav groups the person's threads: by project until they choose, their choice kept
 * by the server (`grouping/get`, `grouping/set`), so it follows them to every browser. The side
 * nav reads it as it opens, signed in. A choice shows at once; the server's answer is what
 * stays, and when it keeps none, the choice before it comes back, with why.
 */
@Service()
export class GroupingService {
  private readonly session = inject(SessionService);
  private readonly chosen = signal<Grouping>('project');
  private readonly failed = signal<string | null>(null);
  /**
   * How many choices the person made in this tab: only the latest choice's answer is taken, and
   * a read begun before a choice never undoes it.
   */
  private choices = 0;

  /** How the person's threads are grouped. */
  readonly grouping = this.chosen.asReadonly();
  /** Why the last choice was not kept, until the next. */
  readonly problem = this.failed.asReadonly();

  /** Reads the person's choice; one that cannot be read leaves the threads as they are. */
  async read(): Promise<void> {
    const choices = this.choices;
    try {
      const { grouping } = await this.session.call('grouping/get', {});
      if (choices === this.choices) {
        this.chosen.set(grouping);
      }
    } catch {
      // Unread, the threads stay grouped as they are; the lists say when glave is out of reach.
    }
  }

  /** Groups the threads by `grouping`, at once, and keeps it as the person's choice. */
  async choose(grouping: Grouping): Promise<void> {
    const before = this.chosen();
    this.choices += 1;
    const choice = this.choices;
    this.chosen.set(grouping);
    try {
      const { grouping: kept } = await this.session.call('grouping/set', { grouping });
      if (choice === this.choices) {
        this.chosen.set(kept);
        this.failed.set(null);
      }
    } catch (error: unknown) {
      if (choice === this.choices) {
        this.chosen.set(before);
        this.failed.set(error instanceof ApiError ? sentence(error.message) : unreached());
      }
    }
  }
}
