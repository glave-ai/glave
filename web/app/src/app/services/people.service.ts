import { Service, inject, signal } from '@angular/core';
import { ApiError, type InviteInfo, type OrgRole, type PersonInfo } from '@glave/client';

import { unreached } from '../lib/text';
import { ServerService } from './server.service';
import { SessionService } from './session.service';

/** An invite just made: the link that opens it, this once, and the invite itself. */
export interface Made {
  readonly link: string;
  readonly invite: InviteInfo;
}

/** The most people one list reads, as the server lists at most. */
const LISTED = 1_000;

/**
 * The organization's people and the invites waiting, for its admins (`people/list`,
 * `invite/list`): giving someone a role, taking them out, inviting someone and cancelling an
 * invite. What each person is to the caller (themselves, the roles they may give them,
 * whether they may remove them) is the server's word. Each change reads both lists again, so
 * what shows is what the server keeps, and asks the server again what exists, since an invite
 * waiting shows the organization. Each says `null` when it worked, else why not, in the
 * server's sentence when it refused.
 */
@Service()
export class PeopleService {
  private readonly session = inject(SessionService);
  private readonly server = inject(ServerService);
  private readonly listed = signal<readonly PersonInfo[]>([]);
  private readonly waiting = signal<readonly InviteInfo[]>([]);
  private readonly read = signal(Date.now());

  /** The organization's people, in the server's order, as it last listed them. */
  readonly people = this.listed.asReadonly();
  /** The invites waiting to be opened, oldest first. */
  readonly invites = this.waiting.asReadonly();
  /** When the lists were last read, in milliseconds since the epoch: how long invites have left counts from it. */
  readonly readAt = this.read.asReadonly();

  /** Reads the people and the invites again. */
  async load(): Promise<string | null> {
    return this.asking(async () => {
      const [{ people }, { invites }] = await Promise.all([
        this.session.call('people/list', { limit: LISTED }),
        this.session.call('invite/list', {}),
      ]);
      // Always a new list, the same as before or not: a menu the person changed shows again
      // what the server keeps.
      this.listed.set([...people]);
      this.waiting.set(invites);
      this.read.set(Date.now());
      return null;
    });
  }

  /** Gives `person` the role `role`. */
  async setRole(person: string, role: OrgRole): Promise<string | null> {
    return this.changing(() => this.session.call('role/set', { person, role }));
  }

  /** Takes `person` out of the organization. */
  async remove(person: string): Promise<string | null> {
    return this.changing(() => this.session.call('person/remove', { person }));
  }

  /** Cancels the invite `invite`. */
  async revoke(invite: string): Promise<string | null> {
    return this.changing(() => this.session.call('invite/revoke', { invite }));
  }

  /**
   * Makes an invite giving `role`, emailed to `email` when there is one: its link at `origin`,
   * or why it was not made.
   */
  async invite(role: OrgRole, email: string | null, origin: string): Promise<Made | string> {
    try {
      const params = email === null ? { role } : { role, email };
      const { code, invite } = await this.session.call('invite/create', params);
      return { link: `${origin}/#invite=${code}`, invite };
    } catch (error: unknown) {
      return error instanceof ApiError ? error.message : unreached();
    } finally {
      // Refused or not, so what shows is what the server keeps: an invite may wait though
      // its email was not sent.
      await Promise.all([this.load(), this.server.reread()]);
    }
  }

  /**
   * Makes a change, then reads the lists again, refused or not, so what shows is what the server
   * keeps.
   */
  private async changing(change: () => Promise<unknown>): Promise<string | null> {
    const refused = await this.asking(async () => {
      await change();
      return null;
    });
    const [reread] = await Promise.all([this.load(), this.server.reread()]);
    return refused ?? reread;
  }

  /** What `ask` says, or the server's sentence when it refused, or the app's own. */
  private async asking(ask: () => Promise<string | null>): Promise<string | null> {
    try {
      return await ask();
    } catch (error: unknown) {
      return error instanceof ApiError ? error.message : unreached();
    }
  }
}
