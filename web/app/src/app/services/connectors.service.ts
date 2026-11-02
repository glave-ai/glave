import { Service, inject, signal } from '@angular/core';
import type { ConnectorCategory, ConnectorInfo, SignInLink } from '@glave/client';

import { connectionProblem } from '../lib/connections';
import { onChange } from './changes.service';
import { SessionService } from './session.service';

/** glave's connectors the person may connect, and their categories, as the server lists them. */
export interface Catalog {
  readonly categories: readonly ConnectorCategory[];
  readonly connectors: readonly ConnectorInfo[];
}

/**
 * glave's catalog of connectors (`connector/list`): the apps glave can work in for the person,
 * each an official server its maker runs; connecting one, signing in to it in the browser when
 * it asks, and disconnecting one.
 */
@Service()
export class ConnectorsService {
  private readonly session = inject(SessionService);
  private readonly list = signal<Catalog | null>(null);
  private readonly failure = signal<string | null>(null);
  /** Counts what the list was last set from, so an answer that comes late never wins. */
  private answers = 0;

  /** The catalog, once listed. */
  readonly catalog = this.list.asReadonly();
  /** Why it could not be listed last, in the server's words. */
  readonly problem = this.failure.asReadonly();

  constructor() {
    onChange('connections', () => void this.load());
  }

  /** Lists the connectors; a list that fails keeps what was listed before, and says why. */
  async load(): Promise<void> {
    this.answers += 1;
    const asked = this.answers;
    try {
      const { categories, connectors } = await this.session.call('connector/list', {});
      if (asked === this.answers) {
        this.list.set({ categories, connectors });
        this.failure.set(null);
      }
    } catch (error: unknown) {
      if (asked === this.answers) {
        this.failure.set(connectionProblem(error));
      }
    }
  }

  /**
   * Connects the connector `id`: none once it is connected, as one that asks no one to sign in
   * is at once; else the page where the person signs in, and the sign-in's flow.
   */
  async connect(id: string): Promise<SignInLink | null> {
    const answer = await this.session.call('connector/connect', { connector: id });
    if ('sign_in' in answer) {
      return answer.sign_in;
    }
    await this.load();
    return null;
  }

  /**
   * Waits for the sign-in `flow` until the person signed in in the browser, then lists the
   * connectors again: whether they did, before `signal` said to stop. A refusal is thrown on.
   */
  async finish(flow: string, signal: AbortSignal): Promise<boolean> {
    while (!signal.aborted) {
      const polled = await this.session.call('connection/poll', { flow }, signal);
      if ('connection' in polled) {
        await this.load();
        return true;
      }
    }
    return false;
  }

  /** Disconnects the connector `id`, destroying its sign-in, then lists them again. */
  async disconnect(id: string): Promise<void> {
    await this.session.call('connector/disconnect', { connector: id });
    await this.load();
  }
}
