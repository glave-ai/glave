import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  linkedSignal,
  signal,
} from '@angular/core';
import { Router } from '@angular/router';
import type { ProviderKind } from '@glave/client';
import {
  GlzButtonComponent,
  GlzFormFieldComponent,
  GlzInputDirective,
  GlzRadioComponent,
  GlzRadioGroupComponent,
  GlzTopBarComponent,
} from '@glave/glaze';

import { connectProblem, isServiceKind, needsAddress, servicesFor } from '../../lib/providers';
import { BackService } from '../../services/back.service';
import { ModelsService } from '../../services/models.service';
import { ProvidersService } from '../../services/providers.service';

/**
 * Adding a model: choose a service by its plain name, paste the key, and glave checks it
 * before keeping it. The page a first run lands on, and where settings send the person to
 * add or change an account (`?kind=` picks the service, `?url=` fills another server's
 * address); connecting the same service and address again replaces its key.
 */
@Component({
  selector: 'app-connect',
  imports: [
    GlzButtonComponent,
    GlzFormFieldComponent,
    GlzInputDirective,
    GlzRadioComponent,
    GlzRadioGroupComponent,
    GlzTopBarComponent,
  ],
  templateUrl: './connect.component.html',
  styleUrl: './connect.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ConnectComponent {
  /** The service to start on, from the link's `?kind=`. */
  readonly kind = input<string | undefined>(undefined);
  /** Another server's address to start with, from the link's `?url=`, to change its key. */
  readonly url = input<string | undefined>(undefined);

  protected readonly copy = {
    name: 'glave',
    heading: $localize`:@@connect.heading:Add a model`,
    lead: $localize`:@@connect.lead:Choose where glave reaches models, then paste your key from there. With OpenRouter, one key reaches every major model.`,
    service: $localize`:@@connect.service:Service`,
    address: $localize`:@@connect.address:Server address`,
    addressHint: $localize`:@@connect.addressHint:Its API address, such as http://localhost:11434/v1`,
    key: $localize`:@@connect.key:Key`,
    keyHint: $localize`:@@connect.keyHint:It stays on this computer, locked away. glave never shows it again.`,
    connect: $localize`:@@connect.connect:Add`,
    connecting: $localize`:@@connect.connecting:Checking the key…`,
    back: $localize`:@@connect.back:Back`,
    missing: $localize`:@@connect.missing:Paste your key first.`,
    noAddress: $localize`:@@connect.noAddress:Give the server address first.`,
  } as const;
  protected readonly picked = linkedSignal<ProviderKind>(() => {
    const kind = this.kind();
    return isServiceKind(kind) ? kind : 'openrouter';
  });
  /** The services offered, and the one a link picked to change its key. */
  protected readonly services = computed(() => {
    const kind = this.kind();
    return servicesFor(isServiceKind(kind) ? kind : 'openrouter');
  });
  protected readonly address = linkedSignal(() => this.url() ?? '');
  protected readonly key = signal('');
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);
  protected readonly asksAddress = computed(() => needsAddress(this.picked()));
  protected readonly label = computed(() =>
    this.busy() ? this.copy.connecting : this.copy.connect,
  );
  private readonly providers = inject(ProvidersService);
  private readonly models = inject(ModelsService);
  private readonly router = inject(Router);
  private readonly backs = inject(BackService);
  /** Whether a model was set up already, so the page leads back to settings. */
  protected readonly adding = computed(() => this.models.models().length > 0);

  constructor() {
    void this.models.load();
  }

  protected pick(kind: string | null): void {
    if (isServiceKind(kind)) {
      this.picked.set(kind);
      this.problem.set(null);
    }
  }

  protected typedAddress(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.address.set(event.target.value);
    }
  }

  protected typedKey(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.key.set(event.target.value);
    }
  }

  protected async connect(event?: Event): Promise<void> {
    event?.preventDefault();
    if (this.busy()) {
      return;
    }
    const key = this.key().trim();
    const address = this.address().trim();
    if (key === '') {
      this.problem.set(this.copy.missing);
      return;
    }
    if (this.asksAddress() && address === '') {
      this.problem.set(this.copy.noAddress);
      return;
    }
    const kind = this.picked();
    const adding = this.adding();
    this.busy.set(true);
    this.problem.set(null);
    try {
      await this.providers.connect(
        needsAddress(kind) ? { kind, key, url: address } : { kind, key },
      );
      this.key.set('');
      if (adding) {
        await this.back();
      } else {
        await this.router.navigateByUrl('/');
      }
    } catch (error: unknown) {
      this.problem.set(connectProblem(error));
    } finally {
      this.busy.set(false);
    }
  }

  /** Back where the person came from, as the browser's Back would: Settings, mostly. */
  protected async back(): Promise<void> {
    await this.backs.back('/settings');
  }
}
