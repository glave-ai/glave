import { Service, inject, signal } from '@angular/core';
import { ApiError, type Grouping } from '@glave/client';

import { sentence, unreached } from '../lib/text';
import { SessionService } from './session.service';

/**
 * How the side nav groups the person's threads: by project until they choose, their choice kept
 * by the server (`grouping/get`, `grouping/set`), so it follows them to every browser. The side
 * nav reads it as it opens, signed in. A choice shows at once and is sent after the ones before
 * it, so the server keeps the person's last; the server's answer is what stays, and when it
 * keeps none, what it last kept comes back, with why.
 */
@Service()
export class GroupingService {
  private readonly session = inject(SessionService);
  private readonly chosen = signal<Grouping>('project');
  private readonly failed = signal<string | null>(null);
  /** What the server last said it keeps. */
  private kept: Grouping = 'project';
  /**
   * How many choices the person made in this tab: only the latest choice's answer is shown, and
   * a read begun before a choice never undoes it.
   */
  private choices = 0;
  /** The choices on their way, each sent once the one before it is answered. */
  private sending: Promise<unknown> = Promise.resolve();

  /** How the person's threads are grouped. */
  readonly grouping = this.chosen.asReadonly();
  /** Why the last choice was not kept, until the next, or the next read. */
  readonly problem = this.failed.asReadonly();

  /** Reads the person's choice; one that cannot be read leaves the threads as they are. */
  async read(): Promise<void> {
    const choices = this.choices;
    this.failed.set(null);
    try {
      const { grouping } = await this.session.call('grouping/get', {});
      if (choices === this.choices) {
        this.kept = grouping;
        this.chosen.set(grouping);
      }
    } catch {
      // Unread, the threads stay grouped as they are; the lists say when glave is out of reach.
    }
  }

  /** Groups the threads by `grouping`, at once, and keeps it as the person's choice. */
  async choose(grouping: Grouping): Promise<void> {
    this.choices += 1;
    const choice = this.choices;
    this.chosen.set(grouping);
    const answered = this.sending.then(() => this.session.call('grouping/set', { grouping }));
    this.sending = answered.catch(() => undefined);
    try {
      this.kept = (await answered).grouping;
      if (choice === this.choices) {
        this.chosen.set(this.kept);
        this.failed.set(null);
      }
    } catch (error: unknown) {
      if (choice === this.choices) {
        this.chosen.set(this.kept);
        this.failed.set(error instanceof ApiError ? sentence(error.message) : unreached());
      }
    }
  }
}
