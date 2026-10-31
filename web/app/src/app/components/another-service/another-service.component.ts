import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { GlzButtonComponent, GlzFormFieldComponent, GlzInputDirective } from '@glave/glaze';

import { connectProblem, hostOf } from '../../lib/credentials';
import { CredentialsService } from '../../services/credentials.service';

/**
 * Connections' "Another service": a git server of the person's own, GitLab or another, added
 * by its address, then the access token made in its settings, which the server checks first
 * and keeps locked away, never shown again.
 */
@Component({
  selector: 'app-another-service',
  imports: [GlzButtonComponent, GlzFormFieldComponent, GlzInputDirective],
  templateUrl: './another-service.component.html',
  styleUrl: './another-service.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AnotherServiceComponent {
  protected readonly copy = {
    heading: $localize`:@@anotherService.heading:Another service`,
    lead: $localize`:@@anotherService.lead:Any git server with an address, such as GitLab or your own.`,
    address: $localize`:@@anotherService.address:Address`,
    add: $localize`:@@anotherService.add:Add`,
    token: $localize`:@@anotherService.token:Access token`,
    connect: $localize`:@@anotherService.connect:Connect`,
    checking: $localize`:@@anotherService.checking:Checking the token…`,
    cancel: $localize`:@@anotherService.cancel:Cancel`,
    missingAddress: $localize`:@@anotherService.missingAddress:Give the server’s address first.`,
    missingToken: $localize`:@@anotherService.missingToken:Paste the token first.`,
  } as const;
  protected readonly placeholder = 'https://gitlab.com';
  private readonly credentials = inject(CredentialsService);
  protected readonly address = signal('');
  /** The address a token is asked for, once one was added. */
  protected readonly adding = signal<string | null>(null);
  protected readonly token = signal('');
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);
  protected readonly connectLabel = computed(() =>
    this.busy() ? this.copy.checking : this.copy.connect,
  );
  /** What the token step says of the service at the address added. */
  protected readonly asks = computed(() => {
    const host = hostOf(this.adding() ?? '');
    return $localize`:@@anotherService.asks:${host}:HOST: needs an access token`;
  });
  protected readonly how = computed(() => {
    const host = hostOf(this.adding() ?? '');
    return $localize`:@@anotherService.how:Make one in ${host}:HOST:’s settings, then paste it here. It stays locked away, and is never shown again.`;
  });

  protected typedAddress(event: Event): void {
    this.address.set(valueOf(event));
  }

  protected typedToken(event: Event): void {
    this.token.set(valueOf(event));
  }

  /** Takes the address, then asks for its token. */
  protected add(event: Event): void {
    event.preventDefault();
    const address = this.address().trim();
    if (address === '') {
      this.problem.set(this.copy.missingAddress);
      return;
    }
    this.problem.set(null);
    this.adding.set(address);
  }

  /** Connects the service with its token: the server checks it first. */
  protected async connect(event: Event): Promise<void> {
    event.preventDefault();
    const address = this.adding();
    const token = this.token().trim();
    if (address === null || this.busy()) {
      return;
    }
    if (token === '') {
      this.problem.set(this.copy.missingToken);
      return;
    }
    this.problem.set(null);
    this.busy.set(true);
    try {
      await this.credentials.connect(address, token);
      this.cancel();
    } catch (error: unknown) {
      this.problem.set(connectProblem(error));
    } finally {
      this.busy.set(false);
    }
  }

  /** Back to the address, nothing kept, nothing said. */
  protected cancel(): void {
    this.adding.set(null);
    this.address.set('');
    this.token.set('');
    this.problem.set(null);
  }
}

/** What an input event's field holds now. */
function valueOf(event: Event): string {
  return event.target instanceof HTMLInputElement ? event.target.value : '';
}
