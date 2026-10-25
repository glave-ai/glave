import { Service, computed, inject, signal } from '@angular/core';
import type { Permissions, ServerInfo } from '@glave/client';

import { SessionService } from './session.service';

/**
 * What the glave that served the app says about itself (`server/discover`): its name and
 * version, the preset a thread runs with when it chooses none, the presets a thread may choose,
 * and, where no sandbox can run, the sentence saying so. Read once, when first asked.
 */
@Service()
export class ServerService {
  private readonly session = inject(SessionService);
  private readonly read = signal<ServerInfo | null>(null);
  private pending: Promise<void> | null = null;

  /** The server's answer, or `null` until it has given one. */
  readonly info = this.read.asReadonly();
  /** What the agent may do where nothing chose, or `null` until the server has said. */
  readonly permissions = computed<Permissions | null>(() => this.info()?.permissions ?? null);
  /** The presets a thread may choose, the most cautious first; none until the server has said. */
  readonly presets = computed<readonly Permissions[]>(() => this.info()?.presets ?? []);
  /** Why Read-only and Workspace ask before everything here, in the server's words, if they do. */
  readonly unsandboxed = computed<string | null>(() => this.info()?.unsandboxed ?? null);

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
}
