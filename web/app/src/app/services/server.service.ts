import { Service, computed, inject, signal } from '@angular/core';
import type { Permissions, ServerInfo } from '@glave/client';

import { SessionService } from './session.service';

/**
 * What the glave that served the app says about itself (`server/discover`): its name and
 * version, the mode a thread runs with when it chooses none, the modes a thread may choose,
 * and, where no sandbox can run, the sentence saying so. Read once, when first asked; its
 * version is asked again when a part of the app could not load, to tell whether glave was
 * updated meanwhile.
 */
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
