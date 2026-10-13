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

import { SERVICES, connectProblem, needsAddress } from '../../lib/providers';
import { ModelsService } from '../../services/models.service';
import { ProvidersService } from '../../services/providers.service';

const COPY = {
  name: 'glave',
  heading: 'Add a model',
  lead: 'glave works with the AI service you already use. Choose it, then paste your key from it.',
  service: 'Service',
  address: 'Server address',
  addressHint: 'Its API address, such as http://localhost:11434/v1',
  key: 'Key',
  keyHint: 'It stays on this computer, locked away. glave never shows it again.',
  connect: 'Add',
  connecting: 'Checking the key…',
  back: 'Back',
  missing: 'Paste your key first.',
  noAddress: 'Give the server address first.',
} as const;

/** Whether `value` names a service this page offers. */
function isKind(value: unknown): value is ProviderKind {
  return SERVICES.some((service) => service.kind === value);
}

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

  protected readonly copy = COPY;
  protected readonly services = SERVICES;
  protected readonly picked = linkedSignal<ProviderKind>(() => {
    const kind = this.kind();
    return isKind(kind) ? kind : 'openai';
  });
  protected readonly address = linkedSignal(() => this.url() ?? '');
  protected readonly key = signal('');
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);
  protected readonly asksAddress = computed(() => needsAddress(this.picked()));
  protected readonly label = computed(() => (this.busy() ? COPY.connecting : COPY.connect));
  private readonly providers = inject(ProvidersService);
  private readonly models = inject(ModelsService);
  private readonly router = inject(Router);
  /** Whether a model was set up already, so the page leads back to settings. */
  protected readonly adding = computed(() => this.models.models().length > 0);

  constructor() {
    void this.models.load();
  }

  protected pick(kind: string | null): void {
    if (isKind(kind)) {
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
      this.problem.set(COPY.missing);
      return;
    }
    if (this.asksAddress() && address === '') {
      this.problem.set(COPY.noAddress);
      return;
    }
    const kind = this.picked();
    const back = this.adding() ? '/settings' : '/';
    this.busy.set(true);
    this.problem.set(null);
    try {
      await this.providers.connect(
        needsAddress(kind) ? { kind, key, url: address } : { kind, key },
      );
      this.key.set('');
      await this.router.navigateByUrl(back);
    } catch (error: unknown) {
      this.problem.set(connectProblem(error));
    } finally {
      this.busy.set(false);
    }
  }

  protected async back(): Promise<void> {
    await this.router.navigateByUrl('/settings');
  }
}
