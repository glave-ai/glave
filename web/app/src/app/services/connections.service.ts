import { Service, inject, signal } from '@angular/core';
import type { ConnectionInfo, SignInLink } from '@glave/client';

import { connectionProblem } from '../lib/connections';
import { SessionService } from './session.service';

/**
 * The person's connections (`connection/list`): the services they are signed in to, and those
 * that ask them to sign in; signing in to one in the browser, and removing a sign-in.
 */
@Service()
export class ConnectionsService {
  private readonly session = inject(SessionService);
  private readonly list = signal<readonly ConnectionInfo[] | null>(null);
  private readonly failure = signal<string | null>(null);

  /** The connections, once listed. */
  readonly connections = this.list.asReadonly();
  /** Why they could not be listed last, in the server's words. */
  readonly problem = this.failure.asReadonly();

  /** Lists the connections; a list that fails keeps what was listed before, and says why. */
  async load(): Promise<void> {
    try {
      const { connections } = await this.session.call('connection/list', {});
      this.list.set(connections);
      this.failure.set(null);
    } catch (error: unknown) {
      this.failure.set(connectionProblem(error));
    }
  }

  /** Starts signing in to `server` in the browser: the page to open, and the sign-in's flow. */
  async start(server: string): Promise<SignInLink> {
    const { sign_in } = await this.session.call('connection/start', { server });
    return sign_in;
  }

  /**
   * Waits for the sign-in `flow` until the person has signed in in the browser, asking again
   * each time the server says it still waits, then lists the connections again; or until
   * `signal` says to stop. A refusal is thrown on.
   */
  async finish(flow: string, signal: AbortSignal): Promise<void> {
    while (!signal.aborted) {
      const polled = await this.session.call('connection/poll', { flow }, signal);
      if ('connection' in polled) {
        await this.load();
        return;
      }
    }
  }

  /** Removes the sign-in to `server`, destroying its tokens, then lists the connections again. */
  async remove(server: string): Promise<void> {
    await this.session.call('connection/remove', { server });
    await this.load();
  }
}
