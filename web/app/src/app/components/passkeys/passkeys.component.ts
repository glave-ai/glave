import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { GlzButtonComponent, GlzCardComponent, GlzConfirmDialogComponent } from '@glave/glaze';

import { PasskeysService } from '../../services/passkeys.service';

const COPY = {
  heading: 'Passkeys',
  lead: 'These sign you in to glave from your devices: your face, your fingerprint or a screen lock, never a password.',
  none: 'No passkeys yet.',
  add: 'Add a passkey on this device',
  adding: 'Waiting for your device…',
  remove: 'Remove',
  removeHeading: 'Remove this passkey?',
  removeText: 'It signs you in no more, and the browsers it signed in are signed out.',
  added: 'Added',
  used: 'last used',
} as const;

/** A passkey as the card shows it: its name, and when it was added and last used. */
interface Shown {
  readonly id: string;
  readonly name: string;
  readonly detail: string;
}

/**
 * The person's passkeys, in Settings, where glave is reached from elsewhere: each one, with
 * when it was added and last used, adding one on this device, and removing one.
 */
@Component({
  selector: 'app-passkeys',
  imports: [GlzButtonComponent, GlzCardComponent, GlzConfirmDialogComponent],
  templateUrl: './passkeys.component.html',
  styleUrl: './passkeys.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class PasskeysComponent {
  private readonly keys = inject(PasskeysService);
  private readonly dates = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' });
  protected readonly copy = COPY;
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);
  protected readonly confirming = signal(false);
  private readonly removing = signal<string | null>(null);

  protected readonly shown = computed<readonly Shown[]>(() =>
    this.keys.passkeys().map((passkey) => {
      const added = `${COPY.added} ${this.dates.format(new Date(passkey.added))}`;
      const last = passkey.used ?? null;
      const used = last === null ? '' : `, ${COPY.used} ${this.dates.format(new Date(last))}`;
      return { id: passkey.id, name: passkey.name, detail: `${added}${used}` };
    }),
  );
  protected readonly addLabel = computed(() => (this.busy() ? COPY.adding : COPY.add));

  constructor() {
    void this.keys.load().then((problem) => {
      this.problem.set(problem);
    });
  }

  /** Adds a passkey on this device, saying why when it cannot. */
  protected async add(): Promise<void> {
    this.busy.set(true);
    this.problem.set(await this.keys.add());
    this.busy.set(false);
  }

  /** Asks before removing the passkey `id`. */
  protected ask(id: string): void {
    this.removing.set(id);
    this.confirming.set(true);
  }

  /** Removes the passkey asked about, when the person said yes. */
  protected async decided(yes: boolean): Promise<void> {
    const id = this.removing();
    this.removing.set(null);
    if (yes && id !== null) {
      this.problem.set(await this.keys.remove(id));
    }
  }
}
