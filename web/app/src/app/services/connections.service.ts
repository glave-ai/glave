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
  /** Counts what the list was last set from, so an answer that comes late never wins. */
  private answers = 0;

  /** The connections, once listed. */
  readonly connections = this.list.asReadonly();
  /** Why they could not be listed last, in the server's words. */
  readonly problem = this.failure.asReadonly();

  /**
   * Lists the connections; a list that fails keeps what was listed before, and says why. A
   * list overtaken meanwhile, by another or by a sign-in's answer, is set aside.
   */
  async load(): Promise<void> {
    this.answers += 1;
    const asked = this.answers;
    try {
      const { connections } = await this.session.call('connection/list', {});
      if (asked === this.answers) {
        this.list.set(connections);
        this.failure.set(null);
      }
    } catch (error: unknown) {
      if (asked === this.answers) {
        this.failure.set(connectionProblem(error));
      }
    }
  }

  /** Starts signing in to `server` in the browser: the page to open, and the sign-in's flow. */
  async start(server: string): Promise<SignInLink> {
    const { sign_in } = await this.session.call('connection/start', { server });
    return sign_in;
  }

  /**
   * Waits for the sign-in `flow` until the person has signed in in the browser, asking again
   * each time the server says it still waits, and takes the connection the server answers in
   * place of what was listed at its address; or until `signal` says to stop. A refusal is
   * thrown on.
   */
  async finish(flow: string, signal: AbortSignal): Promise<ConnectionInfo | null> {
    while (!signal.aborted) {
      const polled = await this.session.call('connection/poll', { flow }, signal);
      if ('connection' in polled) {
        this.take(polled.connection);
        return polled.connection;
      }
    }
    return null;
  }

  /** Removes the sign-in to `server`, destroying its tokens, then lists the connections again. */
  async remove(server: string): Promise<void> {
    await this.session.call('connection/remove', { server });
    await this.load();
  }

  /** `connection`, as the server just answered it, in place of what was listed at its address. */
  private take(connection: ConnectionInfo): void {
    this.answers += 1;
    this.list.update((list) => {
      const listed = list ?? [];
      return listed.some((kept) => kept.url === connection.url)
        ? listed.map((kept) => (kept.url === connection.url ? connection : kept))
        : [...listed, connection];
    });
  }
}
