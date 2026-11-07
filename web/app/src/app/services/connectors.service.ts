import { Service, inject, signal } from '@angular/core';
import type { ConnectorCategory, ConnectorInfo, ProjectId, SignInLink } from '@glave/client';

import { connectionProblem } from '../lib/connections';
import { onChange } from './changes.service';
import { SessionService } from './session.service';

/** glave's connectors the person may connect, and their categories, as the server lists them. */
export interface Catalog {
  readonly categories: readonly ConnectorCategory[];
  readonly connectors: readonly ConnectorInfo[];
  /** Those the threads of the project the list is for use, when it is for one. */
  readonly here: readonly ConnectorInfo[] | null;
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
  /** The project the list is for: where a connector connected from it is used. */
  private readonly forProject = signal<ProjectId | null>(null);

  constructor() {
    onChange('connections', () => void this.load());
  }

  /**
   * Lists the connectors, and with `project`, those its threads use; a list that fails keeps
   * what was listed before, and says why.
   */
  async load(project: ProjectId | null = this.forProject()): Promise<void> {
    this.forProject.set(project);
    this.answers += 1;
    const asked = this.answers;
    try {
      const [{ categories, connectors }, here] = await Promise.all([
        this.session.call('connector/list', {}),
        project === null
          ? Promise.resolve(null)
          : this.session.call('connector/list', { used_in: project }),
      ]);
      if (asked === this.answers) {
        this.list.set({ categories, connectors, here: here?.connectors ?? null });
        this.failure.set(null);
      }
    } catch (error: unknown) {
      if (asked === this.answers) {
        this.failure.set(connectionProblem(error));
      }
    }
  }

  /**
   * Connects the account `account` of the connector `id` (none: the one never named), used
   * only in `only`, by default the project the list is for, else everywhere: none once it is
   * connected, as one that asks no one to sign in is at once; else the page where the person
   * signs in, and the sign-in's flow.
   */
  async connect(
    id: string,
    account: string | null = null,
    only: ProjectId | null = this.forProject(),
  ): Promise<SignInLink | null> {
    const answer = await this.session.call('connector/connect', {
      connector: id,
      ...named(account),
      ...(only === null ? {} : { only }),
    });
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

  /**
   * Uses the account `account` of the connector `id` (none: the one never named) only in
   * `only`, a project or Chats, or everywhere, then lists again.
   */
  async place(id: string, account: string | null, only: ProjectId | null): Promise<void> {
    await this.session.call('connector/place', {
      connector: id,
      ...named(account),
      ...(only === null ? {} : { only }),
    });
    await this.load();
  }

  /**
   * Disconnects the account `account` of the connector `id` (none: the one never named),
   * destroying its sign-in, then lists them again.
   */
  async disconnect(id: string, account: string | null = null): Promise<void> {
    await this.session.call('connector/disconnect', { connector: id, ...named(account) });
    await this.load();
  }
}

/** An account's name as a call's parameters: none for the one never named. */
function named(account: string | null): { readonly account?: string } {
  return account === null ? {} : { account };
}
