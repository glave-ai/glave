import { Service, inject, signal } from '@angular/core';
import type { ProviderConnectParams, ProviderInfo } from '@glave/client';

import { onChange } from './changes.service';
import { ModelsService } from './models.service';
import { SessionService } from './session.service';

/**
 * The model accounts the person connected (`provider/list`), and connecting or removing one:
 * after each change the models are read again, since they follow the accounts.
 */
@Service()
export class ProvidersService {
  private readonly session = inject(SessionService);
  private readonly models = inject(ModelsService);
  private readonly list = signal<readonly ProviderInfo[]>([]);

  readonly accounts = this.list.asReadonly();

  constructor() {
    onChange('providers', () => void this.load());
  }

  /** Reads the accounts; a failed read leaves the list as it was. */
  async load(): Promise<void> {
    try {
      const { providers } = await this.session.call('provider/list', {});
      this.list.set(providers);
    } catch {
      // The settings page shows what it has; there is nothing to fix here.
    }
  }

  /** Connects an account: the server checks the key first, and throws when it will not do. */
  async connect(params: ProviderConnectParams): Promise<ProviderInfo> {
    const { provider } = await this.session.call('provider/connect', params);
    await Promise.all([this.load(), this.models.reload()]);
    return provider;
  }

  /**
   * Has every account's service list its models again now, as they change there; throws when
   * a service could not say, after keeping what the others did.
   */
  async refresh(): Promise<void> {
    try {
      const { providers } = await this.session.call('provider/refresh', {});
      this.list.set(providers);
    } finally {
      await this.models.reload();
    }
  }

  /** Removes the account `id`, and its key with it. */
  async remove(id: string): Promise<void> {
    await this.session.call('provider/remove', { provider: id });
    await Promise.all([this.load(), this.models.reload()]);
  }
}
