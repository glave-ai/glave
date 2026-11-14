import { ChangeDetectionStrategy, Component, LOCALE_ID, computed, inject, signal } from '@angular/core';
import type { Money } from '@glave/client';
import { GlzCardComponent, formatMoney } from '@glave/glaze';

import { ServerService } from '../../services/server.service';
import { SessionService } from '../../services/session.service';

/**
 * In Settings, where the server keeps a balance: what is left of it, as the server says, in
 * dollars as the reader writes them. Only for those the server shows it to; elsewhere nothing,
 * not even room in the page.
 */
@Component({
  selector: 'app-balance',
  imports: [GlzCardComponent],
  templateUrl: './balance.component.html',
  styleUrl: './balance.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class BalanceComponent {
  private readonly session = inject(SessionService);
  private readonly server = inject(ServerService);
  private readonly locale = inject(LOCALE_ID);
  protected readonly copy = {
    heading: $localize`:@@balance.heading:Balance`,
  } as const;
  /** What is left, once the server has said it. */
  private readonly balance = signal<Money | null>(null);
  protected readonly left = computed(() => {
    const balance = this.balance();
    if (balance === null) {
      return null;
    }
    const amount = formatMoney(balance, this.locale);
    return $localize`:@@balance.left:${amount}:AMOUNT: left`;
  });

  constructor() {
    void this.load();
  }

  private async load(): Promise<void> {
    await this.server.load();
    if (!this.server.exists().credits) {
      return;
    }
    try {
      const { credits } = await this.session.call('credits/get', {});
      this.balance.set(credits.account?.balance ?? null);
    } catch {
      // Where it can't be read, Settings shows nothing of it.
      this.balance.set(null);
    }
  }
}
