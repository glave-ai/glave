import { InjectionToken, Service, inject } from '@angular/core';

import { ServerService } from './server.service';
import { SessionService } from './session.service';

/**
 * The reader's own time zone, by its IANA name: the browser's, which the person's schedules
 * repeat in and their times are read against.
 */
export const READER_ZONE = new InjectionToken<string>('the reader’s time zone', {
  factory: () => Intl.DateTimeFormat().resolvedOptions().timeZone,
});

/**
 * Gives the server this browser's time zone (`zone/set`), which the person's schedules repeat
 * in, where schedules run: once a load, as a page the person chats from opens, so a schedule
 * the model sets up there repeats on their clocks; and again as the next such page opens when
 * it could not.
 */
@Service()
export class ZoneService {
  private readonly session = inject(SessionService);
  private readonly server = inject(ServerService);
  private readonly zone = inject(READER_ZONE);
  /** Whether the server has the zone, and the giving of it under way. */
  private given = false;
  private giving: Promise<void> | null = null;

  /** Gives the zone, unless the server has it or schedules don't run here; one giving at a time. */
  give(): Promise<void> {
    if (this.given) {
      return Promise.resolve();
    }
    this.giving ??= this.server
      .load()
      .then(async () => {
        if (this.server.schedules()) {
          await this.session.call('zone/set', { zone: this.zone });
          this.given = true;
        }
      })
      // Schedules repeat in the zone the server last had, until it is given again.
      .catch(() => undefined)
      .finally(() => {
        this.giving = null;
      });
    return this.giving;
  }
}
