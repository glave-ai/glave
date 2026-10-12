import { Service, computed, inject, signal } from '@angular/core';
import type { ApprovalMode, ServerInfo } from '@glave/client';

import { SessionService } from './session.service';

/**
 * What the glave that served the app says about itself (`server/discover`): its name and
 * version, the approval mode a thread runs in when it chooses none, and the modes a thread may
 * choose. Read once, when first asked.
 */
@Service()
export class ServerService {
  private readonly session = inject(SessionService);
  private readonly read = signal<ServerInfo | null>(null);
  private pending: Promise<void> | null = null;

  /** The server's answer, or `null` until it has given one. */
  readonly info = this.read.asReadonly();
  /** How much the server asks before acting, or `null` until it has said. */
  readonly approval = computed<ApprovalMode | null>(() => this.info()?.approval ?? null);
  /** The modes a thread may choose, cautious first; none until the server has said. */
  readonly approvals = computed<readonly ApprovalMode[]>(() => this.info()?.approvals ?? []);

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
