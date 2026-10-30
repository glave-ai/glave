import { Service, computed, inject, signal } from '@angular/core';
import type { CredentialInfo, DeviceCode } from '@glave/client';

import { onChange } from './changes.service';
import { SessionService } from './session.service';

/**
 * The accounts with git hosts the person connected (`credential/list`): connecting one with a
 * token or by signing in on GitHub, and removing one. Where GitHub's sign-in goes, and where
 * its repositories are chosen, come with the list. Someone the server keeps no git hosts for
 * (a company login's people) is never listed any.
 */
@Service()
export class CredentialsService {
  private readonly session = inject(SessionService);
  private readonly list = signal<readonly CredentialInfo[] | null>(null);
  private readonly choose = signal<string | null>(null);
  private readonly signsIn = signal<string | null>(null);

  readonly accounts = computed<readonly CredentialInfo[]>(() => this.list() ?? []);
  /** Whether the server listed them: it keeps git hosts for this person. */
  readonly listed = computed(() => this.list() !== null);
  /** GitHub's page where the person chooses which repositories glave may use, if any. */
  readonly install = this.choose.asReadonly();
  /** Where signing in to GitHub goes, as the server says: the host of GitHub's account. */
  readonly github = this.signsIn.asReadonly();

  constructor() {
    onChange('credentials', () => void this.load());
  }

  /** Reads the accounts; a failed read leaves the list as it was. */
  async load(): Promise<void> {
    try {
      const listed = await this.session.call('credential/list', {});
      this.list.set(listed.credentials);
      this.choose.set(listed.install ?? null);
      this.signsIn.set(listed.github);
    } catch {
      // The settings page shows what it has; there is nothing to fix here.
    }
  }

  /** Connects a host with a token: the server checks it first, and throws when it will not do. */
  async connect(url: string, token: string): Promise<CredentialInfo> {
    const { credential } = await this.session.call('credential/connect', { url, token });
    await this.load();
    return credential;
  }

  /** Removes the account `id`, and its token with it. */
  async remove(id: string): Promise<void> {
    await this.session.call('credential/remove', { credential: id });
    await this.load();
  }

  /** Starts signing in to GitHub in the browser: the code to enter, and where. */
  async start(): Promise<DeviceCode> {
    const { device } = await this.session.call('credential/device/start', {});
    return device;
  }

  /**
   * Waits for the sign-in `flow` until the person approves it on GitHub, asking again each time
   * the server says it still waits, unless `stopped` says to give up first: then nothing.
   */
  async finish(flow: string, stopped: () => boolean): Promise<CredentialInfo | null> {
    while (!stopped()) {
      const polled = await this.session.call('credential/device/poll', { flow });
      if ('credential' in polled) {
        await this.load();
        return polled.credential;
      }
    }
    return null;
  }
}
