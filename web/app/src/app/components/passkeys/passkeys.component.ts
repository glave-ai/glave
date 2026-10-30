import {
  ChangeDetectionStrategy,
  Component,
  LOCALE_ID,
  computed,
  inject,
  signal,
} from '@angular/core';
import { GlzButtonComponent, GlzCardComponent, GlzConfirmDialogComponent } from '@glave/glaze';

import { PasskeysService } from '../../services/passkeys.service';

/**
 * A passkey as the card shows it: its name, when it was added and last used, and whether it
 * signed this browser in or was made from an emailed link.
 */
interface Shown {
  readonly id: string;
  readonly name: string;
  readonly detail: string;
  /** What else is said of it: "This device", "From an emailed link". */
  readonly marks: readonly string[];
}

/**
 * The person's passkeys, in Settings, where glave is reached from elsewhere or makes them on
 * this computer: each one, with when it was added and last used, adding one on this device,
 * and removing one.
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
  private readonly dates = new Intl.DateTimeFormat(inject(LOCALE_ID), { dateStyle: 'medium' });
  protected readonly copy = {
    heading: $localize`:@@passkeys.heading:Passkeys`,
    lead: $localize`:@@passkeys.lead:These sign you in to glave from your devices: your face, your fingerprint or a screen lock, never a password.`,
    none: $localize`:@@passkeys.none:No passkeys yet.`,
    add: $localize`:@@passkeys.add:Add a passkey on this device`,
    adding: $localize`:@@passkeys.adding:Waiting for your device…`,
    remove: $localize`:@@passkeys.remove:Remove`,
    removeHeading: $localize`:@@passkeys.removeHeading:Remove this passkey?`,
    removeText: $localize`:@@passkeys.removeText:It signs you in no more, and the browsers it signed in are signed out.`,
    email: $localize`:@@passkeys.email:Email, for getting back in`,
    current: $localize`:@@passkeys.current:This device`,
    emailed: $localize`:@@passkeys.emailed:From an emailed link`,
  } as const;
  /** The address the person's glave account gets back in with, if it has one. */
  protected readonly email = this.keys.email;
  /** What the server tells of a passkey made by email lately, on another device. */
  protected readonly notice = this.keys.notice;
  protected readonly busy = signal(false);
  protected readonly problem = signal<string | null>(null);
  protected readonly confirming = signal(false);
  private readonly removing = signal<string | null>(null);

  protected readonly shown = computed<readonly Shown[]>(() =>
    this.keys.passkeys().map((passkey) => {
      const added = this.dates.format(new Date(passkey.added));
      const last = passkey.used ?? null;
      const used = last === null ? null : this.dates.format(new Date(last));
      const detail =
        used === null
          ? $localize`:@@passkeys.added:Added ${added}:ADDED:`
          : $localize`:@@passkeys.addedUsed:Added ${added}:ADDED:, last used ${used}:USED:`;
      return {
        id: passkey.id,
        name: passkey.name,
        detail,
        marks: [
          ...(passkey.current === true ? [this.copy.current] : []),
          ...(passkey.emailed === true ? [this.copy.emailed] : []),
        ],
      };
    }),
  );
  protected readonly addLabel = computed(() => (this.busy() ? this.copy.adding : this.copy.add));

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
