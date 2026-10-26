import { Service, computed, inject, signal } from '@angular/core';
import { ApiError, type SearchInfo } from '@glave/client';

import { sentence, unreached } from '../lib/text';
import { onChange } from './changes.service';
import { SessionService } from './session.service';

/**
 * Whether the person's agent may search the web, as the server says (`search/get`,
 * `search/set`): read as the app starts, signed in, so the composer shows it while it is on,
 * and again as Settings opens, with how often it searched this month. Each switch is sent once
 * the one before it is answered, so the server keeps the person's last, and the answer to the
 * last is what stays; when the server keeps none, what it said before stays, with why.
 */
@Service()
export class SearchService {
  private readonly session = inject(SessionService);
  private readonly said = signal<SearchInfo | null>(null);
  private readonly failed = signal<string | null>(null);
  /** How many times it was asked, so an answer that comes late never wins. */
  private asked = 0;
  /** The switches on their way, each sent once the one before it is answered. */
  private sending: Promise<unknown> = Promise.resolve();
  /** How many switches are on their way: the answer to the last says what is so now. */
  private switching = 0;
  /** Whether anything here has asked how often it searched, which a read again asks too. */
  private counting = false;

  /** What the server last said; none until it has. */
  readonly search = this.said.asReadonly();
  /** Why the person's last switch was not kept, until the next, or the next read. */
  readonly problem = this.failed.asReadonly();
  /** Whether the agent may search the web now. */
  readonly on = computed(() => {
    const search = this.said();
    return search?.kind === 'chosen' && search.on;
  });

  constructor() {
    void this.load();
    // Changed elsewhere, or by this tab: a switch on its way here is answered with what is so.
    onChange('search', () => {
      if (this.switching === 0) {
        void this.load(this.counting);
      }
    });
  }

  /**
   * Reads it again, with how often it searched this month when `usage` asks, which the server
   * reads from the month's threads; one that cannot be read leaves what was read before.
   */
  async load(usage = false): Promise<void> {
    this.counting ||= usage;
    this.asked += 1;
    const asked = this.asked;
    this.failed.set(null);
    try {
      const { search } = await this.session.call('search/get', { usage });
      if (asked === this.asked) {
        this.said.set(search);
      }
    } catch {
      // Unread, nothing shows it: Settings shows no switch, and the composer no globe.
    }
  }

  /** Turns web search on or off; the server's answer to the last switch is what stays. */
  async set(on: boolean): Promise<void> {
    this.asked += 1;
    const asked = this.asked;
    const answered = this.sending.then(() => this.session.call('search/set', { on }));
    this.sending = answered.catch(() => undefined);
    this.switching += 1;
    try {
      const { search } = await answered;
      if (asked === this.asked) {
        this.said.set(search);
        this.failed.set(null);
      }
    } catch (error: unknown) {
      if (asked === this.asked) {
        this.failed.set(error instanceof ApiError ? sentence(error.message) : unreached());
      }
    } finally {
      this.switching -= 1;
    }
  }
}
