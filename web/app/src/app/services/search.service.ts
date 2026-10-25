import { Service, computed, inject, signal } from '@angular/core';
import { ApiError, type SearchInfo } from '@glave/client';

import { sentence, unreached } from '../lib/text';
import { SessionService } from './session.service';

/**
 * Whether the person's agent may search the web, as the server says (`search/get`,
 * `search/set`): read as the app starts, signed in, so the composer shows it while it is on,
 * and again as Settings opens. The person's switch is sent, and the server's answer is what
 * stays; when it keeps none, what it said before stays, with why.
 */
@Service()
export class SearchService {
  private readonly session = inject(SessionService);
  private readonly said = signal<SearchInfo | null>(null);
  private readonly failed = signal<string | null>(null);
  /** How many times it was asked, so an answer that comes late never wins. */
  private asked = 0;

  /** What the server last said; none until it has. */
  readonly search = this.said.asReadonly();
  /** Why the person's last switch was not kept, until the next. */
  readonly problem = this.failed.asReadonly();
  /** Whether the agent may search the web now. */
  readonly on = computed(() => {
    const search = this.said();
    return search?.kind === 'chosen' && search.on;
  });

  constructor() {
    void this.load();
  }

  /** Reads it again; one that cannot be read leaves what was read before. */
  async load(): Promise<void> {
    this.asked += 1;
    const asked = this.asked;
    try {
      const { search } = await this.session.call('search/get', {});
      if (asked === this.asked) {
        this.said.set(search);
      }
    } catch {
      // Unread, nothing shows it: Settings shows no switch, and the composer no globe.
    }
  }

  /** Turns web search on or off; the server's answer is what stays. */
  async set(on: boolean): Promise<void> {
    this.asked += 1;
    try {
      const { search } = await this.session.call('search/set', { on });
      this.said.set(search);
      this.failed.set(null);
    } catch (error: unknown) {
      this.failed.set(error instanceof ApiError ? sentence(error.message) : unreached());
    }
  }
}
