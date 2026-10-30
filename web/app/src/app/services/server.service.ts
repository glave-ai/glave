import { Service, computed, inject, signal } from '@angular/core';
import type { Exists, Permissions, ServerInfo } from '@glave/client';

import { SessionService } from './session.service';

/**
 * What the glave that served the app says about itself (`server/discover`): its name and
 * version, the mode a thread runs with when it chooses none, the modes a thread may choose,
 * where no sandbox can run, the sentence saying so, and what exists beyond a person's own
 * server (an organization, workers), which every part of the app reads here and draws only
 * when the server says so. Read once, when first asked; its
 * version is asked again when a part of the app could not load, to tell whether glave was
 * updated meanwhile.
 */
/** What a server that has said nothing beyond a person's own has: none of it. */
const NONE: Exists = { organization: false, workers: false };

@Service()
export class ServerService {
  private readonly session = inject(SessionService);
  private readonly read = signal<ServerInfo | null>(null);
  private readonly newer = signal(false);
  private pending: Promise<void> | null = null;
  private asking: Promise<void> | null = null;

  /** The server's answer, or `null` until it has given one. */
  readonly info = this.read.asReadonly();
  /** What the agent may do where nothing chose, or `null` until the server has said. */
  readonly permissions = computed<Permissions | null>(() => this.info()?.permissions ?? null);
  /** The modes a thread may choose, in the order they are offered; none until the server has said. */
  readonly modes = computed<readonly Permissions[]>(() => this.info()?.modes ?? []);
  /** Why Auto and Ask first ask before everything here, in the server's words, if they do. */
  readonly unsandboxed = computed<string | null>(() => this.info()?.unsandboxed ?? null);
  /**
   * What exists beyond a person's own server, as the server says: none of it until it has
   * said, and none on a person's own computer. Admin and organization UI renders only when
   * this says there is one.
   */
  readonly exists = computed<Exists>(() => this.info()?.exists ?? NONE);
  /**
   * Whether the person may open Admin: their organization is shown, and the server says they
   * are one of its Admins or Owners.
   */
  readonly admin = computed(() => {
    const role = this.info()?.role;
    return this.exists().organization && (role === 'admin' || role === 'owner');
  });
  /**
   * Whether the person may invite someone here, though they are alone and no organization is
   * shown yet: once one is, Admin is where they invite.
   */
  readonly invitesAlone = computed(() => (this.info()?.invites ?? false) && !this.admin());
  /** Whether this glave sends email, as an invite's link: where it doesn't, no one is asked for an address. */
  readonly email = computed(() => this.info()?.email ?? false);
  /** Where people reach this glave from elsewhere, if they do, for links to it. */
  readonly origin = computed<string | null>(() => this.info()?.origin ?? null);
  /**
   * Whether glave was updated since this page loaded: the server said another version than
   * the one the page's code is of, the last time it was asked (`recheck`).
   */
  readonly updated = this.newer.asReadonly();

  /** Asks the server about itself, once; a failed ask is tried again next time. */
  load(): Promise<void> {
    this.pending ??= this.session.call('server/discover', {}).then(
      (info) => {
        this.read.set(info);
      },
      () => {
        this.pending = null;
      },
    );
    return this.pending;
  }

  /**
   * Asks the server its version again, as when a part of the app could not load: whether the
   * code the page asked for is gone because glave was updated (`updated`). One ask at a time;
   * a server out of reach leaves it as it was.
   */
  recheck(): Promise<void> {
    this.asking ??= this.session
      .call('server/discover', {})
      .then(
        (info) => {
          const loaded = this.session.loadedVersion();
          if (loaded !== null && info.version !== loaded) {
            this.newer.set(true);
          }
        },
        () => undefined,
      )
      .finally(() => {
        this.asking = null;
      });
    return this.asking;
  }
}
